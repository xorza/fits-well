//! Image writer encoding and incremental seekable streams.

use std::io::{Seek, Write};

use crate::bitpix::Bitpix;
use crate::checksum::running::Running;
use crate::fits_error::{FitsError, Result, SizeUnit};
use crate::fits_writer::{Body, FitsWriter, StreamHeader, merge_header_template};
use crate::hdu::HduPosition;
use crate::hdu::prologue::{Opening, Prologue, Xtension};
use crate::header_model::Header;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Metadata, check_sample_count, shape_product};

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

pub(super) fn write_template<W: Write>(
    writer: &mut FitsWriter<W>,
    image: Borrowed<'_>,
    template: Option<&Header>,
) -> Result<()> {
    debug_assert!(
        check_sample_count(image.shape, image.samples.len()).is_ok(),
        "a borrowed image's shape fixes its sample count"
    );
    let header = image_header(image.metadata(), writer.next_position()?)?;
    writer.finish_hdu(&header, template, Body::Samples(image.samples))
}

pub(super) fn stream_template<'a, W: Write + Seek>(
    writer: &'a mut FitsWriter<W>,
    layout: Metadata<'_>,
    template: Option<&Header>,
) -> Result<ImageWriter<'a, W>> {
    let expected_samples = shape_product(layout.shape)?;
    // A stream writes its header up front and rewrites it at `finish`, so it
    // never reaches `finish_hdu` — it applies the template merge itself.
    let mut header = image_header(layout, writer.next_position()?)?;
    if let Some(template) = template {
        merge_header_template(&mut header, template);
    }
    let header = writer.start_stream(header)?;
    Ok(ImageWriter {
        writer,
        header,
        expected_samples,
        written_samples: 0,
        bitpix: layout.bitpix,
        checksum: Running::new(),
        finished: false,
    })
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
fn image_header(layout: Metadata<'_>, position: HduPosition) -> Result<Header> {
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
