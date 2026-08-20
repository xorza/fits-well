//! [`UnsignedData`]: the unsigned reading of a signed FITS array.

use crate::data::sample_type::UnsignedKind;
use crate::data::{flip_i8, flip_u16, flip_u32, flip_u64};
use crate::endian::decode_be_cells;

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
    ) -> UnsignedData {
        match kind {
            UnsignedKind::I8 => {
                UnsignedData::I8(decode_be_cells(cells, capacity, |[x]| flip_i8(x)))
            }
            UnsignedKind::U16 => UnsignedData::U16(decode_be_cells(cells, capacity, |b| {
                flip_u16(i16::from_be_bytes(b))
            })),
            UnsignedKind::U32 => UnsignedData::U32(decode_be_cells(cells, capacity, |b| {
                flip_u32(i32::from_be_bytes(b))
            })),
            UnsignedKind::U64 => UnsignedData::U64(decode_be_cells(cells, capacity, |b| {
                flip_u64(i64::from_be_bytes(b))
            })),
        }
    }
}
