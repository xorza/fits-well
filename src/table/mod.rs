//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. This
//! module parses that structure into [`Column`] descriptors; decoding goes through
//! a [`ColumnReader`] (from [`BinTable::column_by_idx`] / [`BinTable::column_by_name`]),
//! whose methods yield typed [`ColumnData`] ([`ColumnReader::raw`]), the
//! `TSCALn`/`TZEROn` physical plane ([`ColumnReader::physical`]), and `P`/`Q`
//! variable-length arrays out of the heap ([`ColumnReader::vla`]).

use std::ops::Index;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::view::BitView;
use num_complex::Complex;

use crate::data::U16_OFFSET;
use crate::data::U32_OFFSET;
use crate::data::U64_OFFSET;
use crate::data::UnsignedView;
use crate::endian::decode_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

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
    pub(crate) fn elem_size(self) -> usize {
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
            // Saturating: an absurd `repeat` from a hostile `TFORMn` saturates to
            // `usize::MAX` rather than wrapping to a small width that could slip
            // past the row-width check in `from_data`.
            _ => self.repeat.saturating_mul(self.kind.elem_size()),
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
    /// `TSCALn` (default 1.0); applied by [`ColumnReader::physical`].
    pub tscale: f64,
    /// `TZEROn` (default 0.0); applied by [`ColumnReader::physical`].
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
    ComplexF32(Vec<Complex<f32>>),
    ComplexF64(Vec<Complex<f64>>),
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
            Some(_) => return Err(FitsError::KeywordOutOfRange { name: "TFIELDS" }),
            None => return Err(FitsError::MissingKeyword { name: "TFIELDS" }),
        };

        let mut columns = Vec::with_capacity(tfields);
        let mut offset = 0;
        for n in 1..=tfields {
            let tform_value = header
                .get_text(key!("TFORM{n}").as_str())
                .ok_or(FitsError::MissingKeyword { name: "TFORMn" })?;
            let tform = Tform::parse(tform_value)?;
            let tdim = header
                .get_text(key!("TDIM{n}").as_str())
                .and_then(parse_tdim);
            // §7.3.2: for a fixed-width column a `TDIMn` shape must reshape exactly the
            // repeat count (checked product so a hostile shape can't overflow past the
            // equality). Variable-length (`P`/`Q`) columns are exempt — there `TDIMn`
            // describes the heap array's shape, not the descriptor repeat (1), as in a
            // §10.3 compressed-table container that carries the original column's TDIM.
            let is_vla = matches!(tform.kind, TformKind::ArrayDesc32 | TformKind::ArrayDesc64);
            if let Some(dims) = &tdim
                && !is_vla
                && dims.iter().try_fold(1usize, |a, &x| a.checked_mul(x)) != Some(tform.repeat)
            {
                return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
            }
            columns.push(Column {
                name: header
                    .get_text(key!("TTYPE{n}").as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(key!("TUNIT{n}").as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                tform,
                tscale: header.get_real(key!("TSCAL{n}").as_str()).unwrap_or(1.0),
                tzero: header.get_real(key!("TZERO{n}").as_str()).unwrap_or(0.0),
                tnull: header.get_integer(key!("TNULL{n}").as_str()),
                tdim,
                tdisp: header
                    .get_text(key!("TDISP{n}").as_str())
                    .and_then(TDisp::parse),
                byte_offset: offset,
            });
            offset = offset.saturating_add(tform.byte_width());
        }
        if offset != row_len {
            return Err(FitsError::RowWidthMismatch {
                computed: offset,
                declared: row_len,
            });
        }

        // `nrows · row_len` from untrusted axes: check once (guards a 32-bit-usize
        // overflow that `data_extent`'s u64 math wouldn't catch) and reuse.
        let main_table = nrows.checked_mul(row_len).ok_or(FitsError::UnexpectedEof)?;
        if data.len() < main_table {
            return Err(FitsError::UnexpectedEof);
        }
        let heap_offset = header
            .get_integer("THEAP")
            .map_or(main_table, |t| t.max(0) as usize);
        // §6.6: the heap follows the main table, so THEAP must be ≥ its size.
        if heap_offset < main_table {
            return Err(FitsError::KeywordOutOfRange { name: "THEAP" });
        }
        // PCOUNT counts the gap-plus-heap bytes after the main table, so the real
        // heap ends here — anything past it is block fill (§6.6).
        let pcount = header
            .get_integer("PCOUNT")
            .map_or(0, |p| p.max(0) as usize);
        let heap_end = main_table
            .checked_add(pcount)
            .ok_or(FitsError::UnexpectedEof)?
            .min(data.len());
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

    fn column_index_checked(&self, name: &str) -> Result<usize> {
        self.column_index(name)
            .ok_or_else(|| FitsError::ColumnNotFound {
                name: name.to_string(),
            })
    }

    /// A reader handle for the column at `index`. Decode through it — [`ColumnReader`]
    /// exposes `raw`/`physical`/`unsigned`/`complex`/`bits` and the `vla*` variants —
    /// without re-passing the column descriptor. Errors with
    /// [`FitsError::ColumnIndexOutOfBounds`] for a bad index.
    pub fn column_by_idx(&self, index: usize) -> Result<ColumnReader<'_>> {
        if index >= self.columns.len() {
            return Err(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            });
        }
        Ok(ColumnReader { table: self, index })
    }

    /// A reader handle for the column named `name` (`TTYPEn`, case-insensitive, §6.7).
    /// Errors with [`FitsError::ColumnNotFound`] if no such column exists.
    pub fn column_by_name(&self, name: &str) -> Result<ColumnReader<'_>> {
        let index = self.column_index_checked(name)?;
        Ok(ColumnReader { table: self, index })
    }

    /// The `nbytes` of heap at descriptor `offset`, bounds-checked against the heap.
    /// All arithmetic is checked so a crafted `P`/`Q` descriptor (huge offset/count)
    /// cannot wrap past the guard or read outside the heap proper.
    fn bounded_heap(&self, offset: usize, nbytes: usize) -> Result<&[u8]> {
        let start = self
            .heap_offset
            .checked_add(offset)
            .ok_or(FitsError::UnexpectedEof)?;
        let end = start.checked_add(nbytes).ok_or(FitsError::UnexpectedEof)?;
        if end > self.heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        self.bytes.get(start..end).ok_or(FitsError::UnexpectedEof)
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

/// A handle to one column of a [`BinTable`], from [`BinTable::column_by_idx`] or
/// [`BinTable::column_by_name`]. Decode through it without re-passing the column
/// descriptor: [`raw`](Self::raw) for the typed values, [`physical`](Self::physical)
/// for the scaled `f64` plane, [`unsigned`](Self::unsigned)/[`complex`](Self::complex)/
/// [`bits`](Self::bits) for the special kinds, and [`vla`](Self::vla) (+
/// [`vla_physical`](Self::vla_physical)/[`vla_bits`](Self::vla_bits)) for
/// variable-length `P`/`Q` columns. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub struct ColumnReader<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> ColumnReader<'a> {
    /// The column's [`Column`] descriptor — name, `TFORMn`, `TSCALn`/`TZEROn`/`TNULLn`,
    /// `TDIMn`, `TDISPn`.
    pub fn descriptor(&self) -> &'a Column {
        &self.table.columns[self.index]
    }

    /// Decode a fixed-width column into a typed, row-flattened [`ColumnData`]: `A` is
    /// one [`String`] per row, every other fixed kind decodes from the concatenated
    /// cell bytes. Variable-length (`P`/`Q`) columns error here — use
    /// [`ColumnReader::vla`].
    pub fn raw(&self) -> Result<ColumnData> {
        let col = self.descriptor();
        if matches!(
            col.tform.kind,
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64
        ) {
            return Err(FitsError::VariableLengthColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(if col.tform.kind == TformKind::Char {
            ColumnData::Text(
                (0..self.table.nrows)
                    .map(|r| trim_text(self.table.cell(col, r)))
                    .collect(),
            )
        } else {
            decode_array(col.tform.kind, &self.table.flatten(col))
        })
    }

    /// The numeric column scaled to its physical `f64` plane: `TZEROn + TSCALn × raw`,
    /// mapping integers equal to `TNULLn` to `NaN`. Errors for the non-numeric kinds
    /// (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn physical(&self) -> Result<Vec<f64>> {
        let col = self.descriptor();
        column_data_physical(
            &self.raw()?,
            col.tform.kind,
            col.tscale,
            col.tzero,
            col.tnull,
        )
    }

    /// Exact typed integers when the column uses the FITS unsigned (or signed-byte)
    /// convention — `TSCALn == 1`, no `TNULLn`, `TZEROn` the matching sign-bit offset
    /// on a `B`/`I`/`J`/`K` column — without the `f64` rounding of
    /// [`physical`](Self::physical). `Ok(None)` for any other column; errors only for a
    /// variable-length column. Mirrors [`crate::Image::unsigned`].
    pub fn unsigned(&self) -> Result<Option<UnsignedView>> {
        let col = self.descriptor();
        if col.tscale != 1.0 || col.tnull.is_some() {
            return Ok(None);
        }
        let tzero = col.tzero;
        Ok(match (self.raw()?, col.tform.kind) {
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

    /// A `C`/`M` complex column as [`Complex<f64>`] values, applying `TZEROn + TSCALn ×`
    /// to each component (§6.4). Errors on non-complex columns.
    pub fn complex(&self) -> Result<Vec<Complex<f64>>> {
        let col = self.descriptor();
        let scale = |re: f64, im: f64| Complex {
            re: col.tzero + col.tscale * re,
            im: col.tzero + col.tscale * im,
        };
        Ok(match self.raw()? {
            ColumnData::ComplexF32(v) => v
                .iter()
                .map(|&Complex { re, im }| scale(re as f64, im as f64))
                .collect(),
            ColumnData::ComplexF64(v) => {
                v.iter().map(|&Complex { re, im }| scale(re, im)).collect()
            }
            _ => {
                return Err(FitsError::NotAComplexColumn {
                    code: col.tform.kind.code(),
                });
            }
        })
    }

    /// An `X` (bit-array) column as a borrowed 2-D [`BitColumn`] — `nrows × repeat`
    /// bits viewed in place over the data unit, MSB-first (bit 0 is the MSB of the
    /// first byte, §7.3.2), with no per-row allocation. Errors on any non-`X` column.
    pub fn bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        if col.tform.kind != TformKind::Bit {
            return Err(FitsError::NotABitColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(BitColumn {
            table: self.table,
            index: self.index,
        })
    }

    /// Decode a variable-length (`P`/`Q`) column: one [`ColumnData`] per row, each
    /// holding that row's heap array (which may be empty). Errors for fixed-width
    /// columns.
    pub fn vla(&self) -> Result<Vec<ColumnData>> {
        let col = self.descriptor();
        let (elem, wide) = match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(e)) => (e, false),
            (TformKind::ArrayDesc64, Some(e)) => (e, true),
            _ => {
                return Err(FitsError::NotAVla {
                    code: col.tform.kind.code(),
                });
            }
        };
        let mut out = Vec::with_capacity(self.table.nrows);
        for r in 0..self.table.nrows {
            let d = decode_descriptor(self.table.cell(col, r), wide);
            let nbytes = match elem {
                TformKind::Bit => d.nelem.div_ceil(8),
                _ => d
                    .nelem
                    .checked_mul(elem.elem_size())
                    .ok_or(FitsError::UnexpectedEof)?,
            };
            out.push(decode_array(
                elem,
                self.table.bounded_heap(d.offset, nbytes)?,
            ));
        }
        Ok(out)
    }

    /// Scale each row of a `P`/`Q` column to its physical plane: `TZEROn + TSCALn ×
    /// element`, mapping integers equal to `TNULLn` to `NaN` (§6.4 — scaling applies to
    /// the heap values). Errors for fixed-width or non-numeric-heap columns.
    pub fn vla_physical(&self) -> Result<Vec<Vec<f64>>> {
        let rows = self.vla()?; // validates VLA + heap bounds
        let col = self.descriptor();
        let elem = col
            .tform
            .vla_elem
            .expect("vla() succeeded ⇒ vla_elem is Some");
        rows.iter()
            .map(|row| column_data_physical(row, elem, col.tscale, col.tzero, col.tnull))
            .collect()
    }

    /// A variable-length `X` (`1PX`/`1QX`) column as a borrowed 2-D [`BitColumn`],
    /// MSB-first (§7.3.2/§7.3.5 — the descriptor's element count is the bit count). The
    /// rows are *jagged* (each its own length), so [`BitColumn::row`]`(r).len()` gives a
    /// row's width. Errors on any non-bit VLA.
    pub fn vla_bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        let wide = match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(TformKind::Bit)) => false,
            (TformKind::ArrayDesc64, Some(TformKind::Bit)) => true,
            _ => {
                return Err(FitsError::NotABitColumn {
                    code: col.tform.kind.code(),
                });
            }
        };
        // Validate every row's heap span up front (no allocation) so [`BitColumn::row`]
        // can resolve a row lazily and infallibly — the only place an overrun surfaces.
        for r in 0..self.table.nrows {
            let d = decode_descriptor(self.table.cell(col, r), wide);
            self.table.bounded_heap(d.offset, d.nelem.div_ceil(8))?;
        }
        Ok(BitColumn {
            table: self.table,
            index: self.index,
        })
    }
}

