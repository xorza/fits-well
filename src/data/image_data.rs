//! [`ImageData`]: an owned image array in one of the six FITS sample types.

use crate::bitpix::Bitpix;
use crate::data::image_view::ImageView;
use crate::data::physical_out::PhysicalOut;
use crate::data::sample_type::UnsignedKind;
use crate::data::scaling::Scaling;
use crate::data::unsigned_data::UnsignedData;
use crate::data::{assert_whole_elements, physical_view};
use crate::data::{flip_i8, flip_u16, flip_u32, flip_u64};
use crate::endian::decode_be;
use crate::endian::extend_be;
use std::ops::Range;

/// An owned, host-endian sample buffer, tagged by its `BITPIX` element type.
#[derive(Debug, Clone, PartialEq)]
pub enum ImageData {
    U8(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

macro_rules! impl_image_data_from_vec {
    ($variant:ident, $type:ty) => {
        impl From<Vec<$type>> for ImageData {
            fn from(values: Vec<$type>) -> ImageData {
                ImageData::$variant(values)
            }
        }
    };
}

impl_image_data_from_vec!(U8, u8);
impl_image_data_from_vec!(I16, i16);
impl_image_data_from_vec!(I32, i32);
impl_image_data_from_vec!(I64, i64);
impl_image_data_from_vec!(F32, f32);
impl_image_data_from_vec!(F64, f64);

impl ImageData {
    /// The `BITPIX` element kind backing this buffer.
    pub fn bitpix(&self) -> Bitpix {
        match self {
            ImageData::U8(_) => Bitpix::U8,
            ImageData::I16(_) => Bitpix::I16,
            ImageData::I32(_) => Bitpix::I32,
            ImageData::I64(_) => Bitpix::I64,
            ImageData::F32(_) => Bitpix::F32,
            ImageData::F64(_) => Bitpix::F64,
        }
    }

    /// Number of samples in the buffer.
    pub fn len(&self) -> usize {
        match self {
            ImageData::U8(v) => v.len(),
            ImageData::I16(v) => v.len(),
            ImageData::I32(v) => v.len(),
            ImageData::I64(v) => v.len(),
            ImageData::F32(v) => v.len(),
            ImageData::F64(v) => v.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub(crate) fn view(&self, range: Range<usize>) -> ImageView<'_> {
        match self {
            ImageData::U8(values) => ImageView::U8(&values[range]),
            ImageData::I16(values) => ImageView::I16(&values[range]),
            ImageData::I32(values) => ImageView::I32(&values[range]),
            ImageData::I64(values) => ImageView::I64(&values[range]),
            ImageData::F32(values) => ImageView::F32(&values[range]),
            ImageData::F64(values) => ImageView::F64(&values[range]),
        }
    }

    /// Decode the raw, big-endian data unit into host-endian typed samples.
    /// `bytes` is the unpadded data (a whole number of `bitpix` elements); the
    /// fill past the data range must already be sliced off (see
    /// [`crate::io::DataUnit::data`]).
    pub(crate) fn decode(bytes: &[u8], bitpix: Bitpix) -> ImageData {
        assert_whole_elements(bytes, bitpix);
        match bitpix {
            Bitpix::U8 => ImageData::U8(bytes.to_vec()),
            Bitpix::I16 => ImageData::I16(decode_be(bytes, i16::from_be_bytes)),
            Bitpix::I32 => ImageData::I32(decode_be(bytes, i32::from_be_bytes)),
            Bitpix::I64 => ImageData::I64(decode_be(bytes, i64::from_be_bytes)),
            Bitpix::F32 => ImageData::F32(decode_be(bytes, f32::from_be_bytes)),
            Bitpix::F64 => ImageData::F64(decode_be(bytes, f64::from_be_bytes)),
        }
    }

    /// Append the samples to `out` in big-endian order — the inverse of
    /// [`ImageData::decode`]. This is the unpadded data unit; the writer pads it
    /// to the 2880-byte block grid. Appends (never clears), so a writer reusing one
    /// buffer across HDUs clears it first and pays no per-image staging allocation.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            ImageData::U8(v) => out.extend_from_slice(v),
            ImageData::I16(v) => extend_be(out, v, i16::to_be_bytes),
            ImageData::I32(v) => extend_be(out, v, i32::to_be_bytes),
            ImageData::I64(v) => extend_be(out, v, i64::to_be_bytes),
            ImageData::F32(v) => extend_be(out, v, f32::to_be_bytes),
            ImageData::F64(v) => extend_be(out, v, f64::to_be_bytes),
        }
    }

    pub(crate) fn physical_as<O: PhysicalOut>(&self, scaling: &Scaling) -> Vec<O> {
        physical_view(self.view(0..self.len()), scaling)
    }

    /// Exact typed unsigned (or signed-byte) reinterpretation when `scaling` is
    /// precisely the FITS unsigned convention (`BSCALE == 1`, no `BLANK`, and
    /// `BZERO` the matching sign-bit offset); `None` otherwise. Exact for all 64-bit
    /// values (no `f64` rounding). Shared by [`Image::unsigned`]/[`ReadImage::unsigned`].
    pub(crate) fn unsigned(&self, scaling: &Scaling) -> Option<UnsignedData> {
        let kind = scaling.unsigned_kind(self.bitpix())?;
        // The kind is derived from this buffer's own `BITPIX`, so the pairings below
        // are the only reachable ones.
        Some(match (self, kind) {
            (ImageData::U8(v), UnsignedKind::I8) => {
                UnsignedData::I8(v.iter().map(|&x| flip_i8(x)).collect())
            }
            (ImageData::I16(v), UnsignedKind::U16) => {
                UnsignedData::U16(v.iter().map(|&x| flip_u16(x)).collect())
            }
            (ImageData::I32(v), UnsignedKind::U32) => {
                UnsignedData::U32(v.iter().map(|&x| flip_u32(x)).collect())
            }
            (ImageData::I64(v), UnsignedKind::U64) => {
                UnsignedData::U64(v.iter().map(|&x| flip_u64(x)).collect())
            }
            _ => return None,
        })
    }
}
