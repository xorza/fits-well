//! A binary-table column decoded to typed values.

use num_complex::Complex;

/// A decoded column, flattened across all rows in row order: each row contributes
/// its `repeat` elements. Values are raw (big-endian decoded but not
/// `TSCALn`/`TZEROn`-scaled). A `P`/`Q` column's rows vary in length, so its values
/// come as a [`Ragged`](crate::table::Ragged) of one `ColumnData`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `L` — `Some(true)`/`Some(false)`, or `None` for the `0x00` null value (§7.3.3).
    Logical(Vec<Option<bool>>),
    /// `B` (bytes) and `X` (packed bits).
    Bytes(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    ComplexF32(Vec<Complex<f32>>),
    ComplexF64(Vec<Complex<f64>>),
    /// `A` — one byte per element, stored exactly: a fixed column's row is its
    /// `repeat`-byte field, padding and terminator included. Read a field through
    /// [`CharacterField`](crate::table::CharacterField).
    Character(Vec<u8>),
}

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub const fn element_count(&self) -> usize {
        match self {
            Self::Logical(v) => v.len(),
            Self::Bytes(v) | Self::Character(v) => v.len(),
            Self::I16(v) => v.len(),
            Self::I32(v) => v.len(),
            Self::I64(v) => v.len(),
            Self::F32(v) => v.len(),
            Self::F64(v) => v.len(),
            Self::ComplexF32(v) => v.len(),
            Self::ComplexF64(v) => v.len(),
        }
    }

    /// Append `other`'s values when it holds the same type, returning whether it
    /// did.
    pub(crate) fn extend_from(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Logical(a), Self::Logical(b)) => a.extend_from_slice(b),
            (Self::Bytes(a), Self::Bytes(b)) | (Self::Character(a), Self::Character(b)) => {
                a.extend_from_slice(b);
            }
            (Self::I16(a), Self::I16(b)) => a.extend_from_slice(b),
            (Self::I32(a), Self::I32(b)) => a.extend_from_slice(b),
            (Self::I64(a), Self::I64(b)) => a.extend_from_slice(b),
            (Self::F32(a), Self::F32(b)) => a.extend_from_slice(b),
            (Self::F64(a), Self::F64(b)) => a.extend_from_slice(b),
            (Self::ComplexF32(a), Self::ComplexF32(b)) => a.extend_from_slice(b),
            (Self::ComplexF64(a), Self::ComplexF64(b)) => a.extend_from_slice(b),
            _ => return false,
        }
        true
    }

    /// What a value of this type is called in a type-mismatch error.
    pub(crate) const fn type_name(&self) -> &'static str {
        match self {
            Self::Logical(_) => "logical column data",
            Self::Bytes(_) => "byte column data",
            Self::I16(_) => "i16 column data",
            Self::I32(_) => "i32 column data",
            Self::I64(_) => "i64 column data",
            Self::F32(_) => "f32 column data",
            Self::F64(_) => "f64 column data",
            Self::ComplexF32(_) => "complex-f32 column data",
            Self::ComplexF64(_) => "complex-f64 column data",
            Self::Character(_) => "character column data",
        }
    }
}