/// A binary table's `X` (bit-array) column as a borrowed, 2-D bit view — from
/// [`ColumnReader::bits`] (rectangular, `nrows × repeat`) or [`ColumnReader::vla_bits`]
/// (jagged `PX`/`QX`). Bits are MSB-first (§7.3.2) and viewed in place over the data
/// unit (zero-copy), so this borrows the table and can't outlive it.
///
/// Index a row (`flags[row]` → a [`BitSlice`]), a bit by nesting (`flags[row][col]`)
/// or by cell (`flags[(row, col)]`), reach for the checked [`get`](Self::get), or take
/// a row with the source lifetime via [`row`](Self::row). Rows are full `bitvec`
/// slices — `count_ones()`, `iter_ones()`, `.to_bitvec()` to own, etc.
///
/// ```ignore
/// let flags = table.column_by_name("DQ")?.bits()?;
/// let bit = flags[(row, 3)];             // bool (panics out of range)
/// let bit = flags[row][3];               // same, via the row slice
/// let bit = flags.get(row, 3);           // Option<bool> (checked)
/// let set = flags[row].count_ones();     // bitvec ops on the row
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BitColumn<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> BitColumn<'a> {
    /// The number of rows.
    pub fn nrows(&self) -> usize {
        self.table.nrows
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.table.nrows == 0
    }

    /// Row `r`'s bits as a borrowed [`BitSlice`], MSB-first — resolved on demand from
    /// the data unit (no per-row storage). Index it (`row[c]`), iterate it, or
    /// `.to_bitvec()` to own it. Panics if `r >= nrows()`.
    pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0> {
        assert!(
            r < self.table.nrows,
            "row {r} out of bounds ({} rows)",
            self.table.nrows
        );
        let col = &self.table.columns[self.index];
        if col.tform.kind == TformKind::Bit {
            // Fixed `rX`: the row's cell, truncated to `repeat` bits.
            &self.table.cell(col, r).view_bits::<Msb0>()[..col.tform.repeat]
        } else {
            // Variable-length `PX`/`QX`: follow the descriptor into the heap. The span
            // was bounds-checked by `vla_bits`, so the lookup can't fail here.
            let wide = col.tform.kind == TformKind::ArrayDesc64;
            let d = decode_descriptor(self.table.cell(col, r), wide);
            let cell = self
                .table
                .bounded_heap(d.offset, d.nelem.div_ceil(8))
                .expect("vla_bits validated every heap span");
            &cell.view_bits::<Msb0>()[..d.nelem]
        }
    }

    /// The bit at `(row, col)`, MSB-first — `None` if either index is out of range.
    pub fn get(&self, row: usize, col: usize) -> Option<bool> {
        if row >= self.table.nrows {
            return None;
        }
        let bits = self.row(row);
        (col < bits.len()).then(|| bits[col])
    }

    /// Iterate the rows, each a borrowed [`BitSlice`], resolved on demand.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a BitSlice<u8, Msb0>> + '_ {
        (0..self.table.nrows).map(move |r| self.row(r))
    }
}

