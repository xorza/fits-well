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
            ColumnData::Logical(v) => v.len(),
            ColumnData::Bytes(v) | ColumnData::Character(v) => v.len(),
            ColumnData::I16(v) => v.len(),
            ColumnData::I32(v) => v.len(),
            ColumnData::I64(v) => v.len(),
            ColumnData::F32(v) => v.len(),
            ColumnData::F64(v) => v.len(),
            ColumnData::ComplexF32(v) => v.len(),
            ColumnData::ComplexF64(v) => v.len(),
        }
    }

    /// Append `other`'s values when it holds the same type, returning whether it
    /// did.
    pub(crate) fn extend_from(&mut self, other: &ColumnData) -> bool {
        match (self, other) {
            (ColumnData::Logical(a), ColumnData::Logical(b)) => a.extend_from_slice(b),
            (ColumnData::Bytes(a), ColumnData::Bytes(b))
            | (ColumnData::Character(a), ColumnData::Character(b)) => a.extend_from_slice(b),
            (ColumnData::I16(a), ColumnData::I16(b)) => a.extend_from_slice(b),
            (ColumnData::I32(a), ColumnData::I32(b)) => a.extend_from_slice(b),
            (ColumnData::I64(a), ColumnData::I64(b)) => a.extend_from_slice(b),
            (ColumnData::F32(a), ColumnData::F32(b)) => a.extend_from_slice(b),
            (ColumnData::F64(a), ColumnData::F64(b)) => a.extend_from_slice(b),
            (ColumnData::ComplexF32(a), ColumnData::ComplexF32(b)) => a.extend_from_slice(b),
            (ColumnData::ComplexF64(a), ColumnData::ComplexF64(b)) => a.extend_from_slice(b),
            _ => return false,
        }
        true
    }

    /// What a value of this type is called in a type-mismatch error.
    pub(crate) const fn type_name(&self) -> &'static str {
        match self {
            ColumnData::Logical(_) => "logical column data",
            ColumnData::Bytes(_) => "byte column data",
            ColumnData::I16(_) => "i16 column data",
            ColumnData::I32(_) => "i32 column data",
            ColumnData::I64(_) => "i64 column data",
            ColumnData::F32(_) => "f32 column data",
            ColumnData::F64(_) => "f64 column data",
            ColumnData::ComplexF32(_) => "complex-f32 column data",
            ColumnData::ComplexF64(_) => "complex-f64 column data",
            ColumnData::Character(_) => "character column data",
        }
    }
}
