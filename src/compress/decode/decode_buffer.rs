//! The caller's output plane, which selects the stored sample type tiles narrow into.

use crate::bitpix::Bitpix;
use crate::compress::decode::image_decode_plan::ImageDecodePlan;
use crate::compress::decode::tiled_image::TileSection;
use crate::compress::decode::tiled_image::TiledImage;
use crate::data::image_data::ImageData;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::table_view::TableView;
use crate::words;

/// A typed mutable view of the samples a decode writes into — owned [`ImageData`] or
/// a caller's word-aligned scratch. Its variant, not the plan, picks the narrowing
/// each tile's values go through on their way to the plane.
#[derive(Debug)]
pub(super) enum DecodeBuffer<'a> {
    U8(&'a mut [u8]),
    I16(&'a mut [i16]),
    I32(&'a mut [i32]),
    I64(&'a mut [i64]),
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
}

impl<'a> DecodeBuffer<'a> {
    pub(super) fn from_samples(samples: &'a mut ImageData) -> DecodeBuffer<'a> {
        match samples {
            ImageData::U8(values) => DecodeBuffer::U8(values),
            ImageData::I16(values) => DecodeBuffer::I16(values),
            ImageData::I32(values) => DecodeBuffer::I32(values),
            ImageData::I64(values) => DecodeBuffer::I64(values),
            ImageData::F32(values) => DecodeBuffer::F32(values),
            ImageData::F64(values) => DecodeBuffer::F64(values),
        }
    }

    pub(super) fn from_words(
        words: &'a mut [u64],
        bitpix: Bitpix,
        count: usize,
    ) -> DecodeBuffer<'a> {
        // SAFETY: the callers resize `words` to `count` zeroed samples before this, and
        // zero is a valid value for every sample type, so the view covers initialized
        // storage. Alignment and bit-pattern validity are `words::samples_mut`'s to
        // uphold, not this caller's.
        unsafe {
            match bitpix {
                Bitpix::U8 => DecodeBuffer::U8(words::samples_mut(words, count)),
                Bitpix::I16 => DecodeBuffer::I16(words::samples_mut(words, count)),
                Bitpix::I32 => DecodeBuffer::I32(words::samples_mut(words, count)),
                Bitpix::I64 => DecodeBuffer::I64(words::samples_mut(words, count)),
                Bitpix::F32 => DecodeBuffer::F32(words::samples_mut(words, count)),
                Bitpix::F64 => DecodeBuffer::F64(words::samples_mut(words, count)),
            }
        }
    }

    /// Decode every tile of the image into this buffer.
    pub(super) fn decode_image(
        self,
        header: &Header,
        table: TableView<'_>,
        tiled: &TiledImage<'_>,
    ) -> Result<()> {
        debug_assert_eq!(self.bitpix(), tiled.image.bitpix);
        let plan = ImageDecodePlan::new(header, table, tiled)?;
        match self {
            DecodeBuffer::U8(out) => plan.decode_all_into(out),
            DecodeBuffer::I16(out) => plan.decode_all_into(out),
            DecodeBuffer::I32(out) => plan.decode_all_into(out),
            DecodeBuffer::I64(out) => plan.decode_all_into(out),
            DecodeBuffer::F32(out) => plan.decode_all_into(out),
            DecodeBuffer::F64(out) => plan.decode_all_into(out),
        }
    }

    /// Decode the tiles `section` intersects into this section-sized buffer.
    pub(super) fn decode_section(
        self,
        header: &Header,
        table: TableView<'_>,
        tiled: &TiledImage<'_>,
        section: TileSection<'_>,
    ) -> Result<()> {
        debug_assert_eq!(self.bitpix(), tiled.image.bitpix);
        debug_assert_eq!(table.nrows, section.rows.len());
        let plan = ImageDecodePlan::new(header, table, tiled)?;
        match self {
            DecodeBuffer::U8(out) => plan.decode_region_into(section, out),
            DecodeBuffer::I16(out) => plan.decode_region_into(section, out),
            DecodeBuffer::I32(out) => plan.decode_region_into(section, out),
            DecodeBuffer::I64(out) => plan.decode_region_into(section, out),
            DecodeBuffer::F32(out) => plan.decode_region_into(section, out),
            DecodeBuffer::F64(out) => plan.decode_region_into(section, out),
        }
    }

    /// The sample type this buffer holds. Every constructor sizes it from `ZBITPIX`,
    /// and the tile decoder picks its plane from the same value.
    const fn bitpix(&self) -> Bitpix {
        match self {
            DecodeBuffer::U8(_) => Bitpix::U8,
            DecodeBuffer::I16(_) => Bitpix::I16,
            DecodeBuffer::I32(_) => Bitpix::I32,
            DecodeBuffer::I64(_) => Bitpix::I64,
            DecodeBuffer::F32(_) => Bitpix::F32,
            DecodeBuffer::F64(_) => Bitpix::F64,
        }
    }
}