/// `bits[row]` is row `row`'s [`BitSlice`] (panics out of range, like slice indexing);
/// `bits[row][col]` is the bit. Use [`BitColumn::get`] for the checked element.
impl Index<usize> for BitColumn<'_> {
    type Output = BitSlice<u8, Msb0>;

    fn index(&self, row: usize) -> &BitSlice<u8, Msb0> {
        self.row(row)
    }
}

/// `bits[(row, col)]` is the bit at that cell (panics out of range) — the matrix-style
/// counterpart of [`BitColumn::get`].
impl Index<(usize, usize)> for BitColumn<'_> {
    type Output = bool;

    fn index(&self, (row, col): (usize, usize)) -> &bool {
        &self.row(row)[col]
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
        TformKind::ComplexF32 => ColumnData::ComplexF32(decode_be(bytes, |b: [u8; 8]| Complex {
            re: f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
            im: f32::from_be_bytes([b[4], b[5], b[6], b[7]]),
        })),
        TformKind::ComplexF64 => ColumnData::ComplexF64(decode_be(bytes, |b: [u8; 16]| Complex {
            re: f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
            im: f64::from_be_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
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

/// A decoded `P`/`Q` array descriptor: a row's heap array element count and byte
/// offset into the heap.
#[derive(Debug, Clone, Copy)]
struct Descriptor {
    nelem: usize,
    offset: usize,
}

/// Decode an array descriptor — a pair of 32-bit (`P`) or 64-bit (`Q`) big-endian
/// integers — from a variable-length column cell.
fn decode_descriptor(desc: &[u8], wide: bool) -> Descriptor {
    if wide {
        Descriptor {
            nelem: be_u64(&desc[0..8]),
            offset: be_u64(&desc[8..16]),
        }
    } else {
        Descriptor {
            nelem: be_u32(&desc[0..4]),
            offset: be_u32(&desc[4..8]),
        }
    }
}

/// Decode a big-endian `P`/`Q` array-descriptor field (element count or heap
/// offset). The standard treats these as unsigned; an out-of-range value is left
/// to the heap-bounds check to reject (rather than silently clamping it to 0).
fn be_u32(b: &[u8]) -> usize {
    u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize
}

fn be_u64(b: &[u8]) -> usize {
    // On a 32-bit target a `Q` count/offset can exceed `usize`; saturate so it fails
    // the heap bounds check rather than wrapping into a spuriously in-range value.
    // On 64-bit this is the identity (`usize == u64`).
    usize::try_from(u64::from_be_bytes([
        b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7],
    ]))
    .unwrap_or(usize::MAX)
}

#[cfg(test)]
mod tests;
