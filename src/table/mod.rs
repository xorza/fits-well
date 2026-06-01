//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. This
//! module parses that structure into [`Column`] descriptors and decodes:
//! fixed-width fields into typed [`ColumnData`] ([`BinTable::read_column`]), the
//! `TSCALn`/`TZEROn` physical plane ([`BinTable::read_column_physical`]), and
//! `P`/`Q` variable-length arrays out of the heap ([`BinTable::read_vla_column`]).

use crate::data::U16_OFFSET;
use crate::data::U32_OFFSET;
use crate::data::U64_OFFSET;
use crate::data::UnsignedView;
use crate::endian::decode_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

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
    fn from_code(code: u8) -> Option<TformKind> {
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
    /// [`Tform::byte_width`] for a column's true in-row width.
    fn elem_size(self) -> usize {
        match self {
            TformKind::Logical | TformKind::Bit | TformKind::Byte | TformKind::Char => 1,
            TformKind::I16 => 2,
            TformKind::I32 | TformKind::F32 => 4,
            TformKind::I64 | TformKind::F64 | TformKind::ComplexF32 | TformKind::ArrayDesc32 => 8,
            TformKind::ComplexF64 | TformKind::ArrayDesc64 => 16,
        }
    }
}

/// A parsed `TFORMn` value: a repeat count, an element kind, and (for the `P`/`Q`
/// variable-length-array descriptors) the kind of the array elements in the heap.
/// The `rTa` form's trailing `(emax)` size hint is not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tform {
    pub repeat: usize,
    pub kind: TformKind,
    /// For `P`/`Q` columns, the element kind of the heap array (the `t` in
    /// `rPt(emax)`); `None` for fixed-width columns.
    pub vla_elem: Option<TformKind>,
}

impl Tform {
    /// Parse a `TFORMn` value such as `"8A"`, `"3D"`, `"1J"`, `"E"`, or `"1PE(5)"`.
    pub fn parse(value: &str) -> Result<Tform> {
        let s = value.trim();
        let invalid = || FitsError::InvalidTform {
            tform: value.to_string(),
        };
        let pos = s
            .bytes()
            .position(|b| b.is_ascii_alphabetic())
            .ok_or_else(invalid)?;
        let repeat = if pos == 0 {
            1
        } else {
            s[..pos].parse().map_err(|_| invalid())?
        };
        let kind = TformKind::from_code(s.as_bytes()[pos]).ok_or_else(invalid)?;
        // A P/Q descriptor is followed by its heap element-type letter (`rPt`).
        let vla_elem = if matches!(kind, TformKind::ArrayDesc32 | TformKind::ArrayDesc64) {
            let elem = s.as_bytes().get(pos + 1).copied().ok_or_else(invalid)?;
            // §6.3: a `P`/`Q` descriptor's repeat count is restricted to 0 or 1.
            if repeat > 1 {
                return Err(invalid());
            }
            Some(TformKind::from_code(elem).ok_or_else(invalid)?)
        } else {
            None
        };
        Ok(Tform {
            repeat,
            kind,
            vla_elem,
        })
    }

    /// The number of bytes this column occupies in every row.
    pub fn byte_width(self) -> usize {
        match self.kind {
            TformKind::Bit => self.repeat.div_ceil(8),
            _ => self.repeat * self.kind.elem_size(),
        }
    }
}

/// The format letter of a `TDISPn` display format (§7.3.4, Table 20).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TDispKind {
    /// `Aw` character.
    Char,
    /// `Lw` logical.
    Logical,
    /// `Iw[.m]` integer.
    Integer,
    /// `Bw[.m]` binary.
    Binary,
    /// `Ow[.m]` octal.
    Octal,
    /// `Zw[.m]` hexadecimal.
    Hex,
    /// `Fw.d` fixed-point float.
    Float,
    /// `Ew.d[Ee]` exponential.
    Exponential,
    /// `ENw.d` engineering (exponent a multiple of 3).
    Engineering,
    /// `ESw.d` scientific (mantissa 1–10).
    Scientific,
    /// `Gw.d[Ee]` general.
    General,
    /// `Dw.d[Ee]` double-precision exponential.
    Double,
}

/// A parsed `TDISPn` display format: the format letter, field width, optional
/// decimal places (`.d`/`.m`), and optional exponent width (a trailing `Ee`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TDisp {
    pub kind: TDispKind,
    pub width: usize,
    pub decimals: Option<usize>,
    pub exponent: Option<usize>,
}

