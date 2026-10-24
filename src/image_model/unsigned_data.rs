//! [`UnsignedData`]: the unsigned reading of a signed FITS array.

use std::ops::Range;

use crate::endian::decode_be_cells;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::unsigned_view::UnsignedView;
use crate::image_model::view::View;
use crate::image_model::{flip_i8, flip_u16, flip_u32, flip_u64};
use crate::ragged::RowValues;

/// A typed integer realization of the FITS unsigned (and signed-byte) storage
/// conventions — `BSCALE`/`BZERO` for images or `TSCALn`/`TZEROn` for table
/// columns, with unit scale and the matching sign-bit offset. Values are exact (no
/// `f64` rounding), recovered by flipping the stored sign bit. Returned directly
/// for images and fixed columns, and once per jagged row by
/// [`crate::table::ColumnReader::vla_unsigned`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UnsignedData {
    /// `BITPIX = 8`, `BZERO = -128`: stored `u8` → `i8`.
    I8(Vec<i8>),
    /// `BITPIX = 16`, `BZERO = 2¹⁵`: stored `i16` → `u16`.
    U16(Vec<u16>),
    /// `BITPIX = 32`, `BZERO = 2³¹`: stored `i32` → `u32`.
    U32(Vec<u32>),
    /// `BITPIX = 64`, `BZERO = 2⁶³`: stored `i64` → `u64`.
    U64(Vec<u64>),
}

impl UnsignedData {
    pub const fn len(&self) -> usize {
        self.view().len()
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Every value, borrowed.
    pub const fn view(&self) -> UnsignedView<'_> {
        match self {
            Self::I8(values) => UnsignedView::I8(values.as_slice()),
            Self::U16(values) => UnsignedView::U16(values.as_slice()),
            Self::U32(values) => UnsignedView::U32(values.as_slice()),
            Self::U64(values) => UnsignedView::U64(values.as_slice()),
        }
    }

    pub const fn as_i8(&self) -> Option<&[i8]> {
        self.view().as_i8()
    }

    pub const fn as_u16(&self) -> Option<&[u16]> {
        self.view().as_u16()
    }

    pub const fn as_u32(&self) -> Option<&[u32]> {
        self.view().as_u32()
    }

    pub const fn as_u64(&self) -> Option<&[u64]> {
        self.view().as_u64()
    }

    /// Recover exact values from the host-endian stored `view`, of the `BITPIX` that
    /// `kind` was resolved from, by flipping the stored sign bit.
    pub(crate) fn from_view(view: View<'_>, kind: UnsignedKind) -> Self {
        match (view, kind) {
            (View::U8(v), UnsignedKind::I8) => Self::I8(v.iter().map(|&x| flip_i8(x)).collect()),
            (View::I16(v), UnsignedKind::U16) => {
                Self::U16(v.iter().map(|&x| flip_u16(x)).collect())
            }
            (View::I32(v), UnsignedKind::U32) => {
                Self::U32(v.iter().map(|&x| flip_u32(x)).collect())
            }
            (View::I64(v), UnsignedKind::U64) => {
                Self::U64(v.iter().map(|&x| flip_u64(x)).collect())
            }
            (view, kind) => unreachable!("{kind:?} resolves from no {:?} view", view.bitpix()),
        }
    }

    /// Recover exact values from big-endian sign-bit-offset storage (the §5.2.5 /
    /// Table 19 convention) by flipping the stored sign bit. `cells` is one contiguous
    /// run for an image or heap array, or one strided cell per row for a table column;
    /// `capacity` is a `Vec::with_capacity` hint.
    ///
    /// This is exact across the whole 64-bit range, unlike routing the same values
    /// through the `f64` physical plane.
    pub(crate) fn from_be_cells<'a>(
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        kind: UnsignedKind,
    ) -> Self {
        match kind {
            UnsignedKind::I8 => Self::I8(decode_be_cells(cells, capacity, |[x]| flip_i8(x))),
            UnsignedKind::U16 => Self::U16(decode_be_cells(cells, capacity, |b| {
                flip_u16(i16::from_be_bytes(b))
            })),
            UnsignedKind::U32 => Self::U32(decode_be_cells(cells, capacity, |b| {
                flip_u32(i32::from_be_bytes(b))
            })),
            UnsignedKind::U64 => Self::U64(decode_be_cells(cells, capacity, |b| {
                flip_u64(i64::from_be_bytes(b))
            })),
        }
    }
}

impl RowValues for UnsignedData {
    type Row<'a> = UnsignedView<'a>;

    fn len(&self) -> usize {
        self.view().len()
    }

    fn row(&self, range: Range<usize>) -> UnsignedView<'_> {
        self.view().slice(range)
    }
}

/// `UnsignedData` from a typed vector, and back: the reverse gives the data back when
/// it holds another type.
macro_rules! impl_unsigned_data_vec_conversions {
    ($variant:ident, $type:ty) => {
        impl From<Vec<$type>> for UnsignedData {
            fn from(values: Vec<$type>) -> Self {
                Self::$variant(values)
            }
        }

        impl TryFrom<UnsignedData> for Vec<$type> {
            type Error = UnsignedData;

            fn try_from(data: UnsignedData) -> Result<Self, UnsignedData> {
                match data {
                    UnsignedData::$variant(values) => Ok(values),
                    other => Err(other),
                }
            }
        }
    };
}

impl_unsigned_data_vec_conversions!(I8, i8);
impl_unsigned_data_vec_conversions!(U16, u16);
impl_unsigned_data_vec_conversions!(U32, u32);
impl_unsigned_data_vec_conversions!(U64, u64);
