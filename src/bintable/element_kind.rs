//! The `TFORMn` element kind and the per-kind decode dispatch it selects.

use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use bitvec::view::BitView;
use num_complex::Complex;
use std::mem;

use crate::allocation;
use crate::bintable::column_data::ColumnData;
use crate::endian::{decode_be_cells, decode_be_cells_into};
use crate::fits_error::Result;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::scaling::Scaling;

/// The type of a binary-table column's elements, from the letter of its `TFORMn` code
/// (Table 18): a fixed-width column's cells hold them, and a `P`/`Q` column's heap arrays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ElementKind {
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
}

impl ElementKind {
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
        }
    }

    /// Bytes per element, of every kind but `X`, whose eight bits share a byte: the
    /// bytes of a run of elements are [`ElementKind::payload_len`]'s.
    pub(crate) const fn elem_size(self) -> usize {
        debug_assert!(!matches!(self, Self::Bit), "a bit is no whole byte");
        match self {
            Self::Logical | Self::Bit | Self::U8 | Self::Character => 1,
            Self::I16 => 2,
            Self::I32 | Self::F32 => 4,
            Self::I64 | Self::F64 | Self::ComplexF32 => 8,
            Self::ComplexF64 => 16,
        }
    }

    /// The bytes that `count` elements take, packed: `⌈count / 8⌉` for bits; `None`
    /// past `usize::MAX`.
    pub(crate) const fn payload_len(self, count: usize) -> Option<usize> {
        match self {
            Self::Bit => Some(count.div_ceil(8)),
            _ => count.checked_mul(self.elem_size()),
        }
    }

    /// Whether a column's `scaling` realizes a FITS unsigned (or signed-byte)
    /// convention over this element kind, and which one. This only maps the stored
    /// type onto the matching `BITPIX` and hands `scaling` to
    /// [`Scaling::unsigned_kind`], which resolves the convention for both paths.
    pub(super) fn unsigned_kind(self, scaling: &Scaling) -> Option<UnsignedKind> {
        scaling.unsigned_kind(self.bitpix()?)
    }

    /// Decode `cells`, which hold `capacity` elements in all, as a flat run of this
    /// kind's values. The run is reserved once, fallibly: heap arrays may overlap
    /// (§7.3.5), so their total is no bound the file's size sets.
    pub(super) fn decode_cells<'a>(
        self,
        cells: impl Iterator<Item = Cell<'a>>,
        capacity: usize,
    ) -> Result<ColumnData> {
        let mut out = ColumnData::U8(Vec::new());
        self.decode_cells_into(cells, capacity, &mut out)?;
        Ok(out)
    }

    /// [`ElementKind::decode_cells`] into `out`, whose buffer it reuses when `out` already
    /// holds values of this kind.
    pub(super) fn decode_cells_into<'a, I: Iterator<Item = Cell<'a>>>(
        self,
        cells: I,
        capacity: usize,
        out: &mut ColumnData,
    ) -> Result<()> {
        let bytes = |cells: I| cells.map(|cell: Cell<'a>| cell.bytes);
        match self {
            Self::Bit => {
                let mut storage = match mem::replace(out, ColumnData::U8(Vec::new())) {
                    ColumnData::Bit(bits) => bits.into_vec(),
                    _ => Vec::new(),
                };
                storage.clear();
                allocation::try_reserve_exact(&mut storage, capacity.div_ceil(8))?;
                let mut bits = BitVec::<u8, Msb0>::from_vec(storage);
                for cell in cells {
                    bits.extend_from_bitslice(&cell.bytes.view_bits::<Msb0>()[..cell.count]);
                }
                *out = ColumnData::Bit(bits);
                Ok(())
            }
            Self::Logical => refill(
                out,
                bytes(cells),
                capacity,
                logical_from_byte,
                ColumnData::Logical,
            ),
            Self::U8 => refill(out, bytes(cells), capacity, |[byte]| byte, ColumnData::U8),
            Self::Character => {
                let mut values = match mem::replace(out, ColumnData::U8(Vec::new())) {
                    ColumnData::Character(values) => values,
                    _ => Vec::new(),
                };
                decode_be_cells_into(bytes(cells), capacity, |[byte]| byte, &mut values)?;
                *out = ColumnData::Character(values);
                Ok(())
            }
            Self::I16 => refill(
                out,
                bytes(cells),
                capacity,
                i16::from_be_bytes,
                ColumnData::I16,
            ),
            Self::I32 => refill(
                out,
                bytes(cells),
                capacity,
                i32::from_be_bytes,
                ColumnData::I32,
            ),
            Self::I64 => refill(
                out,
                bytes(cells),
                capacity,
                i64::from_be_bytes,
                ColumnData::I64,
            ),
            Self::F32 => refill(
                out,
                bytes(cells),
                capacity,
                f32::from_be_bytes,
                ColumnData::F32,
            ),
            Self::F64 => refill(
                out,
                bytes(cells),
                capacity,
                f64::from_be_bytes,
                ColumnData::F64,
            ),
            Self::ComplexF32 => refill(
                out,
                bytes(cells),
                capacity,
                |bytes: [u8; 8]| Complex {
                    re: f32::from_be_bytes(bytes[..4].try_into().unwrap()),
                    im: f32::from_be_bytes(bytes[4..].try_into().unwrap()),
                },
                ColumnData::ComplexF32,
            ),
            Self::ComplexF64 => refill(
                out,
                bytes(cells),
                capacity,
                |bytes: [u8; 16]| Complex {
                    re: f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    im: f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                },
                ColumnData::ComplexF64,
            ),
        }
    }

    /// The `BITPIX` that stores the same values as this kind, for a numeric kind; `None`
    /// for `L`, `X`, `A`, `C` and `M`.
    pub(crate) const fn bitpix(self) -> Option<Bitpix> {
        match self {
            Self::U8 => Some(Bitpix::U8),
            Self::I16 => Some(Bitpix::I16),
            Self::I32 => Some(Bitpix::I32),
            Self::I64 => Some(Bitpix::I64),
            Self::F32 => Some(Bitpix::F32),
            Self::F64 => Some(Bitpix::F64),
            Self::Logical | Self::Bit | Self::Character | Self::ComplexF32 | Self::ComplexF64 => {
                None
            }
        }
    }

    /// Whether this kind is `C` or `M`.
    pub(super) const fn is_complex(self) -> bool {
        matches!(self, Self::ComplexF32 | Self::ComplexF64)
    }

    /// Decode `cells`, which hold `capacity` elements in all, to the physical `f64`
    /// plane, as an image's samples decode: `TZEROn + TSCALn × raw`, mapping integers
    /// equal to `TNULLn` to `NaN`.
    ///
    /// # Panics
    /// For a non-numeric kind, which the caller refuses first.
    pub(super) fn decode_physical_into<'a, T: PhysicalSample>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        scaling: &Scaling,
        out: &mut Vec<T>,
    ) -> Result<()> {
        let bitpix = self
            .bitpix()
            .expect("the caller refuses a non-numeric kind");
        allocation::try_reserve(out, capacity)?;
        for cell in cells {
            bitpix.extend_physical_be(cell, scaling, out);
        }
        Ok(())
    }

    /// Decode `cells`, which hold `capacity` elements in all, as `C`/`M` complex values,
    /// applying `TSCALn` to both components and `TZEROn` to the real component (§7.3.2).
    ///
    /// # Panics
    /// For a kind that is not complex, which the caller refuses first.
    pub(super) fn decode_complex<'a>(
        self,
        cells: impl Iterator<Item = &'a [u8]>,
        capacity: usize,
        tscale: f64,
        tzero: f64,
    ) -> Result<Vec<Complex<f64>>> {
        let scale = |re: f64, im: f64| Complex {
            re: tscale.mul_add(re, tzero),
            im: tscale * im,
        };
        match self {
            Self::ComplexF32 => decode_be_cells(cells, capacity, |bytes: [u8; 8]| {
                scale(
                    f64::from(f32::from_be_bytes(bytes[..4].try_into().unwrap())),
                    f64::from(f32::from_be_bytes(bytes[4..].try_into().unwrap())),
                )
            }),
            Self::ComplexF64 => decode_be_cells(cells, capacity, |bytes: [u8; 16]| {
                scale(
                    f64::from_be_bytes(bytes[..8].try_into().unwrap()),
                    f64::from_be_bytes(bytes[8..].try_into().unwrap()),
                )
            }),
            _ => unreachable!("the caller refuses a kind that is not complex"),
        }
    }
}

/// One cell's stored bytes and the elements they hold: a fixed-width cell's repeat, or
/// a heap array's descriptor count, which for `X` is no multiple of 8.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Cell<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) count: usize,
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
) -> Result<()>
where
    Vec<T>: TryFrom<ColumnData>,
{
    let mut values =
        Vec::<T>::try_from(mem::replace(out, ColumnData::U8(Vec::new()))).unwrap_or_default();
    let decoded = decode_be_cells_into(cells, capacity, conv, &mut values);
    *out = wrap(values);
    decoded
}
