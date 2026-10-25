//! A binary-table column decoded to typed values.

use std::ops::Range;
use std::result;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use num_complex::Complex;

use crate::bintable::column_element::ColumnElement;
use crate::bintable::column_view::ColumnView;
use crate::bintable::element_kind::ElementKind;
use crate::fits_error::{FitsError, Result};
use crate::ragged::{Ragged, RowValues};

/// A decoded column, flattened across all rows in row order: each row contributes
/// its `repeat` elements. Values are raw (big-endian decoded but not
/// `TSCALn`/`TZEROn`-scaled). A `P`/`Q` column's rows vary in length, so its values
/// come as a [`Ragged`] of one `ColumnData`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `L` — `Some(true)`/`Some(false)`, or `None` for the `0x00` null value (§7.3.3).
    Logical(Vec<Option<bool>>),
    /// `X` — bits, MSB-first (§7.3.2): a fixed column's row is its `repeat` bits, and
    /// a `PX`/`QX` row its descriptor's count, with no padding between rows.
    Bit(BitVec<u8, Msb0>),
    /// `B`.
    U8(Vec<u8>),
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

/// `ColumnData` from a typed vector, and back: the reverse gives the data back when it
/// holds another type. An `A` column's bytes are [`ColumnData::Character`], which neither
/// direction reaches, as `Vec<u8>` is a `B` column's.
impl<T: ColumnElement> From<Vec<T>> for ColumnData {
    fn from(values: Vec<T>) -> Self {
        T::into_data(values)
    }
}

impl<T: ColumnElement> TryFrom<ColumnData> for Vec<T> {
    type Error = ColumnData;

    fn try_from(data: ColumnData) -> result::Result<Self, ColumnData> {
        T::from_data(data)
    }
}

impl From<BitVec<u8, Msb0>> for ColumnData {
    fn from(values: BitVec<u8, Msb0>) -> Self {
        Self::Bit(values)
    }
}

impl TryFrom<ColumnData> for BitVec<u8, Msb0> {
    type Error = ColumnData;

    fn try_from(data: ColumnData) -> result::Result<Self, ColumnData> {
        match data {
            ColumnData::Bit(values) => Ok(values),
            other => Err(other),
        }
    }
}

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub fn len(&self) -> usize {
        self.view().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The `TFORMn` element kind the values are stored as.
    pub(crate) const fn element_kind(&self) -> ElementKind {
        match self {
            Self::Logical(_) => ElementKind::Logical,
            Self::Bit(_) => ElementKind::Bit,
            Self::U8(_) => ElementKind::U8,
            Self::I16(_) => ElementKind::I16,
            Self::I32(_) => ElementKind::I32,
            Self::I64(_) => ElementKind::I64,
            Self::F32(_) => ElementKind::F32,
            Self::F64(_) => ElementKind::F64,
            Self::ComplexF32(_) => ElementKind::ComplexF32,
            Self::ComplexF64(_) => ElementKind::ComplexF64,
            Self::Character(_) => ElementKind::Character,
        }
    }

    /// Every value, borrowed.
    pub fn view(&self) -> ColumnView<'_> {
        match self {
            Self::Logical(values) => ColumnView::Logical(values.as_slice()),
            Self::Bit(values) => ColumnView::Bit(values.as_bitslice()),
            Self::U8(values) => ColumnView::U8(values.as_slice()),
            Self::I16(values) => ColumnView::I16(values.as_slice()),
            Self::I32(values) => ColumnView::I32(values.as_slice()),
            Self::I64(values) => ColumnView::I64(values.as_slice()),
            Self::F32(values) => ColumnView::F32(values.as_slice()),
            Self::F64(values) => ColumnView::F64(values.as_slice()),
            Self::ComplexF32(values) => ColumnView::ComplexF32(values.as_slice()),
            Self::ComplexF64(values) => ColumnView::ComplexF64(values.as_slice()),
            Self::Character(values) => ColumnView::Character(values.as_slice()),
        }
    }

    pub const fn as_logical(&self) -> Option<&[Option<bool>]> {
        match self {
            Self::Logical(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub fn as_bit(&self) -> Option<&BitSlice<u8, Msb0>> {
        self.view().as_bit()
    }

    pub const fn as_u8(&self) -> Option<&[u8]> {
        match self {
            Self::U8(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_i16(&self) -> Option<&[i16]> {
        match self {
            Self::I16(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_i32(&self) -> Option<&[i32]> {
        match self {
            Self::I32(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_i64(&self) -> Option<&[i64]> {
        match self {
            Self::I64(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_f32(&self) -> Option<&[f32]> {
        match self {
            Self::F32(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_f64(&self) -> Option<&[f64]> {
        match self {
            Self::F64(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_complex_f32(&self) -> Option<&[Complex<f32>]> {
        match self {
            Self::ComplexF32(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_complex_f64(&self) -> Option<&[Complex<f64>]> {
        match self {
            Self::ComplexF64(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub const fn as_character(&self) -> Option<&[u8]> {
        match self {
            Self::Character(values) => Some(values.as_slice()),
            _ => None,
        }
    }

    pub(crate) fn extend_from(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Logical(a), Self::Logical(b)) => a.extend_from_slice(b),
            (Self::Bit(a), Self::Bit(b)) => a.extend_from_bitslice(b),
            (Self::U8(a), Self::U8(b)) | (Self::Character(a), Self::Character(b)) => {
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
}

impl RowValues for ColumnData {
    type Row<'a> = ColumnView<'a>;

    fn len(&self) -> usize {
        self.view().len()
    }

    fn row(&self, range: Range<usize>) -> ColumnView<'_> {
        self.view().slice(range)
    }
}

impl Ragged<ColumnData> {
    /// Rows from one [`ColumnData`] per row, which must all hold one type. Errors
    /// for an empty list, which has no type, and for a row of another type.
    pub fn from_rows(rows: impl IntoIterator<Item = ColumnData>) -> Result<Self> {
        let mut rows = rows.into_iter();
        let mut values = rows.next().ok_or(FitsError::EmptyVlaNeedsType)?;
        let mut ends = vec![values.len()];
        for (row, data) in rows.enumerate() {
            if !values.extend_from(&data) {
                return Err(FitsError::MixedArrayRows {
                    row: row + 1,
                    expected: values.element_kind(),
                    got: data.element_kind(),
                });
            }
            ends.push(values.len());
        }
        Ok(Self::new(values, ends))
    }
}

#[cfg(test)]
mod tests {
    use crate::bintable::column_data::ColumnData;
    use crate::bintable::element_kind::ElementKind;
    use crate::fits_error::FitsError;
    use crate::ragged::Ragged;
    use std::assert_matches;

    /// Rows of 1 and 2 values end at 1 and 3. Rows of two types, or no row, which fixes
    /// no type, are refused.
    #[test]
    fn rows_of_one_type_join_and_others_are_refused() {
        let ragged =
            Ragged::from_rows([ColumnData::I16(vec![1]), ColumnData::I16(vec![2, 3])]).unwrap();
        assert_eq!(ragged.values(), &ColumnData::I16(vec![1, 2, 3]));
        assert_eq!(ragged.ends(), [1, 3]);
        assert_matches!(
            Ragged::from_rows([ColumnData::I16(vec![1]), ColumnData::I32(vec![2])]),
            Err(FitsError::MixedArrayRows {
                row: 1,
                expected: ElementKind::I16,
                got: ElementKind::I32
            })
        );
        assert_matches!(
            Ragged::from_rows(Vec::new()),
            Err(FitsError::EmptyVlaNeedsType)
        );
    }
}