impl TDisp {
    /// Parse a `TDISPn` value such as `"I5"`, `"F8.2"`, `"E12.5E3"`, `"ES15.6"`, or
    /// `"A20"`. Returns `None` if the format letter or width is missing/invalid.
    pub fn parse(s: &str) -> Option<TDisp> {
        let s = s.trim().to_ascii_uppercase();
        let (kind, rest) = if let Some(r) = s.strip_prefix("EN") {
            (TDispKind::Engineering, r)
        } else if let Some(r) = s.strip_prefix("ES") {
            (TDispKind::Scientific, r)
        } else {
            let kind = match s.bytes().next()? {
                b'A' => TDispKind::Char,
                b'L' => TDispKind::Logical,
                b'I' => TDispKind::Integer,
                b'B' => TDispKind::Binary,
                b'O' => TDispKind::Octal,
                b'Z' => TDispKind::Hex,
                b'F' => TDispKind::Float,
                b'E' => TDispKind::Exponential,
                b'G' => TDispKind::General,
                b'D' => TDispKind::Double,
                _ => return None,
            };
            (kind, &s[1..])
        };
        // rest = width[.decimals][E exponent]
        let (main, exponent) = match rest.split_once('E') {
            Some((m, e)) => (m, Some(e.parse().ok()?)),
            None => (rest, None),
        };
        let (width, decimals) = match main.split_once('.') {
            Some((w, d)) => (w, Some(d.parse().ok()?)),
            None => (main, None),
        };
        Some(TDisp {
            kind,
            width: width.parse().ok()?,
            decimals,
            exponent,
        })
    }
}

/// One column of a binary table: its `TFORMn` format, optional name/unit, the
/// `TSCALn`/`TZEROn`/`TNULLn` metadata, and its byte offset within a row.
#[derive(Debug, Clone)]
pub struct Column {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub tform: Tform,
    /// `TSCALn` (default 1.0); applied by [`BinTable::read_column_physical`].
    pub tscale: f64,
    /// `TZEROn` (default 0.0); applied by [`BinTable::read_column_physical`].
    pub tzero: f64,
    /// `TNULLn`, the integer value denoting an undefined element, if declared.
    pub tnull: Option<i64>,
    /// `TDIMn` array shape (e.g. `'(4,4)'` → `[4, 4]`), if declared — reshapes the
    /// `repeat` elements of each row into a multidimensional array (§7.3.2).
    pub tdim: Option<Vec<usize>>,
    /// `TDISPn` display format (§7.3.4), parsed, if declared.
    pub tdisp: Option<TDisp>,
    /// Byte offset of this column from the start of a row.
    pub byte_offset: usize,
}

/// A decoded column, flattened across all rows in row order. For array columns
/// (`repeat > 1`) each row contributes `repeat` consecutive elements; for `A`,
/// each row contributes one [`String`]. Values are raw (big-endian decoded but
/// not `TSCALn`/`TZEROn`-scaled).
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    /// `L` — `Some(true)`/`Some(false)`, or `None` for the `0x00` null value (§7.3.3).
    Logical(Vec<Option<bool>>),
    /// `B` (bytes) and `X` (packed bits).
    Bytes(Vec<u8>),
    I16(Vec<i16>),
    I32(Vec<i32>),
    I64(Vec<i64>),
    F32(Vec<f32>),
    F64(Vec<f64>),
    ComplexF32(Vec<(f32, f32)>),
    ComplexF64(Vec<(f64, f64)>),
    /// `A` — one string per row, trailing spaces and NULs trimmed.
    Text(Vec<String>),
}

impl ColumnData {
    /// Total element count across all rows (the backing `Vec`'s length).
    pub fn element_count(&self) -> usize {
        match self {
            ColumnData::Logical(v) => v.len(),
            ColumnData::Bytes(v) => v.len(),
            ColumnData::I16(v) => v.len(),
            ColumnData::I32(v) => v.len(),
            ColumnData::I64(v) => v.len(),
            ColumnData::F32(v) => v.len(),
            ColumnData::F64(v) => v.len(),
            ColumnData::ComplexF32(v) => v.len(),
            ColumnData::ComplexF64(v) => v.len(),
            ColumnData::Text(v) => v.len(),
        }
    }
}

