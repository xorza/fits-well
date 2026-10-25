//! [`Data`]: an owned image array in one of the six FITS sample types.

#[cfg(feature = "compression")]
use crate::allocation;
#[cfg(feature = "compression")]
use crate::fits_error::Result;
use crate::image_model::bitpix::Bitpix;
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
    /// `len` zero samples of `bitpix`, the plane a compressed read decodes into.
    #[cfg(feature = "compression")]
    pub(crate) fn zeroed(bitpix: Bitpix, len: usize) -> Result<Self> {
        Ok(match bitpix {
            Bitpix::U8 => Self::U8(allocation::try_zeroed(0u8, len)?),
            Bitpix::I16 => Self::I16(allocation::try_zeroed(0i16, len)?),
            Bitpix::I32 => Self::I32(allocation::try_zeroed(0i32, len)?),
            Bitpix::I64 => Self::I64(allocation::try_zeroed(0i64, len)?),
            Bitpix::F32 => Self::F32(allocation::try_zeroed(0.0f32, len)?),
            Bitpix::F64 => Self::F64(allocation::try_zeroed(0.0f64, len)?),
        })
    }

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

    pub const fn as_u8(&self) -> Option<&[u8]> {
        self.view().as_u8()
    }

    pub const fn as_i16(&self) -> Option<&[i16]> {
        self.view().as_i16()
    }

    pub const fn as_i32(&self) -> Option<&[i32]> {
        self.view().as_i32()
    }

    pub const fn as_i64(&self) -> Option<&[i64]> {
        self.view().as_i64()
    }

    pub const fn as_f32(&self) -> Option<&[f32]> {
        self.view().as_f32()
    }

    pub const fn as_f64(&self) -> Option<&[f64]> {
        self.view().as_f64()
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::image_model::bitpix::Bitpix;
    use crate::image_model::data::Data;
    use crate::image_model::sample::Sample;

    impl Data {
        /// The big-endian bytes the samples are stored as, the inverse of [`Data::decode`].
        pub(crate) fn encode(&self) -> Vec<u8> {
            let mut out = Vec::new();
            self.view().encode_into(&mut out);
            out
        }

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
