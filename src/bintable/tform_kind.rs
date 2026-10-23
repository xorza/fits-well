//! The `TFORMn` element kind and the per-kind decode dispatch it selects.

use num_complex::Complex;
use std::mem;

use crate::bintable::column_data::ColumnData;
use crate::bitpix::Bitpix;
use crate::endian::{decode_be_cells, decode_be_cells_into};
use crate::error::FitsError;
use crate::error::Result;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::scaling::Scaling;

/// The element type of a binary-table column, from the letter of its `TFORMn`
/// code (Table 18).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TformKind {
    /// `L` — logical (one ASCII `T`/`F` byte per element).
    Logical,
    /// `X` — bit array (`repeat` bits packed into `ceil(repeat/8)` bytes).
    Bit,
    /// `B` — unsigned byte.
    U8,
    /// `I` — 16-bit integer.
    I16,
    /// `J` — 32-bit integer.
    I32,
    /// `K` — 64-bit integer.
    I64,
    /// `A` — character (a `repeat`-length string per row).
    Character,
    /// `E` — single-precision float.
    F32,
    /// `D` — double-precision float.
    F64,
    /// `C` — single-precision complex (real, imaginary).
    ComplexF32,
    /// `M` — double-precision complex.
    ComplexF64,
    /// `P` — 32-bit variable-length-array descriptor (into the heap).
    ArrayDesc32,
    /// `Q` — 64-bit variable-length-array descriptor.
    ArrayDesc64,
}

impl TformKind {
    pub(super) const fn from_code(code: u8) -> Option<Self> {
        Some(match code {
            b'L' => Self::Logical,
            b'X' => Self::Bit,
            b'B' => Self::U8,
            b'I' => Self::I16,
            b'J' => Self::I32,
            b'K' => Self::I64,
            b'A' => Self::Character,
            b'E' => Self::F32,
            b'D' => Self::F64,
            b'C' => Self::ComplexF32,
            b'M' => Self::ComplexF64,
            b'P' => Self::ArrayDesc32,
            b'Q' => Self::ArrayDesc64,
            _ => return None,
        })
    }

    /// The `TFORMn` letter for this kind.
    pub const fn code(self) -> char {
        match self {
            Self::Logical => 'L',
            Self::Bit => 'X',
            Self::U8 => 'B',
            Self::I16 => 'I',
            Self::I32 => 'J',
            Self::I64 => 'K',
            Self::Character => 'A',
            Self::F32 => 'E',
            Self::F64 => 'D',
            Self::ComplexF32 => 'C',
            Self::ComplexF64 => 'M',
            Self::ArrayDesc32 => 'P',
            Self::ArrayDesc64 => 'Q',
        }
    }

    /// Bytes per element. For `X` this is the per-*bit* size (1) — use
    /// [`Tform::byte_width`](crate::table::Tform::byte_width) for a
    /// column's true in-row width.
    pub(crate) const fn elem_size(self) -> usize {
        match self {
            Self::Logical | Self::Bit | Self::U8 | Self::Character => 1,
            Self::I16 => 2,
            Self::I32 | Self::F32 => 4,
            Self::I64 | Self::F64 | Self::ComplexF32 | Self::ArrayDesc32 => 8,
            Self::ComplexF64 | Self::ArrayDesc64 => 16,
        }
    }

    /// Whether this is a `P`/`Q` variable-length-array descriptor rather than a
    /// stored value: those cells address the heap, so every fixed-width decode
    /// rejects them.
    pub(crate) const fn is_descriptor(self) -> bool {
        matches!(self, Self::ArrayDesc32 | Self::ArrayDesc64)
    }

    /// Whether a column's `scaling` realizes a FITS unsigned (or signed-byte)
    /// convention over this element kind, and which one. This only maps the stored
    /// type onto the matching `BITPIX` and hands `scaling` to
    /// [`Scaling::unsigned_kind`], which resolves the convention for both paths.
    pub(super) fn unsigned_kind(self, scaling: &Scaling) -> Option<UnsignedKind> {
        let bitpix = match self {
            Self::U8 => Bitpix::U8,
            Self::I16 => Bitpix::I16,
            Self::I32 => Bitpix::I32,
            Self::I64 => Bitpix::I64,
            _ => return None,
        };
        scaling.unsigned_kind(bitpix)
    }

