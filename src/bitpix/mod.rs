use crate::error::FitsError;
use crate::error::Result;

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

impl Bitpix {
    /// Parse the integer `BITPIX` keyword value.
    pub const fn from_code(code: i64) -> Result<Self> {
        match code {
            8 => Ok(Bitpix::U8),
            16 => Ok(Bitpix::I16),
            32 => Ok(Bitpix::I32),
            64 => Ok(Bitpix::I64),
            -32 => Ok(Bitpix::F32),
            -64 => Ok(Bitpix::F64),
            _ => Err(FitsError::InvalidBitpix { code }),
        }
    }

    /// The integer `BITPIX` keyword value for this type.
    pub const fn code(self) -> i64 {
        match self {
            Bitpix::U8 => 8,
            Bitpix::I16 => 16,
            Bitpix::I32 => 32,
            Bitpix::I64 => 64,
            Bitpix::F32 => -32,
            Bitpix::F64 => -64,
        }
    }

    /// Size of a single element in bytes (`|BITPIX| / 8`).
    pub const fn elem_size(self) -> usize {
        (self.code().unsigned_abs() / 8) as usize
    }

    pub const fn is_float(self) -> bool {
        matches!(self, Bitpix::F32 | Bitpix::F64)
    }

    pub const fn is_integer(self) -> bool {
        !self.is_float()
    }
}

#[cfg(test)]
mod tests;
