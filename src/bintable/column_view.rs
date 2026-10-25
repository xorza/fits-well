//! [`ColumnView`]: borrowed column values, the twin of [`ColumnData`].

use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use num_complex::Complex;

use crate::bintable::column_data::ColumnData;
use crate::bintable::column_element::ColumnElement;

/// Borrowed column values, the twin of [`ColumnData`] as [`View`](crate::image::View) is
/// of [`Data`](crate::image::Data): what [`ColumnData::view`] lends, and one row of a
/// variable-length column's [`Ragged`](crate::table::Ragged).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ColumnView<'a> {
    Logical(&'a [Option<bool>]),
    Bit(&'a BitSlice<u8, Msb0>),
    U8(&'a [u8]),
    I16(&'a [i16]),
    I32(&'a [i32]),
    I64(&'a [i64]),
    F32(&'a [f32]),
    F64(&'a [f64]),
    ComplexF32(&'a [Complex<f32>]),
    ComplexF64(&'a [Complex<f64>]),
    Character(&'a [u8]),
}

impl<'a> ColumnView<'a> {
    /// Element count.
    pub fn len(&self) -> usize {
        match self {
            Self::Logical(values) => values.len(),
            Self::Bit(values) => values.len(),
            Self::U8(values) | Self::Character(values) => values.len(),
            Self::I16(values) => values.len(),
            Self::I32(values) => values.len(),
            Self::I64(values) => values.len(),
            Self::F32(values) => values.len(),
            Self::F64(values) => values.len(),
            Self::ComplexF32(values) => values.len(),
            Self::ComplexF64(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn as_logical(&self) -> Option<&'a [Option<bool>]> {
        if let Self::Logical(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_bit(&self) -> Option<&'a BitSlice<u8, Msb0>> {
        if let Self::Bit(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_u8(&self) -> Option<&'a [u8]> {
        if let Self::U8(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_i16(&self) -> Option<&'a [i16]> {
        if let Self::I16(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_i32(&self) -> Option<&'a [i32]> {
        if let Self::I32(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_i64(&self) -> Option<&'a [i64]> {
        if let Self::I64(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_f32(&self) -> Option<&'a [f32]> {
        if let Self::F32(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_f64(&self) -> Option<&'a [f64]> {
        if let Self::F64(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_complex_f32(&self) -> Option<&'a [Complex<f32>]> {
        if let Self::ComplexF32(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_complex_f64(&self) -> Option<&'a [Complex<f64>]> {
        if let Self::ComplexF64(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_character(&self) -> Option<&'a [u8]> {
        if let Self::Character(values) = self {
            Some(values)
        } else {
            None
        }
    }

    /// The values in `range`.
    #[must_use]
    pub fn slice(self, range: Range<usize>) -> Self {
        match self {
            Self::Logical(values) => Self::Logical(&values[range]),
            Self::Bit(values) => Self::Bit(&values[range]),
            Self::U8(values) => Self::U8(&values[range]),
            Self::I16(values) => Self::I16(&values[range]),
            Self::I32(values) => Self::I32(&values[range]),
            Self::I64(values) => Self::I64(&values[range]),
            Self::F32(values) => Self::F32(&values[range]),
            Self::F64(values) => Self::F64(&values[range]),
            Self::ComplexF32(values) => Self::ComplexF32(&values[range]),
            Self::ComplexF64(values) => Self::ComplexF64(&values[range]),
            Self::Character(values) => Self::Character(&values[range]),
        }
    }

    /// The values, owned.
    pub fn to_data(&self) -> ColumnData {
        match self {
            Self::Logical(values) => ColumnData::Logical(values.to_vec()),
            Self::Bit(values) => ColumnData::Bit(values.to_bitvec()),
            Self::U8(values) => ColumnData::U8(values.to_vec()),
            Self::I16(values) => ColumnData::I16(values.to_vec()),
            Self::I32(values) => ColumnData::I32(values.to_vec()),
            Self::I64(values) => ColumnData::I64(values.to_vec()),
            Self::F32(values) => ColumnData::F32(values.to_vec()),
            Self::F64(values) => ColumnData::F64(values.to_vec()),
            Self::ComplexF32(values) => ColumnData::ComplexF32(values.to_vec()),
            Self::ComplexF64(values) => ColumnData::ComplexF64(values.to_vec()),
            Self::Character(values) => ColumnData::Character(values.to_vec()),
        }
    }
}

/// The values of a view of `T`; the view back when it holds another type.
impl<'a, T: ColumnElement> TryFrom<ColumnView<'a>> for &'a [T] {
    type Error = ColumnView<'a>;

    fn try_from(view: ColumnView<'a>) -> Result<Self, ColumnView<'a>> {
        T::from_view(view).ok_or(view)
    }
}
