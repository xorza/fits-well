//! A binary-table column decoded to typed values.

use num_complex::Complex;

use crate::bintable::column_view::ColumnView;

/// A decoded column, flattened across all rows in row order: each row contributes
/// its `repeat` elements. Values are raw (big-endian decoded but not
/// `TSCALn`/`TZEROn`-scaled). A `P`/`Q` column's rows vary in length, so its values
/// come as a [`Ragged`](crate::table::Ragged) of one `ColumnData`.
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `L` — `Some(true)`/`Some(false)`, or `None` for the `0x00` null value (§7.3.3).
    Logical(Vec<Option<bool>>),
    /// `B` (bytes) and `X` (packed bits).
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
macro_rules! impl_column_data_vec_conversions {
    ($variant:ident, $type:ty) => {
        impl From<Vec<$type>> for ColumnData {
            fn from(values: Vec<$type>) -> ColumnData {
                ColumnData::$variant(values)
            }
        }

        impl TryFrom<ColumnData> for Vec<$type> {
            type Error = ColumnData;

            fn try_from(data: ColumnData) -> core::result::Result<Self, ColumnData> {
                match data {
                    ColumnData::$variant(values) => Ok(values),
                    other => Err(other),
                }
            }
        }
    };
}

impl_column_data_vec_conversions!(Logical, Option<bool>);
impl_column_data_vec_conversions!(U8, u8);
impl_column_data_vec_conversions!(I16, i16);
impl_column_data_vec_conversions!(I32, i32);
impl_column_data_vec_conversions!(I64, i64);
impl_column_data_vec_conversions!(F32, f32);
impl_column_data_vec_conversions!(F64, f64);
impl_column_data_vec_conversions!(ComplexF32, Complex<f32>);
impl_column_data_vec_conversions!(ComplexF64, Complex<f64>);

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub const fn len(&self) -> usize {
        self.view().len()
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every value, borrowed.
    pub const fn view(&self) -> ColumnView<'_> {
        match self {
            Self::Logical(values) => ColumnView::Logical(values.as_slice()),
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
        self.view().as_logical()
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

    pub const fn as_complex_f32(&self) -> Option<&[Complex<f32>]> {
        self.view().as_complex_f32()
    }

    pub const fn as_complex_f64(&self) -> Option<&[Complex<f64>]> {
        self.view().as_complex_f64()
    }

    pub const fn as_character(&self) -> Option<&[u8]> {
        self.view().as_character()
    }

    pub(crate) fn extend_from(&mut self, other: &Self) -> bool {
        match (self, other) {
            (Self::Logical(a), Self::Logical(b)) => a.extend_from_slice(b),
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

    /// What a value of this type is called in a type-mismatch error.
    pub(crate) const fn type_name(&self) -> &'static str {
        match self {
            Self::Logical(_) => "logical column data",
            Self::U8(_) => "byte column data",
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
