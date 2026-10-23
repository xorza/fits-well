//! [`Data`]: an owned image array in one of the six FITS sample types.

use crate::bitpix::Bitpix;
use crate::endian::decode_be;
use crate::image_model::assert_whole_elements;
use crate::image_model::view::View;
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

/// `Data` from a typed vector, and back: the reverse gives the data back when it holds
/// another type.
macro_rules! impl_data_vec_conversions {
    ($variant:ident, $type:ty) => {
        impl From<Vec<$type>> for Data {
            fn from(values: Vec<$type>) -> Data {
                Data::$variant(values)
            }
        }

        impl TryFrom<Data> for Vec<$type> {
            type Error = Data;

            fn try_from(data: Data) -> core::result::Result<Self, Data> {
                match data {
                    Data::$variant(values) => Ok(values),
                    other => Err(other),
                }
            }
        }
    };
}

impl_data_vec_conversions!(U8, u8);
impl_data_vec_conversions!(I16, i16);
impl_data_vec_conversions!(I32, i32);
impl_data_vec_conversions!(I64, i64);
impl_data_vec_conversions!(F32, f32);
impl_data_vec_conversions!(F64, f64);

impl Data {
    /// The `BITPIX` element kind backing this buffer.
    pub const fn bitpix(&self) -> Bitpix {
        self.view().bitpix()
    }

    /// Number of samples in the buffer.
    pub const fn len(&self) -> usize {
        self.view().len()
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every sample, borrowed.
    pub const fn view(&self) -> View<'_> {
        match self {
            Self::U8(values) => View::U8(values.as_slice()),
            Self::I16(values) => View::I16(values.as_slice()),
            Self::I32(values) => View::I32(values.as_slice()),
            Self::I64(values) => View::I64(values.as_slice()),
            Self::F32(values) => View::F32(values.as_slice()),
            Self::F64(values) => View::F64(values.as_slice()),
        }
    }

    pub(crate) fn view_range(&self, range: Range<usize>) -> View<'_> {
        self.view().slice(range)
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
}