/// A binary table's structure plus its data unit.
#[derive(Debug, Clone)]
pub struct BinTable {
    pub nrows: usize,
    pub columns: Vec<Column>,
    pub(crate) row_len: usize,
    /// Byte offset of the heap within `bytes` (`THEAP`, default = main-table size).
    heap_offset: usize,
    /// Byte offset just past the real heap data (`nrows·row_len + PCOUNT`). `P`/`Q`
    /// spans must lie within `[heap_offset, heap_end)`, never the block fill beyond.
    heap_end: usize,
    /// The whole data unit (the `nrows * row_len` main table, then the heap and
    /// block fill). Fixed-width reads index the main-table prefix; `P`/`Q` columns
    /// follow their descriptors into the heap.
    bytes: Vec<u8>,
}

impl BinTable {
    /// Build a table from its header and owned data unit (`data` is the main
    /// table followed by the optional heap, as returned by the reader).
    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<BinTable> {
        let row_len = header
            .get_integer("NAXIS1")
            .ok_or(FitsError::MissingKeyword { name: "NAXIS1" })?
            .max(0) as usize;
        let nrows = header
            .get_integer("NAXIS2")
            .ok_or(FitsError::MissingKeyword { name: "NAXIS2" })?
            .max(0) as usize;
        // §7.3.1: `0 ≤ TFIELDS ≤ 999` — also a guard, since `tfields` sizes the
        // column `Vec` and drives the `TFORMn` loop (an absurd value would abort).
        let tfields = match header.get_integer("TFIELDS") {
            Some(t) if (0..=999).contains(&t) => t as usize,
            Some(_) => return Err(FitsError::WrongValueType { name: "TFIELDS" }),
            None => return Err(FitsError::MissingKeyword { name: "TFIELDS" }),
        };

        let mut columns = Vec::with_capacity(tfields);
        let mut offset = 0;
        for n in 1..=tfields {
            let tform_value = header
                .get_text(&format!("TFORM{n}"))
                .ok_or(FitsError::MissingKeyword { name: "TFORMn" })?;
            let tform = Tform::parse(tform_value)?;
            columns.push(Column {
                name: header
                    .get_text(&format!("TTYPE{n}"))
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(&format!("TUNIT{n}"))
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                tform,
                tscale: header.get_real(&format!("TSCAL{n}")).unwrap_or(1.0),
                tzero: header.get_real(&format!("TZERO{n}")).unwrap_or(0.0),
                tnull: header.get_integer(&format!("TNULL{n}")),
                tdim: header.get_text(&format!("TDIM{n}")).and_then(parse_tdim),
                tdisp: header.get_text(&format!("TDISP{n}")).and_then(TDisp::parse),
                byte_offset: offset,
            });
            offset += tform.byte_width();
        }
        if offset != row_len {
            return Err(FitsError::RowWidthMismatch {
                computed: offset,
                declared: row_len,
            });
        }

        if data.len() < nrows * row_len {
            return Err(FitsError::UnexpectedEof);
        }
        let heap_offset = header
            .get_integer("THEAP")
            .map_or(nrows * row_len, |t| t.max(0) as usize);
        // §6.6: the heap follows the main table, so THEAP must be ≥ its size.
        if heap_offset < nrows * row_len {
            return Err(FitsError::WrongValueType { name: "THEAP" });
        }
        // PCOUNT counts the gap-plus-heap bytes after the main table, so the real
        // heap ends here — anything past it is block fill (§6.6).
        let pcount = header
            .get_integer("PCOUNT")
            .map_or(0, |p| p.max(0) as usize);
        let heap_end = (nrows * row_len + pcount).min(data.len());
        Ok(BinTable {
            nrows,
            columns,
            row_len,
            heap_offset,
            heap_end,
            bytes: data,
        })
    }

