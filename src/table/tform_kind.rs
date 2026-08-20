//! The `TFORMn` element kind and the per-kind decode dispatch it selects.

use num_complex::Complex;

use crate::bitpix::Bitpix;
use crate::data::sample_type::UnsignedKind;
use crate::data::scaling::Scaling;
use crate::endian::decode_be_cells;
use crate::error::FitsError;
use crate::error::Result;
use crate::table::CharacterField;
use crate::table_impl::column_data::ColumnData;

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
    pub(super) fn from_code(code: u8) -> Option<TformKind> {
        Some(match code {
            b'L' => TformKind::Logical,
            b'X' => TformKind::Bit,
            b'B' => TformKind::Byte,
            b'I' => TformKind::I16,
            b'J' => TformKind::I32,
            b'K' => TformKind::I64,
            b'A' => TformKind::Char,
            b'E' => TformKind::F32,
            b'D' => TformKind::F64,
            b'C' => TformKind::ComplexF32,
            b'M' => TformKind::ComplexF64,
            b'P' => TformKind::ArrayDesc32,
            b'Q' => TformKind::ArrayDesc64,
            _ => return None,
        })
    }

    /// The `TFORMn` letter for this kind.
    pub fn code(self) -> char {
        match self {
            TformKind::Logical => 'L',
            TformKind::Bit => 'X',
            TformKind::Byte => 'B',
            TformKind::I16 => 'I',
            TformKind::I32 => 'J',
            TformKind::I64 => 'K',
            TformKind::Char => 'A',
            TformKind::F32 => 'E',
            TformKind::F64 => 'D',
            TformKind::ComplexF32 => 'C',
            TformKind::ComplexF64 => 'M',
            TformKind::ArrayDesc32 => 'P',
            TformKind::ArrayDesc64 => 'Q',
        }
    }

    /// Bytes per element. For `X` this is the per-*bit* size (1) — use
    /// [`Tform::byte_width`](crate::table::Tform::byte_width) for a
    /// column's true in-row width.
    pub(crate) fn elem_size(self) -> usize {
        match self {
            TformKind::Logical | TformKind::Bit | TformKind::Byte | TformKind::Char => 1,
            TformKind::I16 => 2,
            TformKind::I32 | TformKind::F32 => 4,
            TformKind::I64 | TformKind::F64 | TformKind::ComplexF32 | TformKind::ArrayDesc32 => 8,
            TformKind::ComplexF64 | TformKind::ArrayDesc64 => 16,
        }
    }

    /// Whether this is a `P`/`Q` variable-length-array descriptor rather than a
    /// stored value: those cells address the heap, so every fixed-width decode
    /// rejects them.
    pub(super) fn is_descriptor(self) -> bool {
        matches!(self, TformKind::ArrayDesc32 | TformKind::ArrayDesc64)
    }

    /// Whether a column's `TSCALn`/`TZEROn`/`TNULLn` realize a FITS unsigned (or
    /// signed-byte) convention over this element kind, and which one. A table
    /// column's three keywords mean exactly what an image's `BSCALE`/`BZERO`/`BLANK`
    /// do, so this only maps the stored type onto the matching `BITPIX` and hands
    /// the trio to [`Scaling::unsigned_kind`], which resolves the convention for
    /// both paths.
    pub(super) fn unsigned_kind(
        self,
        tscale: f64,
        tzero: f64,
        tnull: Option<i64>,
    ) -> Option<UnsignedKind> {
        let bitpix = match self {
            TformKind::Byte => Bitpix::U8,
            TformKind::I16 => Bitpix::I16,
            TformKind::I32 => Bitpix::I32,
            TformKind::I64 => Bitpix::I64,
            _ => return None,
        };
        Scaling {
            bscale: tscale,
            bzero: tzero,
            blank: tnull,
        }
        .unsigned_kind(bitpix)
    }

    /// Decode `cells` as a flat run of this kind's values. `Char` and the two
    /// descriptor kinds are resolved by the caller, which knows how a row's bytes
    /// group into fields.
    pub(super) fn decode_cells<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
    ) -> ColumnData {
        match self {
            TformKind::Logical => {
                ColumnData::Logical(decode_be_cells(cells, capacity, |[byte]| match byte {
                    b'T' => Some(true),
                    b'F' => Some(false),
                    // 0x00 (or any non-T/F byte) is the §7.3.3 undefined value.
                    _ => None,
                }))
            }
            TformKind::Byte | TformKind::Bit => {
                ColumnData::Bytes(decode_be_cells(cells, capacity, |[byte]| byte))
            }
            TformKind::I16 => ColumnData::I16(decode_be_cells(cells, capacity, i16::from_be_bytes)),
            TformKind::I32 => ColumnData::I32(decode_be_cells(cells, capacity, i32::from_be_bytes)),
            TformKind::I64 => ColumnData::I64(decode_be_cells(cells, capacity, i64::from_be_bytes)),
            TformKind::F32 => ColumnData::F32(decode_be_cells(cells, capacity, f32::from_be_bytes)),
            TformKind::F64 => ColumnData::F64(decode_be_cells(cells, capacity, f64::from_be_bytes)),
            TformKind::ComplexF32 => {
                ColumnData::ComplexF32(decode_be_cells(cells, capacity, |bytes: [u8; 8]| Complex {
                    re: f32::from_be_bytes(bytes[..4].try_into().unwrap()),
                    im: f32::from_be_bytes(bytes[4..].try_into().unwrap()),
                }))
            }
            TformKind::ComplexF64 => {
                ColumnData::ComplexF64(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                    Complex {
                        re: f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                        im: f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                    }
                }))
            }
            TformKind::Char | TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
                unreachable!("character and descriptor cells are resolved by the caller")
            }
        }
    }

    /// Decode `bytes` as one contiguous run of this kind's elements — a `P`/`Q`
    /// row's heap array. The scalar kinds share [`TformKind::decode_cells`] with the
    /// fixed-width read; only the two run-specific kinds are resolved here.
    pub(super) fn decode_run(self, bytes: &[u8]) -> ColumnData {
        match self {
            // The whole run is one field: an empty descriptor yields no field at all.
            TformKind::Char if bytes.is_empty() => ColumnData::Character(Vec::new()),
            TformKind::Char => ColumnData::Character(vec![CharacterField::new(bytes.to_vec())]),
            // A heap element can't itself be a descriptor; keep the raw bytes.
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => ColumnData::Bytes(bytes.to_vec()),
            kind => kind.decode_cells(std::iter::once(bytes), bytes.len() / kind.elem_size()),
        }
    }

    /// Decode `cells` to the physical `f64` plane: `TZEROn + TSCALn × raw`, mapping
    /// integers equal to `TNULLn` to `NaN`. Errors for the non-numeric kinds.
    pub(super) fn decode_physical<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        tscale: f64,
        tzero: f64,
        tnull: Option<i64>,
    ) -> Result<Vec<f64>> {
        let scale = |x: f64| tzero + tscale * x;
        let scaled_int = |xi: i64| {
            if tnull == Some(xi) {
                f64::NAN
            } else {
                scale(xi as f64)
            }
        };
        Ok(match self {
            TformKind::Byte => decode_be_cells(cells, capacity, |[x]| scaled_int(x as i64)),
            TformKind::I16 => decode_be_cells(cells, capacity, |bytes| {
                scaled_int(i16::from_be_bytes(bytes) as i64)
            }),
            TformKind::I32 => decode_be_cells(cells, capacity, |bytes| {
                scaled_int(i32::from_be_bytes(bytes) as i64)
            }),
            TformKind::I64 => decode_be_cells(cells, capacity, |bytes| {
                scaled_int(i64::from_be_bytes(bytes))
            }),
            TformKind::F32 => decode_be_cells(cells, capacity, |bytes| {
                scale(f32::from_be_bytes(bytes) as f64)
            }),
            TformKind::F64 => {
                decode_be_cells(cells, capacity, |bytes| scale(f64::from_be_bytes(bytes)))
            }
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
            TformKind::ComplexF32 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 8]| {
                scale(
                    f32::from_be_bytes(bytes[..4].try_into().unwrap()) as f64,
                    f32::from_be_bytes(bytes[4..].try_into().unwrap()) as f64,
                )
            })),
            TformKind::ComplexF64 => Ok(decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                scale(
                    f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                )
            })),
            _ => Err(FitsError::NotAComplexColumn { code: self.code() }),
        }
    }
}
