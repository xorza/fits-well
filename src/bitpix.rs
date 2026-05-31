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
    pub fn from_code(code: i64) -> Result<Self> {
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
    pub fn code(self) -> i64 {
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
    pub fn elem_size(self) -> usize {
        (self.code().unsigned_abs() / 8) as usize
    }

    pub fn is_float(self) -> bool {
        matches!(self, Bitpix::F32 | Bitpix::F64)
    }

    pub fn is_integer(self) -> bool {
        !self.is_float()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn code_round_trips_for_every_variant() {
        for bp in [
            Bitpix::U8,
            Bitpix::I16,
            Bitpix::I32,
            Bitpix::I64,
            Bitpix::F32,
            Bitpix::F64,
        ] {
            assert_eq!(Bitpix::from_code(bp.code()).unwrap(), bp);
        }
    }

    #[test]
    fn codes_and_sizes_match_the_standard() {
        // (BITPIX code, byte size, is_float)
        let cases = [
            (Bitpix::U8, 8, 1, false),
            (Bitpix::I16, 16, 2, false),
            (Bitpix::I32, 32, 4, false),
            (Bitpix::I64, 64, 8, false),
            (Bitpix::F32, -32, 4, true),
            (Bitpix::F64, -64, 8, true),
        ];
        for (bp, code, size, is_float) in cases {
            assert_eq!(bp.code(), code);
            assert_eq!(bp.elem_size(), size);
            assert_eq!(bp.is_float(), is_float);
            assert_eq!(bp.is_integer(), !is_float);
        }
    }

    #[test]
    fn rejects_codes_outside_the_allowed_set() {
        for bad in [0, 7, 1, -1, 24, 128, -16] {
            assert!(matches!(
                Bitpix::from_code(bad),
                Err(FitsError::InvalidBitpix { code }) if code == bad
            ));
        }
    }
}