    /// The fixed-width main table (`nrows × NAXIS1` bytes), excluding the heap.
    #[cfg(feature = "compression")]
    pub(crate) fn raw_rows(&self) -> &[u8] {
        &self.bytes[..self.nrows * self.row_len]
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| {
            c.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }

    /// Decode the fixed-width column at `index` into a typed, row-flattened
    /// [`ColumnData`]. Variable-length (`P`/`Q`) columns error here — use
    /// [`BinTable::read_vla_column`].
    pub fn read_column(&self, index: usize) -> Result<ColumnData> {
        let col = self.column(index)?;
        if matches!(
            col.tform.kind,
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64
        ) {
            return Err(FitsError::VariableLengthColumn {
                code: col.tform.kind.code(),
            });
        }
        // `A` is one string per row; every other fixed kind decodes uniformly
        // from the concatenated cell bytes — cell boundaries land on element
        // boundaries, so the flat decode is exact.
        Ok(if col.tform.kind == TformKind::Char {
            ColumnData::Text(
                (0..self.nrows)
                    .map(|r| trim_text(self.cell(col, r)))
                    .collect(),
            )
        } else {
            decode_array(col.tform.kind, &self.flatten(col))
        })
    }

    /// Decode an `X` (bit-array) column, unpacking each row's `repeat` bits
    /// MSB-first (bit 0 is the most significant bit of the first byte, §7.3.2)
    /// into one `Vec<bool>` per row. Errors on any non-`X` column.
    pub fn read_bit_column(&self, index: usize) -> Result<Vec<Vec<bool>>> {
        let col = self.column(index)?;
        if col.tform.kind != TformKind::Bit {
            return Err(FitsError::NotABitColumn {
                code: col.tform.kind.code(),
            });
        }
        let nbits = col.tform.repeat;
        let mut out = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            let cell = self.cell(col, r); // ceil(nbits/8) bytes
            let bits = (0..nbits)
                .map(|i| (cell[i / 8] >> (7 - (i % 8))) & 1 == 1)
                .collect();
            out.push(bits);
        }
        Ok(out)
    }

