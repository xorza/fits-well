//! A binary-table column decoded to typed values.

use num_complex::Complex;

use crate::table_impl::character_field::CharacterField;

/// A decoded column, flattened across all rows in row order. For array columns
/// (`repeat > 1`) each row contributes `repeat` consecutive elements; binary `A`
/// contributes one exact [`CharacterField`] per row. Values are raw (big-endian
/// decoded but not `TSCALn`/`TZEROn`-scaled).
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
    /// Binary-table `A` — one exact field per row for fixed columns; a VLA row uses
    /// zero fields for an empty descriptor or one field containing its heap bytes.
    Character(Vec<CharacterField>),
}

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub fn element_count(&self) -> usize {
        match self {
            ColumnData::Logical(v) => v.len(),
            ColumnData::Bytes(v) => v.len(),
            ColumnData::I16(v) => v.len(),
            ColumnData::I32(v) => v.len(),
            ColumnData::I64(v) => v.len(),
            ColumnData::F32(v) => v.len(),
            ColumnData::F64(v) => v.len(),
            ColumnData::ComplexF32(v) => v.len(),
            ColumnData::ComplexF64(v) => v.len(),
            ColumnData::Character(v) => v.len(),
        }
    }
}
