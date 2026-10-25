use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::View;
use crate::image_model::words;

/// The physical element type of an array, selected by the `BITPIX` keyword.
///
/// Note the asymmetry mandated by the standard: `BITPIX = 8` is the *only*
/// natively unsigned integer; 16/32/64-bit are two's-complement signed. Other
/// unsigned widths and signed bytes are faked via a `BZERO`/`TZERO` offset and
/// are detected at the scaling layer, not here.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Bitpix {
    /// `8` — unsigned 8-bit integer (or raw character).
    U8,
    /// `16` — signed 16-bit integer.
    I16,
    /// `32` — signed 32-bit integer.
    I32,
    /// `64` — signed 64-bit integer.
    I64,
    /// `-32` — IEEE-754 single-precision float.
    F32,
    /// `-64` — IEEE-754 double-precision float.
    F64,
}

/// The type the integer `BITPIX` keyword value `code` names, the inverse of
/// [`Bitpix::code`].
impl TryFrom<i64> for Bitpix {
    type Error = FitsError;

    fn try_from(code: i64) -> Result<Self> {
        match code {
            8 => Ok(Self::U8),
            16 => Ok(Self::I16),
            32 => Ok(Self::I32),
            64 => Ok(Self::I64),
            -32 => Ok(Self::F32),
            -64 => Ok(Self::F64),
            _ => Err(FitsError::InvalidBitpix { code }),
        }
    }
}

impl Bitpix {
    /// The integer `BITPIX` keyword value for this type.
    pub const fn code(self) -> i64 {
        match self {
            Self::U8 => 8,
            Self::I16 => 16,
            Self::I32 => 32,
            Self::I64 => 64,
            Self::F32 => -32,
            Self::F64 => -64,
        }
    }

    /// Size of a single element in bytes (`|BITPIX| / 8`).
    pub const fn elem_size(self) -> usize {
        (self.code().unsigned_abs() / 8) as usize
    }

    pub const fn is_float(self) -> bool {
        matches!(self, Self::F32 | Self::F64)
    }

    pub const fn is_integer(self) -> bool {
        !self.is_float()
    }

    /// Whether this type holds the integer `value`: a `BLANK` or a `TNULLn` must be one
    /// of its values, and a float holds none (§5.2.5, §7.3.2).
    pub(crate) fn holds_integer(self, value: i64) -> bool {
        match self {
            Self::U8 => u8::try_from(value).is_ok(),
            Self::I16 => i16::try_from(value).is_ok(),
            Self::I32 => i32::try_from(value).is_ok(),
            Self::I64 => true,
            Self::F32 | Self::F64 => false,
        }
    }

    /// Append the physical values of the big-endian samples `bytes` of this type to
    /// `out`, as [`Scaling::extend_physical_be`] gives them.
    ///
    /// # Panics
    /// When `bytes` is no whole number of samples: the reader sized it from the axes.
    pub(crate) fn extend_physical_be<O: PhysicalSample>(
        self,
        bytes: &[u8],
        scaling: &Scaling,
        out: &mut Vec<O>,
    ) {
        assert_eq!(
            bytes.len() % self.elem_size(),
            0,
            "data length must be a whole number of {self:?} elements"
        );
        match self {
            Self::U8 => scaling.extend_physical(bytes, out),
            Self::I16 => scaling.extend_physical_be::<i16, O>(bytes, out),
            Self::I32 => scaling.extend_physical_be::<i32, O>(bytes, out),
            Self::I64 => scaling.extend_physical_be::<i64, O>(bytes, out),
            Self::F32 => scaling.extend_physical_be::<f32, O>(bytes, out),
            Self::F64 => scaling.extend_physical_be::<f64, O>(bytes, out),
        }
    }

    /// The first `nbytes` of the host-endian scratch `words`, which a read or the
    /// compressed-image decoder wrote, as a view of this type.
    pub(crate) fn view_words(self, words: &[u64], nbytes: usize) -> View<'_> {
        let count = nbytes / self.elem_size();
        match self {
            Self::U8 => View::U8(words::samples(words, count)),
            Self::I16 => View::I16(words::samples(words, count)),
            Self::I32 => View::I32(words::samples(words, count)),
            Self::I64 => View::I64(words::samples(words, count)),
            Self::F32 => View::F32(words::samples(words, count)),
            Self::F64 => View::F64(words::samples(words, count)),
        }
    }
}

#[cfg(feature = "internals")]
pub(crate) mod internals {
    use crate::image_model::bitpix::Bitpix;
    use crate::image_model::sample::sealed::Raw;
    use crate::image_model::words;

    impl Bitpix {
        /// Swap the big-endian samples `src` of this type into `words`, a `u64`-backed
        /// (8-byte-aligned) reused scratch resized to fit, so [`Bitpix::view_words`] can
        /// hand back typed slices over the result. `U8` needs no swap: the reader borrows
        /// its bytes.
        pub(crate) fn swap_into_words(self, src: &[u8], words: &mut Vec<u64>) {
            let count = src.len() / self.elem_size();
            words.resize(src.len().div_ceil(8), 0);
            match self {
                Self::I16 => i16::decode_be(src, words::samples_mut(words, count)),
                Self::I32 => i32::decode_be(src, words::samples_mut(words, count)),
                Self::I64 => i64::decode_be(src, words::samples_mut(words, count)),
                Self::F32 => f32::decode_be(src, words::samples_mut(words, count)),
                Self::F64 => f64::decode_be(src, words::samples_mut(words, count)),
                Self::U8 => unreachable!("U8 is handled by the caller, never swapped"),
            }
        }
    }
}

#[cfg(test)]
mod tests;