    /// Decode a variable-length `X` (bit-array) column (`1PX`/`1QX`), unpacking
    /// each row's bits MSB-first into one `Vec<bool>` (§7.3.2/§7.3.5 — the
    /// descriptor's element count is the bit count). Errors on any non-bit VLA.
    pub fn read_vla_bit_column(&self, index: usize) -> Result<Vec<Vec<bool>>> {
        let col = self.column(index)?;
        let wide = match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(TformKind::Bit)) => false,
            (TformKind::ArrayDesc64, Some(TformKind::Bit)) => true,
            _ => {
                return Err(FitsError::NotABitColumn {
                    code: col.tform.kind.code(),
                });
            }
        };
        let mut out = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            let desc = self.cell(col, r);
            let (nbits, offset) = if wide {
                (be_u64(&desc[0..8]), be_u64(&desc[8..16]))
            } else {
                (be_u32(&desc[0..4]), be_u32(&desc[4..8]))
            };
            let start = self
                .heap_offset
                .checked_add(offset)
                .ok_or(FitsError::UnexpectedEof)?;
            let end = start
                .checked_add(nbits.div_ceil(8))
                .ok_or(FitsError::UnexpectedEof)?;
            if end > self.heap_end {
                return Err(FitsError::UnexpectedEof);
            }
            let cell = self.bytes.get(start..end).ok_or(FitsError::UnexpectedEof)?;
            out.push(
                (0..nbits)
                    .map(|i| (cell[i / 8] >> (7 - (i % 8))) & 1 == 1)
                    .collect(),
            );
        }
        Ok(out)
    }

    /// Decode a numeric column and apply its scaling: `physical = TZEROn + TSCALn
    /// × raw`, mapping integers equal to `TNULLn` to `NaN`. Errors for the
    /// non-numeric kinds (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn read_column_physical(&self, index: usize) -> Result<Vec<f64>> {
        let col = self.column(index)?;
        let data = self.read_column(index)?;
        column_data_physical(&data, col.tform.kind, col.tscale, col.tzero, col.tnull)
    }

    /// If column `index` uses exactly the FITS unsigned (or signed-byte)
    /// convention — `TSCALn == 1`, no `TNULLn`, and `TZEROn` the matching sign-bit
    /// offset on a `B`/`I`/`J`/`K` column — decode it as exact typed integers (no
    /// `f64` rounding, unlike [`BinTable::read_column_physical`]). Array columns
    /// flatten row-major. Returns `Ok(None)` for any other column. Errors only on a
    /// bad index or a variable-length column. Mirrors [`crate::Image::unsigned`].
    pub fn read_column_unsigned(&self, index: usize) -> Result<Option<UnsignedView>> {
        let col = self.column(index)?;
        if col.tscale != 1.0 || col.tnull.is_some() {
            return Ok(None);
        }
        let tzero = col.tzero;
        Ok(match (self.read_column(index)?, col.tform.kind) {
            (ColumnData::Bytes(v), TformKind::Byte) if tzero == -128.0 => {
                Some(UnsignedView::from_signed_byte(&v))
            }
            (ColumnData::I16(v), _) if tzero == U16_OFFSET => {
                Some(UnsignedView::from_offset_i16(&v))
            }
            (ColumnData::I32(v), _) if tzero == U32_OFFSET => {
                Some(UnsignedView::from_offset_i32(&v))
            }
            (ColumnData::I64(v), _) if tzero == U64_OFFSET => {
                Some(UnsignedView::from_offset_i64(&v))
            }
            _ => None,
        })
    }

    /// Decode a `C`/`M` complex column into `f64` `(re, im)` pairs, applying
    /// `TZEROn + TSCALn ×` to each component (§6.4). Errors on non-complex columns —
    /// the real numeric kinds go through [`BinTable::read_column_physical`].
    pub fn read_column_complex(&self, index: usize) -> Result<Vec<(f64, f64)>> {
        let col = self.column(index)?;
        let scale = |x: f64| col.tzero + col.tscale * x;
        Ok(match self.read_column(index)? {
            ColumnData::ComplexF32(v) => v
                .iter()
                .map(|&(re, im)| (scale(re as f64), scale(im as f64)))
                .collect(),
            ColumnData::ComplexF64(v) => v.iter().map(|&(re, im)| (scale(re), scale(im))).collect(),
            _ => {
                return Err(FitsError::NotAComplexColumn {
                    code: col.tform.kind.code(),
                });
            }
        })
    }

    /// Decode a variable-length-array (`P`/`Q`) column: one [`ColumnData`] per
    /// row, each holding that row's heap array (which may be empty). Errors for
    /// fixed-width columns.
    pub fn read_vla_column(&self, index: usize) -> Result<Vec<ColumnData>> {
        let col = self.column(index)?;
        let (elem, wide) = match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(e)) => (e, false),
            (TformKind::ArrayDesc64, Some(e)) => (e, true),
            _ => {
                return Err(FitsError::NotAVla {
                    code: col.tform.kind.code(),
                });
            }
        };
        let mut out = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            let desc = self.cell(col, r);
            // The descriptor is (element count, byte offset into the heap), as a
            // pair of 32-bit (`P`) or 64-bit (`Q`) big-endian integers.
            let (nelem, offset) = if wide {
                (be_u64(&desc[0..8]), be_u64(&desc[8..16]))
            } else {
                (be_u32(&desc[0..4]), be_u32(&desc[4..8]))
            };
            let nbytes = match elem {
                TformKind::Bit => nelem.div_ceil(8),
                _ => nelem
                    .checked_mul(elem.elem_size())
                    .ok_or(FitsError::UnexpectedEof)?,
            };
            // Checked: a crafted `Q` descriptor (huge nelem/offset) must not wrap
            // past the heap-bounds guard. The span must lie within the heap proper.
            let start = self
                .heap_offset
                .checked_add(offset)
                .ok_or(FitsError::UnexpectedEof)?;
            let end = start.checked_add(nbytes).ok_or(FitsError::UnexpectedEof)?;
            if end > self.heap_end {
                return Err(FitsError::UnexpectedEof);
            }
            let slice = self.bytes.get(start..end).ok_or(FitsError::UnexpectedEof)?;
            out.push(decode_array(elem, slice));
        }
        Ok(out)
    }

    /// Decode a `P`/`Q` column and scale each row's heap array to its physical
    /// plane: `physical = TZEROn + TSCALn × element`, mapping integers equal to
    /// `TNULLn` to `NaN` (§6.4 — scaling applies to the heap values, not the
    /// descriptor). Errors for fixed-width columns and non-numeric heap elements.
    pub fn read_vla_column_physical(&self, index: usize) -> Result<Vec<Vec<f64>>> {
        let rows = self.read_vla_column(index)?; // validates VLA + heap bounds
        let col = self.column(index)?;
        let elem = col
            .tform
            .vla_elem
            .expect("read_vla_column succeeded ⇒ vla_elem is Some");
        rows.iter()
            .map(|row| column_data_physical(row, elem, col.tscale, col.tzero, col.tnull))
            .collect()
    }

    fn column(&self, index: usize) -> Result<&Column> {
        self.columns
            .get(index)
            .ok_or(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            })
    }

    /// The raw bytes of column `col` in row `r`.
    fn cell(&self, col: &Column, r: usize) -> &[u8] {
        let start = r * self.row_len + col.byte_offset;
        &self.bytes[start..start + col.tform.byte_width()]
    }

    /// Concatenate the raw cell bytes of `col` across every row.
    fn flatten(&self, col: &Column) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.nrows * col.tform.byte_width());
        for r in 0..self.nrows {
            out.extend_from_slice(self.cell(col, r));
        }
        out
    }
}

