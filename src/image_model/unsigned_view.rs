//! [`UnsignedView`]: borrowed unsigned values, the twin of [`UnsignedData`].

use std::ops::Range;

use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::unsigned_element::UnsignedElement;

/// Borrowed unsigned values, the twin of [`UnsignedData`] as
/// [`ColumnView`](crate::table::ColumnView) is of
/// [`ColumnData`](crate::table::ColumnData): what [`UnsignedData::view`] lends, and one
/// row of [`ColumnReader::vla_unsigned`](crate::table::ColumnReader::vla_unsigned).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UnsignedView<'a> {
    I8(&'a [i8]),
    U16(&'a [u16]),
    U32(&'a [u32]),
    U64(&'a [u64]),
}

impl<'a> UnsignedView<'a> {
    pub const fn len(&self) -> usize {
        match self {
            Self::I8(values) => values.len(),
            Self::U16(values) => values.len(),
            Self::U32(values) => values.len(),
            Self::U64(values) => values.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn as_i8(&self) -> Option<&'a [i8]> {
        if let Self::I8(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_u16(&self) -> Option<&'a [u16]> {
        if let Self::U16(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_u32(&self) -> Option<&'a [u32]> {
        if let Self::U32(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_u64(&self) -> Option<&'a [u64]> {
        if let Self::U64(values) = self {
            Some(values)
        } else {
            None
        }
    }

    /// The values in `range`.
    #[must_use]
    pub fn slice(self, range: Range<usize>) -> Self {
        match self {
            Self::I8(values) => Self::I8(&values[range]),
            Self::U16(values) => Self::U16(&values[range]),
            Self::U32(values) => Self::U32(&values[range]),
            Self::U64(values) => Self::U64(&values[range]),
        }
    }

    /// The values, owned.
    pub fn to_data(&self) -> UnsignedData {
        match self {
            Self::I8(values) => UnsignedData::I8(values.to_vec()),
            Self::U16(values) => UnsignedData::U16(values.to_vec()),
            Self::U32(values) => UnsignedData::U32(values.to_vec()),
            Self::U64(values) => UnsignedData::U64(values.to_vec()),
        }
    }
}

/// The values of a view of `T`; the view back when it holds another type.
impl<'a, T: UnsignedElement> TryFrom<UnsignedView<'a>> for &'a [T] {
    type Error = UnsignedView<'a>;

    fn try_from(view: UnsignedView<'a>) -> Result<Self, UnsignedView<'a>> {
        T::from_view(view).ok_or(view)
    }
}
