//! [`Data`]: an owned image array in one of the six FITS sample types.

use crate::bitpix::Bitpix;
use crate::endian::decode_be;
use crate::endian::extend_be;
use crate::image_model::physical_out::PhysicalOut;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::View;
use crate::image_model::{assert_whole_elements, physical_view};
use crate::image_model::{flip_i8, flip_u16, flip_u32, flip_u64};
use std::ops::Range;

/// An owned, host-endian sample buffer, tagged by its `BITPIX` element type.
#[derive(Debug, Clone, PartialEq)]
pub enum Data {
    U8(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
}

macro_rules! impl_image_data_from_vec {
    ($variant:ident, $type:ty) => {
        impl From<Vec<$type>> for Data {
            fn from(values: Vec<$type>) -> Data {
                Data::$variant(values)
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

impl Data {
    /// The `BITPIX` element kind backing this buffer.
    pub fn bitpix(&self) -> Bitpix {
        self.as_view().bitpix()
    }

    /// Number of samples in the buffer.
    pub fn len(&self) -> usize {
        self.as_view().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every sample, borrowed.
    pub fn as_view(&self) -> View<'_> {
        match self {
            Self::U8(values) => View::U8(values),
            Self::I16(values) => View::I16(values),
            Self::I32(values) => View::I32(values),
            Self::I64(values) => View::I64(values),
            Self::F32(values) => View::F32(values),
            Self::F64(values) => View::F64(values),
        }
    }

    pub(crate) fn view(&self, range: Range<usize>) -> View<'_> {
        self.as_view().slice(range)
    }

    /// Decode the raw, big-endian data unit into host-endian typed samples.
    /// `bytes` is the unpadded data (a whole number of `bitpix` elements); the
    /// fill past the data range must already be sliced off (see
    /// [`crate::io::DataUnit::data`]).
    pub(crate) fn decode(bytes: &[u8], bitpix: Bitpix) -> Self {
        assert_whole_elements(bytes, bitpix);
        match bitpix {
            Bitpix::U8 => Self::U8(bytes.to_vec()),
            Bitpix::I16 => Self::I16(decode_be(bytes, i16::from_be_bytes)),
            Bitpix::I32 => Self::I32(decode_be(bytes, i32::from_be_bytes)),
            Bitpix::I64 => Self::I64(decode_be(bytes, i64::from_be_bytes)),
            Bitpix::F32 => Self::F32(decode_be(bytes, f32::from_be_bytes)),
            Bitpix::F64 => Self::F64(decode_be(bytes, f64::from_be_bytes)),
        }
    }

    /// Append the samples to `out` in big-endian order — the inverse of
    /// [`Data::decode`]. This is the unpadded data unit; the writer pads it
    /// to the 2880-byte block grid. Appends (never clears), so a writer reusing one
    /// buffer across HDUs clears it first and pays no per-image staging allocation.
    pub(crate) fn encode_into(&self, out: &mut Vec<u8>) {
        match self {
            Self::U8(v) => out.extend_from_slice(v),
            Self::I16(v) => extend_be(out, v, i16::to_be_bytes),
            Self::I32(v) => extend_be(out, v, i32::to_be_bytes),
            Self::I64(v) => extend_be(out, v, i64::to_be_bytes),
            Self::F32(v) => extend_be(out, v, f32::to_be_bytes),
            Self::F64(v) => extend_be(out, v, f64::to_be_bytes),
        }
    }

    pub(crate) fn physical_as<O: PhysicalOut>(&self, scaling: &Scaling) -> Vec<O> {
        physical_view(self.as_view(), scaling)
    }

    /// Exact typed unsigned (or signed-byte) reinterpretation when `scaling` is
    /// precisely the FITS unsigned convention (`BSCALE == 1`, no `BLANK`, and
    /// `BZERO` the matching sign-bit offset); `None` otherwise. Exact for all 64-bit
    /// values (no `f64` rounding). Shared by [`Image::unsigned`](crate::image_model::Image::unsigned)/
    /// [`Lazy::unsigned`](crate::image_model::lazy::Lazy::unsigned).
    pub(crate) fn unsigned(&self, scaling: &Scaling) -> Option<UnsignedData> {
        let kind = scaling.unsigned_kind(self.bitpix())?;
        // The kind is derived from this buffer's own `BITPIX`, so the pairings below
        // are the only reachable ones.
        Some(match (self, kind) {
            (Self::U8(v), UnsignedKind::I8) => {
                UnsignedData::I8(v.iter().map(|&x| flip_i8(x)).collect())
            }
            (Self::I16(v), UnsignedKind::U16) => {
                UnsignedData::U16(v.iter().map(|&x| flip_u16(x)).collect())
            }
            (Self::I32(v), UnsignedKind::U32) => {
                UnsignedData::U32(v.iter().map(|&x| flip_u32(x)).collect())
            }
            (Self::I64(v), UnsignedKind::U64) => {
                UnsignedData::U64(v.iter().map(|&x| flip_u64(x)).collect())
            }
            _ => return None,
        })
    }
}
