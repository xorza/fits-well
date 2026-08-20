//! Image writer encoding and incremental seekable streams.

use std::io::{Seek, SeekFrom, Write};

use crate::bitpix::Bitpix;
use crate::block;
use crate::block::{BLOCK_SIZE, ZERO_FILL};
use crate::checksum;
#[cfg(feature = "compression")]
use crate::compress::{Compression, CompressionOptions, encode};
use crate::data::image_data::ImageData;
use crate::data::scaling::Scaling;
use crate::data::{Image, shape_product};
use crate::error::{FitsError, Result};
use crate::header::Header;
use crate::header::value;
use crate::keyword::key;
use crate::writer::{
    FitsWriter, PLACEHOLDER_CHECKSUM, WriterState, merge_header_template,
    prepare_header_with_data_sum, render_header,
};

/// Incremental image-HDU write for outputs too large to stage in memory. Chunks
/// are typed host-endian samples; the stream encodes and writes each one
/// immediately, then finalizes padding and checksums.
#[derive(Debug)]
pub struct ImageStream<'a, W: Write + Seek> {
    writer: &'a mut FitsWriter<W>,
    header: Header,
    header_offset: u64,
    expected_samples: usize,
    written_samples: usize,
    bitpix: Bitpix,
    checksum: StreamingChecksum,
    finished: bool,
}

#[derive(Debug, Default)]
struct StreamingChecksum {
    sum: u32,
    pending: [u8; 4],
    pending_len: usize,
}

pub(super) fn write_template<W: Write>(
    writer: &mut FitsWriter<W>,
    image: &Image,
    template: Option<&Header>,
) -> Result<()> {
    writer.ensure_writable()?;
    let expected = image.validate_geometry()?;
    let encoded_len = expected
        .checked_mul(image.samples.bitpix().elem_size())
        .ok_or(FitsError::DataUnitOverflow)?;
    let header = image_header(image, writer.state == WriterState::Empty)?;
    writer.scratch.clear();
    writer.scratch.reserve_exact(encoded_len);
    image.samples.encode_into(&mut writer.scratch);
    writer.finish_hdu(header, template, ZERO_FILL, false)
}

#[cfg(feature = "compression")]
pub(super) fn write_compressed_template<W: Write>(
    writer: &mut FitsWriter<W>,
    image: &Image,
    compression: Compression,
    options: &CompressionOptions,
    template: Option<&Header>,
) -> Result<()> {
    writer.ensure_writable()?;
    let header = encode::compress_image(image, compression, options, &mut writer.scratch)?;
    writer.finish_hdu(header, template, ZERO_FILL, true)
}

pub(super) fn stream_template<'a, W: Write + Seek>(
    writer: &'a mut FitsWriter<W>,
    shape: Vec<usize>,
    bitpix: Bitpix,
    scaling: Scaling,
    template: Option<&Header>,
) -> Result<ImageStream<'a, W>> {
    writer.ensure_writable()?;
    scaling.validate(bitpix)?;
    let expected_samples = shape_product(&shape)?;
    // A stream writes its header up front and rewrites it at `finish`, so it
    // never reaches `finish_hdu` — it applies the template merge itself.
    let mut header =
        image_header_parts(&shape, bitpix, scaling, writer.state == WriterState::Empty)?;
    merge_header_template(&mut header, template);
    let header_offset = writer.sink.stream_position()?;
    let mut initial = header.clone();
    if writer.checksum {
        initial.set_internal("DATASUM", "0");
        initial.set_internal("CHECKSUM", PLACEHOLDER_CHECKSUM);
    }
    render_header(&initial, &mut writer.header_scratch)?;
    if let Err(error) = writer.sink.write_all(&writer.header_scratch) {
        writer.state = WriterState::Failed;
        return Err(FitsError::Io(error));
    }
    writer.state = WriterState::Active;
    Ok(ImageStream {
        writer,
        header,
        header_offset,
        expected_samples,
        written_samples: 0,
        bitpix,
        checksum: StreamingChecksum::default(),
        finished: false,
    })
}