    /// Decode `cells` as a flat run of this kind's values. A heap element cannot
    /// itself be a descriptor, so the caller resolves the two descriptor kinds.
    pub(super) fn decode_cells<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
    ) -> ColumnData {
        let mut out = ColumnData::U8(Vec::new());
        self.decode_cells_into(cells, capacity, &mut out);
        out
    }

    /// [`TformKind::decode_cells`] into `out`, whose buffer it reuses when `out` already
    /// holds values of this kind.
    pub(super) fn decode_cells_into<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        out: &mut ColumnData,
    ) {
        match self {
            Self::Logical => refill(out, cells, capacity, logical_from_byte, ColumnData::Logical),
            Self::U8 | Self::Bit => refill(out, cells, capacity, |[byte]| byte, ColumnData::U8),
            Self::Character => {
                let mut values = match mem::replace(out, ColumnData::U8(Vec::new())) {
                    ColumnData::Character(values) => values,
                    _ => Vec::new(),
                };
                decode_be_cells_into(cells, capacity, |[byte]| byte, &mut values);
                *out = ColumnData::Character(values);
            }
            Self::I16 => refill(out, cells, capacity, i16::from_be_bytes, ColumnData::I16),
            Self::I32 => refill(out, cells, capacity, i32::from_be_bytes, ColumnData::I32),
            Self::I64 => refill(out, cells, capacity, i64::from_be_bytes, ColumnData::I64),
            Self::F32 => refill(out, cells, capacity, f32::from_be_bytes, ColumnData::F32),
            Self::F64 => refill(out, cells, capacity, f64::from_be_bytes, ColumnData::F64),
            Self::ComplexF32 => refill(
                out,
                cells,
                capacity,
                |bytes: [u8; 8]| Complex {
                    re: f32::from_be_bytes(bytes[..4].try_into().unwrap()),
                    im: f32::from_be_bytes(bytes[4..].try_into().unwrap()),
                },
                ColumnData::ComplexF32,
            ),
            Self::ComplexF64 => refill(
                out,
                cells,
                capacity,
                |bytes: [u8; 16]| Complex {
                    re: f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    im: f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                },
                ColumnData::ComplexF64,
            ),
            Self::ArrayDesc32 | Self::ArrayDesc64 => {
                unreachable!("descriptor cells are resolved by the caller")
            }
        }
    }

    /// The kind a `P`/`Q` column's heap values decode as: the element kind itself,
    /// except that a heap element cannot be a descriptor and keeps its raw bytes.
    pub(super) const fn heap_kind(self) -> Self {
        match self {
            Self::ArrayDesc32 | Self::ArrayDesc64 => Self::U8,
            kind => kind,
        }
    }

    /// Decode `cells` to the physical `f64` plane: `TZEROn + TSCALn × raw`, mapping
    /// integers equal to `TNULLn` to `NaN`. Errors for the non-numeric kinds.
    pub(super) fn decode_physical<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        scaling: &Scaling,
    ) -> Result<Vec<f64>> {
        Ok(match self {
            Self::U8 => decode_be_cells(cells, capacity, |[x]| scaling.scale_integer(i64::from(x))),
            Self::I16 => decode_be_cells(cells, capacity, |bytes| {
                scaling.scale_integer(i64::from(i16::from_be_bytes(bytes)))
            }),
            Self::I32 => decode_be_cells(cells, capacity, |bytes| {
                scaling.scale_integer(i64::from(i32::from_be_bytes(bytes)))
            }),
            Self::I64 => decode_be_cells(cells, capacity, |bytes| {
                scaling.scale_integer(i64::from_be_bytes(bytes))
            }),
            Self::F32 => decode_be_cells(cells, capacity, |bytes| {
                scaling.scale(f64::from(f32::from_be_bytes(bytes)))
            }),
            Self::F64 => decode_be_cells(cells, capacity, |bytes| {
                scaling.scale(f64::from_be_bytes(bytes))
            }),
            _ => return Err(FitsError::NotANumericColumn { code: self.code() }),
        })
    }

    /// Decode `cells` as `C`/`M` complex values, applying `TSCALn` to both
    /// components and `TZEROn` to the real component (§7.3.2).
    pub(super) fn decode_complex<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        tscale: f64,
        tzero: f64,
    ) -> Result<Vec<Complex<f64>>> {
        let scale = |re: f64, im: f64| Complex {
            re: tzero + tscale * re,
            im: tscale * im,
        };
        match self {
            Self::ComplexF32 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 8]| {
                scale(
                    f64::from(f32::from_be_bytes(bytes[..4].try_into().unwrap())),
                    f64::from(f32::from_be_bytes(bytes[4..].try_into().unwrap())),
                )
            })),
            Self::ComplexF64 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                scale(
                    f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                )
            })),
            _ => Err(FitsError::NotAComplexColumn { code: self.code() }),
        }
    }
}

/// An `L` value: `T` and `F`, and `None` for `0x00` or any other byte, the §7.3.3
/// undefined value.
const fn logical_from_byte([byte]: [u8; 1]) -> Option<bool> {
    match byte {
        b'T' => Some(true),
        b'F' => Some(false),
        _ => None,
    }
}

/// Decode `cells` into the vector `out` holds as `wrap`'s variant, reusing it, or into a
/// new one when `out` holds another type.
fn refill<'a, const N: usize, T>(
    out: &mut ColumnData,
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    conv: impl Fn([u8; N]) -> T,
    wrap: fn(Vec<T>) -> ColumnData,
) where
    Vec<T>: TryFrom<ColumnData>,
{
    let mut values =
        Vec::<T>::try_from(mem::replace(out, ColumnData::U8(Vec::new()))).unwrap_or_default();
    decode_be_cells_into(cells, capacity, conv, &mut values);
    *out = wrap(values);
}
