//! Image writer encoding and incremental seekable streams.

use std::io::{Seek, Write};

use crate::checksum::running::Running;
use crate::fits_error::{FitsError, Result, SizeUnit};
use crate::fits_writer::{FitsWriter, StreamHeader, merge_header_template};
use crate::hdu::hdu_kind::HduPosition;
use crate::hdu::prologue::{Opening, Prologue, Xtension};
use crate::header_model::Header;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::view::View;
use crate::image_model::{Metadata, shape_product};

/// Incremental image-HDU write for outputs too large to stage in memory. Chunks
/// are typed host-endian samples; the stream encodes and writes each one
/// immediately, then finalizes padding and checksums.
#[derive(Debug)]
pub struct ImageWriter<'a, W: Write + Seek> {
    writer: &'a mut FitsWriter<W>,
    header: StreamHeader,
    expected_samples: usize,
    written_samples: usize,
    bitpix: Bitpix,
    checksum: Running,
    finished: bool,
}

impl<'a, W: Write + Seek> ImageWriter<'a, W> {
    /// The stream of an image of `layout`, its header written by `writer` with the
    /// non-structural cards of `template`. A stream writes its header up front and
    /// rewrites it at `finish`, so it never reaches `finish_hdu`: it merges the template
    /// itself.
    pub(super) fn new(
        writer: &'a mut FitsWriter<W>,
        layout: Metadata<'_>,
        template: Option<&Header>,
    ) -> Result<Self> {
        let expected_samples = shape_product(layout.shape)?;
        let mut header = image_header(layout, writer.next_position()?)?;
        if let Some(template) = template {
            merge_header_template(&mut header, template);
        }
        let header = writer.start_stream(header)?;
        Ok(Self {
            writer,
            header,
            expected_samples,
            written_samples: 0,
            bitpix: layout.bitpix,
            checksum: Running::new(),
            finished: false,
        })
    }
}

impl<W: Write + Seek> ImageWriter<'_, W> {
    /// Append one typed chunk. Every chunk must have the stream's `BITPIX` type.
    pub fn write_chunk(&mut self, samples: View<'_>) -> Result<()> {
        if samples.bitpix() != self.bitpix {
            return Err(FitsError::ChunkTypeMismatch {
                expected: self.bitpix,
                got: samples.bitpix(),
            });
        }
        let total = self
            .written_samples
            .checked_add(samples.len())
            .ok_or(FitsError::DataUnitOverflow)?;
        if total > self.expected_samples {
            return Err(SizeUnit::Samples.mismatch(self.expected_samples, total));
        }
        self.writer
            .emit_samples(samples, Some(&mut self.checksum))?;
        self.written_samples = total;
        Ok(())
    }

    /// Validate the final sample count, write FITS block padding, and patch checksum
    /// cards in place when enabled.
    pub fn finish(mut self) -> Result<()> {
        self.writer.ensure_usable()?;
        if self.written_samples != self.expected_samples {
            return Err(SizeUnit::Samples.mismatch(self.expected_samples, self.written_samples));
        }
        let data_bytes = self
            .expected_samples
            .checked_mul(self.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        self.writer
            .finish_stream(&self.header, data_bytes, &mut self.checksum)?;
        self.finished = true;
        Ok(())
    }
}

impl<W: Write + Seek> Drop for ImageWriter<'_, W> {
    fn drop(&mut self) {
        if !self.finished {
            self.writer.poison();
        }
    }
}

/// Image header at `position`: the primary array (§4.4.1), or an `IMAGE` extension
/// (§7.1). The two differ only in the prologue (`SIMPLE`+`EXTEND` vs
/// `XTENSION`+`PCOUNT`/`GCOUNT`); the axes and scaling keywords are identical.
pub(super) fn image_header(layout: Metadata<'_>, position: HduPosition) -> Result<Header> {
    let opening = match position {
        HduPosition::Primary => Opening::Primary,
        HduPosition::Extension => Opening::Extension(Xtension::Image),
    };
    let mut header = Prologue {
        opening,
        bitpix: layout.bitpix,
        axes: layout.shape,
        pcount: 0,
    }
    .header()?;
    layout.scaling.add_to_header(&mut header, layout.bitpix)?;
    Ok(header)
}