impl<W: Write + Seek> ImageStream<'_, W> {
    /// Append one typed chunk. Every chunk must have the stream's `BITPIX` type.
    pub fn write_chunk(&mut self, samples: &ImageData) -> Result<()> {
        if samples.bitpix() != self.bitpix {
            return Err(FitsError::TypeMismatch {
                name: "streaming image chunk".to_string(),
                expected: bitpix_name(self.bitpix),
            });
        }
        let total = self
            .written_samples
            .checked_add(samples.len())
            .ok_or(FitsError::DataUnitOverflow)?;
        if total > self.expected_samples {
            return Err(FitsError::DataSizeMismatch {
                expected: self.expected_samples,
                got: total,
            });
        }
        self.writer.scratch.clear();
        samples.encode_into(&mut self.writer.scratch);
        if let Err(error) = self.writer.sink.write_all(&self.writer.scratch) {
            self.writer.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        self.checksum.update(&self.writer.scratch);
        self.written_samples = total;
        Ok(())
    }

    /// Validate the final sample count, write FITS block padding, and patch checksum
    /// cards in place when enabled.
    pub fn finish(mut self) -> Result<()> {
        if self.written_samples != self.expected_samples {
            return Err(FitsError::DataSizeMismatch {
                expected: self.expected_samples,
                got: self.written_samples,
            });
        }
        let data_bytes = self
            .expected_samples
            .checked_mul(self.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        // The unit is never materialized here, so this streams the same fill
        // `pad_to_block` would have written into a buffer.
        let padding = block::padding(data_bytes);
        let zeros = [0u8; BLOCK_SIZE];
        if let Err(error) = self.writer.sink.write_all(&zeros[..padding]) {
            self.writer.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        self.checksum.update(&zeros[..padding]);
        debug_assert_eq!(self.checksum.pending_len, 0);

        if self.writer.checksum {
            let end = self.writer.sink.stream_position()?;
            prepare_header_with_data_sum(
                self.header.clone(),
                self.checksum.sum,
                &mut self.writer.header_scratch,
            )?;
            self.writer.sink.seek(SeekFrom::Start(self.header_offset))?;
            if let Err(error) = self.writer.sink.write_all(&self.writer.header_scratch) {
                self.writer.state = WriterState::Failed;
                return Err(FitsError::Io(error));
            }
            self.writer.sink.seek(SeekFrom::Start(end))?;
        }
        self.finished = true;
        Ok(())
    }
}

impl<W: Write + Seek> Drop for ImageStream<'_, W> {
    fn drop(&mut self) {
        if !self.finished {
            self.writer.state = WriterState::Failed;
        }
    }
}

impl StreamingChecksum {
    fn update(&mut self, mut bytes: &[u8]) {
        if self.pending_len != 0 {
            let needed = 4 - self.pending_len;
            let copied = needed.min(bytes.len());
            self.pending[self.pending_len..self.pending_len + copied]
                .copy_from_slice(&bytes[..copied]);
            self.pending_len += copied;
            bytes = &bytes[copied..];
            if self.pending_len == 4 {
                self.sum = checksum::accumulate(&self.pending, self.sum);
                self.pending_len = 0;
            }
        }
        let aligned = bytes.len() / 4 * 4;
        if aligned != 0 {
            self.sum = checksum::accumulate(&bytes[..aligned], self.sum);
        }
        let tail = &bytes[aligned..];
        self.pending[..tail.len()].copy_from_slice(tail);
        self.pending_len = tail.len();
    }
}

fn bitpix_name(bitpix: Bitpix) -> &'static str {
    match bitpix {
        Bitpix::U8 => "u8 image data",
        Bitpix::I16 => "i16 image data",
        Bitpix::I32 => "i32 image data",
        Bitpix::I64 => "i64 image data",
        Bitpix::F32 => "f32 image data",
        Bitpix::F64 => "f64 image data",
    }
}

/// Image header: the primary array (§4.4.1) when `primary`, else an `IMAGE`
/// extension (§7.1). The two differ only in the prologue (`SIMPLE`+`EXTEND` vs
/// `XTENSION`+`PCOUNT`/`GCOUNT`); the axes and scaling keywords are identical.
fn image_header(image: &Image, primary: bool) -> Result<Header> {
    image_header_parts(&image.shape, image.samples.bitpix(), image.scaling, primary)
}

fn image_header_parts(
    shape: &[usize],
    bitpix: Bitpix,
    scaling: Scaling,
    primary: bool,
) -> Result<Header> {
    let mut header = Header::new();
    if primary {
        header
            .set_internal("SIMPLE", true)
            .comment_internal("SIMPLE", "file conforms to FITS standard");
        add_image_axes(&mut header, shape, bitpix)?;
        header
            .set_internal("EXTEND", true)
            .comment_internal("EXTEND", "extensions may follow");
    } else {
        header
            .set_internal("XTENSION", "IMAGE")
            .comment_internal("XTENSION", "image extension");
        add_image_axes(&mut header, shape, bitpix)?;
        header.set_internal("PCOUNT", 0).set_internal("GCOUNT", 1);
    }
    scaling.add_to_header(&mut header, bitpix)?;
    Ok(header)
}

/// `BITPIX`, `NAXIS`, `NAXISn` — the mandatory array-shape keywords, in order.
fn add_image_axes(header: &mut Header, shape: &[usize], bitpix: Bitpix) -> Result<()> {
    if shape.len() > 999 {
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS" });
    }
    header
        .set_internal("BITPIX", bitpix.code())
        .comment_internal("BITPIX", "number of bits per data pixel");
    header
        .set_internal("NAXIS", value::fits_i64(shape.len())?)
        .comment_internal("NAXIS", "number of data axes");
    for (i, &n) in shape.iter().enumerate() {
        header.set_internal(key!("NAXIS{}", i + 1).as_str(), value::fits_i64(n)?);
    }
    Ok(())
}
