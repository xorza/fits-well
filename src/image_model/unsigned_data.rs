//! [`UnsignedData`]: the unsigned reading of a signed FITS array.

use std::ops::Range;

use crate::endian::decode_be_cells_into;
use crate::fits_error::Result;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::unsigned_element::UnsignedElement;
use crate::image_model::unsigned_view::UnsignedView;
use crate::image_model::view::View;
use crate::image_model::{flip_i8, flip_u16, flip_u32, flip_u64};
use crate::ragged::RowValues;

/// A typed integer realization of the FITS unsigned (and signed-byte) storage
/// conventions — `BSCALE`/`BZERO` for images or `TSCALn`/`TZEROn` for table
/// columns, with unit scale and the matching sign-bit offset. Values are exact (no
/// `f64` rounding), recovered by flipping the stored sign bit. Filled for images and
/// fixed columns, and held as the values of the one [`Ragged`](crate::table::Ragged)
/// [`crate::table::ColumnReader::vla_unsigned`] gives.
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

    /// Recover exact values from big-endian sign-bit-offset storage (the §5.2.5 /
    /// Table 19 convention) by flipping the stored sign bit. `cells` is one contiguous
    /// run for an image or heap array, or one strided cell per row for a table column;
    /// `capacity` is the elements the cells hold in all, reserved fallibly.
    ///
    /// This is exact across the whole 64-bit range, unlike routing the same values
    /// through the `f64` physical plane.
    pub(crate) fn from_be_cells<'a>(
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        kind: UnsignedKind,
    ) -> Result<Self> {
        let mut out = Self::empty(kind);
        out.fill_from_be_cells(cells, capacity)?;
        Ok(out)
    }

    /// [`UnsignedData::from_be_cells`] into this buffer, of the kind it holds, which it
    /// refills.
    pub(crate) fn fill_from_be_cells<'a>(
        &mut self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
    ) -> Result<()> {
        match self {
            Self::I8(out) => decode_be_cells_into(cells, capacity, |[x]| flip_i8(x), out),
            Self::U16(out) => {
                decode_be_cells_into(cells, capacity, |b| flip_u16(i16::from_be_bytes(b)), out)
            }
            Self::U32(out) => {
                decode_be_cells_into(cells, capacity, |b| flip_u32(i32::from_be_bytes(b)), out)
            }
            Self::U64(out) => {
                decode_be_cells_into(cells, capacity, |b| flip_u64(i64::from_be_bytes(b)), out)
            }
        }
    }

    /// An empty buffer of `kind`.
    pub(crate) const fn empty(kind: UnsignedKind) -> Self {
        match kind {
            UnsignedKind::I8 => Self::I8(Vec::new()),
            UnsignedKind::U16 => Self::U16(Vec::new()),
            UnsignedKind::U32 => Self::U32(Vec::new()),
            UnsignedKind::U64 => Self::U64(Vec::new()),
        }
    }

    /// The kind this buffer holds.
    pub(crate) const fn kind(&self) -> UnsignedKind {
        match self {
            Self::I8(_) => UnsignedKind::I8,
            Self::U16(_) => UnsignedKind::U16,
            Self::U32(_) => UnsignedKind::U32,
            Self::U64(_) => UnsignedKind::U64,
        }
    }

    /// This buffer refilled with the exact values of `view`, of the `BITPIX` that `kind`
    /// was resolved from, by flipping the stored sign bit: its storage reused when it
    /// holds `kind`.
    pub(crate) fn fill_from_view(&mut self, view: View<'_>, kind: UnsignedKind) {
        if self.kind() != kind {
            *self = Self::empty(kind);
        }
        match (view, self) {
            (View::U8(v), Self::I8(out)) => refill(out, v.iter().map(|&x| flip_i8(x))),
            (View::I16(v), Self::U16(out)) => refill(out, v.iter().map(|&x| flip_u16(x))),
            (View::I32(v), Self::U32(out)) => refill(out, v.iter().map(|&x| flip_u32(x))),
            (View::I64(v), Self::U64(out)) => refill(out, v.iter().map(|&x| flip_u64(x))),
            (view, out) => {
                unreachable!("{:?} resolves from no {:?} view", out.kind(), view.bitpix())
            }
        }
    }
}

/// `out` cleared and filled with `values`.
fn refill<T>(out: &mut Vec<T>, values: impl ExactSizeIterator<Item = T>) {
    out.clear();
    out.extend(values);
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
impl<T: UnsignedElement> From<Vec<T>> for UnsignedData {
    fn from(values: Vec<T>) -> Self {
        T::into_data(values)
    }
}

impl<T: UnsignedElement> TryFrom<UnsignedData> for Vec<T> {
    type Error = UnsignedData;

    fn try_from(data: UnsignedData) -> std::result::Result<Self, UnsignedData> {
        T::from_data(data)
    }
}
