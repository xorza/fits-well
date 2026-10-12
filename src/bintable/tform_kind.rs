//! The `TFORMn` element kind and the per-kind decode dispatch it selects.

use num_complex::Complex;

use crate::bintable::column_data::ColumnData;
use crate::bitpix::Bitpix;
use crate::endian::decode_be_cells;
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
    Byte,
    /// `I` — 16-bit integer.
    I16,
    /// `J` — 32-bit integer.
    I32,
    /// `K` — 64-bit integer.
    I64,
    /// `A` — character (a `repeat`-length string per row).
    Char,
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
            b'B' => Self::Byte,
            b'I' => Self::I16,
            b'J' => Self::I32,
            b'K' => Self::I64,
            b'A' => Self::Char,
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
            Self::Byte => 'B',
            Self::I16 => 'I',
            Self::I32 => 'J',
            Self::I64 => 'K',
            Self::Char => 'A',
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
            Self::Logical | Self::Bit | Self::Byte | Self::Char => 1,
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
            Self::Byte => Bitpix::U8,
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
        match self {
            Self::Logical => {
                ColumnData::Logical(decode_be_cells(cells, capacity, |[byte]| match byte {
                    b'T' => Some(true),
                    b'F' => Some(false),
                    // 0x00 (or any non-T/F byte) is the §7.3.3 undefined value.
                    _ => None,
                }))
            }
            Self::Byte | Self::Bit => {
                ColumnData::Bytes(decode_be_cells(cells, capacity, |[byte]| byte))
            }
            Self::Char => ColumnData::Character(decode_be_cells(cells, capacity, |[byte]| byte)),
            Self::I16 => ColumnData::I16(decode_be_cells(cells, capacity, i16::from_be_bytes)),
            Self::I32 => ColumnData::I32(decode_be_cells(cells, capacity, i32::from_be_bytes)),
            Self::I64 => ColumnData::I64(decode_be_cells(cells, capacity, i64::from_be_bytes)),
            Self::F32 => ColumnData::F32(decode_be_cells(cells, capacity, f32::from_be_bytes)),
            Self::F64 => ColumnData::F64(decode_be_cells(cells, capacity, f64::from_be_bytes)),
            Self::ComplexF32 => {
                ColumnData::ComplexF32(decode_be_cells(cells, capacity, |bytes: [u8; 8]| Complex {
                    re: f32::from_be_bytes(bytes[..4].try_into().unwrap()),
                    im: f32::from_be_bytes(bytes[4..].try_into().unwrap()),
                }))
            }
            Self::ComplexF64 => {
                ColumnData::ComplexF64(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                    Complex {
                        re: f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                        im: f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                    }
                }))
            }
            Self::ArrayDesc32 | Self::ArrayDesc64 => {
                unreachable!("descriptor cells are resolved by the caller")
            }
        }
    }

    /// The kind a `P`/`Q` column's heap values decode as: the element kind itself,
    /// except that a heap element cannot be a descriptor and keeps its raw bytes.
    pub(super) const fn heap_kind(self) -> Self {
        match self {
            Self::ArrayDesc32 | Self::ArrayDesc64 => Self::Byte,
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
            Self::Byte => {
                decode_be_cells(cells, capacity, |[x]| scaling.scale_integer(i64::from(x)))
            }
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
            _ => return Err(FitsError::NonNumericColumn { code: self.code() }),
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
