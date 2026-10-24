//! [`Data`]: an owned image array in one of the six FITS sample types.

use crate::bitpix::Bitpix;
use crate::image_model::view::View;

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

    pub fn as_u8(&self) -> Option<&[u8]> {
        self.view().as_u8()
    }

    pub fn as_i16(&self) -> Option<&[i16]> {
        self.view().as_i16()
    }

    pub fn as_i32(&self) -> Option<&[i32]> {
        self.view().as_i32()
    }

    pub fn as_i64(&self) -> Option<&[i64]> {
        self.view().as_i64()
    }

    pub fn as_f32(&self) -> Option<&[f32]> {
        self.view().as_f32()
    }

    pub fn as_f64(&self) -> Option<&[f64]> {
        self.view().as_f64()
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bitpix::Bitpix;
    use crate::image_model::data::Data;
    use crate::image_model::sample::Sample;

    impl Data {
        /// The stored big-endian `bytes`, a whole number of `bitpix` samples, decoded.
        pub(crate) fn decode(bytes: &[u8], bitpix: Bitpix) -> Self {
            fn decoded<T: Sample>(bytes: &[u8]) -> Vec<T> {
                assert_eq!(bytes.len() % size_of::<T>(), 0, "whole samples");
                let mut values = vec![T::default(); bytes.len() / size_of::<T>()];
                T::decode_be(bytes, &mut values);
                values
            }
            match bitpix {
                Bitpix::U8 => Self::U8(bytes.to_vec()),
                Bitpix::I16 => Self::I16(decoded(bytes)),
                Bitpix::I32 => Self::I32(decoded(bytes)),
                Bitpix::I64 => Self::I64(decoded(bytes)),
                Bitpix::F32 => Self::F32(decoded(bytes)),
                Bitpix::F64 => Self::F64(decoded(bytes)),
            }
        }
    }
}