/// Parse a `TDIMn` value `'(d1,d2,…)'` into axis lengths (fastest-varying first).
fn parse_tdim(value: &str) -> Option<Vec<usize>> {
    let inner = value.trim().strip_prefix('(')?.strip_suffix(')')?;
    inner
        .split(',')
        .map(|s| s.trim().parse::<usize>().ok())
        .collect()
}

/// Scale a decoded numeric [`ColumnData`] to its physical `f64` plane:
/// `TZEROn + TSCALn × element`, mapping integers equal to `TNULLn` to `NaN`.
/// `kind` disambiguates `Bytes` (`B` integer vs `X` bits). Errors for the
/// non-numeric kinds (`A`/`L`/`X`/`C`/`M`).
fn column_data_physical(
    data: &ColumnData,
    kind: TformKind,
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
    Ok(match data {
        ColumnData::Bytes(v) if kind == TformKind::Byte => {
            v.iter().map(|&b| scaled_int(b as i64)).collect()
        }
        ColumnData::I16(v) => v.iter().map(|&x| scaled_int(x as i64)).collect(),
        ColumnData::I32(v) => v.iter().map(|&x| scaled_int(x as i64)).collect(),
        ColumnData::I64(v) => v.iter().map(|&x| scaled_int(x)).collect(),
        ColumnData::F32(v) => v.iter().map(|&x| scale(x as f64)).collect(),
        ColumnData::F64(v) => v.iter().map(|&x| scale(x)).collect(),
        _ => return Err(FitsError::NonNumericColumn { code: kind.code() }),
    })
}

/// Decode `bytes` as a contiguous run of `kind` elements. Shared by fixed-width
/// reads (concatenated cells) and heap arrays.
fn decode_array(kind: TformKind, bytes: &[u8]) -> ColumnData {
    match kind {
        TformKind::Logical => ColumnData::Logical(
            bytes
                .iter()
                .map(|&b| match b {
                    b'T' => Some(true),
                    b'F' => Some(false),
                    _ => None, // 0x00 (or any non-T/F byte) is the undefined value
                })
                .collect(),
        ),
        TformKind::Byte | TformKind::Bit => ColumnData::Bytes(bytes.to_vec()),
        TformKind::Char => ColumnData::Text(vec![trim_text(bytes)]),
        TformKind::I16 => ColumnData::I16(decode_be(bytes, i16::from_be_bytes)),
        TformKind::I32 => ColumnData::I32(decode_be(bytes, i32::from_be_bytes)),
        TformKind::I64 => ColumnData::I64(decode_be(bytes, i64::from_be_bytes)),
        TformKind::F32 => ColumnData::F32(decode_be(bytes, f32::from_be_bytes)),
        TformKind::F64 => ColumnData::F64(decode_be(bytes, f64::from_be_bytes)),
        TformKind::ComplexF32 => ColumnData::ComplexF32(decode_be(bytes, |b: [u8; 8]| {
            (
                f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                f32::from_be_bytes([b[4], b[5], b[6], b[7]]),
            )
        })),
        TformKind::ComplexF64 => ColumnData::ComplexF64(decode_be(bytes, |b: [u8; 16]| {
            (
                f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
                f64::from_be_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
            )
        })),
        // A heap element can't itself be a descriptor; keep the raw bytes.
        TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => ColumnData::Bytes(bytes.to_vec()),
    }
}

/// Decode an `A`-field cell: ASCII text truncated at the first NUL (§6.3 — a NUL
/// terminates the string early), then with trailing spaces removed.
fn trim_text(cell: &[u8]) -> String {
    let nul = cell.iter().position(|&b| b == 0).unwrap_or(cell.len());
    let head = &cell[..nul];
    let end = head.iter().rposition(|&b| b != b' ').map_or(0, |i| i + 1);
    String::from_utf8_lossy(&head[..end]).into_owned()
}

fn be_u32(b: &[u8]) -> usize {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize
}

fn be_u64(b: &[u8]) -> usize {
    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]).max(0) as usize
}

#[cfg(test)]
mod tests;
