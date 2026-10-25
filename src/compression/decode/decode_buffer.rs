//! The caller's output plane, which selects the stored sample type tiles narrow into.

use crate::compression::decode::image_decode_plan::ImageDecodePlan;
use crate::compression::decode::tile_section::TileSection;
use crate::fits_error::Result;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::words;

/// A typed mutable view of the samples a decode writes into — owned [`Data`] or
/// a caller's word-aligned scratch. Its variant, not the plan, picks the narrowing
/// each tile's values go through on their way to the plane.
#[derive(Debug)]
pub(crate) enum DecodeBuffer<'a> {
    U8(&'a mut [u8]),
    I16(&'a mut [i16]),
    I32(&'a mut [i32]),
    I64(&'a mut [i64]),
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
}

impl<'a> DecodeBuffer<'a> {
    pub(crate) fn from_samples(samples: &'a mut Data) -> Self {
        match samples {
            Data::U8(values) => DecodeBuffer::U8(values),
            Data::I16(values) => DecodeBuffer::I16(values),
            Data::I32(values) => DecodeBuffer::I32(values),
            Data::I64(values) => DecodeBuffer::I64(values),
            Data::F32(values) => DecodeBuffer::F32(values),
            Data::F64(values) => DecodeBuffer::F64(values),
        }
    }

    pub(crate) fn from_words(words: &'a mut [u64], bitpix: Bitpix, count: usize) -> Self {
        match bitpix {
            Bitpix::U8 => DecodeBuffer::U8(words::samples_mut(words, count)),
            Bitpix::I16 => DecodeBuffer::I16(words::samples_mut(words, count)),
            Bitpix::I32 => DecodeBuffer::I32(words::samples_mut(words, count)),
            Bitpix::I64 => DecodeBuffer::I64(words::samples_mut(words, count)),
            Bitpix::F32 => DecodeBuffer::F32(words::samples_mut(words, count)),
            Bitpix::F64 => DecodeBuffer::F64(words::samples_mut(words, count)),
        }
    }

    /// Decode the tiles of `section`, or every tile of `plan`'s image for `None`, into this
    /// buffer, which holds the section's samples or the image's.
    pub(super) fn decode(
        self,
        plan: &ImageDecodePlan<'_>,
        section: Option<TileSection<'_>>,
    ) -> Result<()> {
        match self {
            DecodeBuffer::U8(out) => plan.decode_into(section, out, &|value| value),
            DecodeBuffer::I16(out) => plan.decode_into(section, out, &|value| value),
            DecodeBuffer::I32(out) => plan.decode_into(section, out, &|value| value),
            DecodeBuffer::I64(out) => plan.decode_into(section, out, &|value| value),
            DecodeBuffer::F32(out) => plan.decode_into(section, out, &|value| value),
            DecodeBuffer::F64(out) => plan.decode_into(section, out, &|value| value),
        }
    }

    /// Whether this buffer holds no sample.
    pub(super) const fn is_empty(&self) -> bool {
        match self {
            DecodeBuffer::U8(out) => out.is_empty(),
            DecodeBuffer::I16(out) => out.is_empty(),
            DecodeBuffer::I32(out) => out.is_empty(),
            DecodeBuffer::I64(out) => out.is_empty(),
            DecodeBuffer::F32(out) => out.is_empty(),
            DecodeBuffer::F64(out) => out.is_empty(),
        }
    }

    /// The sample type this buffer holds. Every constructor sizes it from `ZBITPIX`,
    /// and the tile decoder picks its plane from the same value.
    pub(super) const fn bitpix(&self) -> Bitpix {
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
