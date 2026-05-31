//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. This
//! module parses that structure into [`Column`] descriptors and decodes:
//! fixed-width fields into typed [`ColumnData`] ([`BinTable::read_column`]), the
//! `TSCALn`/`TZEROn` physical plane ([`BinTable::read_column_physical`]), and
//! `P`/`Q` variable-length arrays out of the heap ([`BinTable::read_vla_column`]).

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
    /// Byte offset of this column from the start of a row.
    pub byte_offset: usize,
}

/// A decoded column, flattened across all rows in row order. For array columns
/// (`repeat > 1`) each row contributes `repeat` consecutive elements; for `A`,
/// each row contributes one [`String`]. Values are raw (big-endian decoded but
/// not `TSCALn`/`TZEROn`-scaled).
#[derive(Debug, Clone, PartialEq)]
pub enum ColumnData {
    Logical(Vec<bool>),
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

/// A binary table's structure plus its data unit.
#[derive(Debug, Clone)]
pub struct BinTable {
    pub nrows: usize,
    pub columns: Vec<Column>,
    row_len: usize,
    /// Byte offset of the heap within `bytes` (`THEAP`, default = main-table size).
    heap_offset: usize,
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
        let tfields = header
            .get_integer("TFIELDS")
            .ok_or(FitsError::MissingKeyword { name: "TFIELDS" })?
            .max(0) as usize;

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
        Ok(BinTable {
            nrows,
            columns,
            row_len,
            heap_offset,
            bytes: data,
        })
    }

    /// The fixed-width main table (`nrows × NAXIS1` bytes), excluding the heap.
    #[cfg(feature = "compression")]
    pub(crate) fn raw_rows(&self) -> &[u8] {
        &self.bytes[..self.nrows * self.row_len]
    }

    /// Row width in bytes (`NAXIS1`).
    #[cfg(feature = "compression")]
    pub(crate) fn row_width(&self) -> usize {
        self.row_len
    }

    /// The index of the first column with this (case-sensitive) name.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.as_deref() == Some(name))
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

    /// Decode a numeric column and apply its scaling: `physical = TZEROn + TSCALn
    /// × raw`, mapping integers equal to `TNULLn` to `NaN`. Errors for the
    /// non-numeric kinds (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn read_column_physical(&self, index: usize) -> Result<Vec<f64>> {
        let col = self.column(index)?;
        let scale = |x: f64| col.tzero + col.tscale * x;
        let tnull = col.tnull;
        let scaled_int = |xi: i64| {
            if tnull == Some(xi) {
                f64::NAN
            } else {
                scale(xi as f64)
            }
        };
        Ok(match self.read_column(index)? {
            ColumnData::Bytes(v) if col.tform.kind == TformKind::Byte => {
                v.iter().map(|&b| scaled_int(b as i64)).collect()
            }
            ColumnData::I16(v) => v.iter().map(|&x| scaled_int(x as i64)).collect(),
            ColumnData::I32(v) => v.iter().map(|&x| scaled_int(x as i64)).collect(),
            ColumnData::I64(v) => v.iter().map(|&x| scaled_int(x)).collect(),
            ColumnData::F32(v) => v.iter().map(|&x| scale(x as f64)).collect(),
            ColumnData::F64(v) => v.iter().map(|&x| scale(x)).collect(),
            _ => {
                return Err(FitsError::NonNumericColumn {
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
                _ => nelem * elem.elem_size(),
            };
            let start = self.heap_offset + offset;
            let slice = self
                .bytes
                .get(start..start + nbytes)
                .ok_or(FitsError::UnexpectedEof)?;
            out.push(decode_array(elem, slice));
        }
        Ok(out)
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

/// Decode `bytes` as a contiguous run of `kind` elements. Shared by fixed-width
/// reads (concatenated cells) and heap arrays.
fn decode_array(kind: TformKind, bytes: &[u8]) -> ColumnData {
    match kind {
        TformKind::Logical => ColumnData::Logical(bytes.iter().map(|&b| b == b'T').collect()),
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

/// Decode an `A`-field cell: ASCII text with trailing spaces and NULs trimmed.
fn trim_text(cell: &[u8]) -> String {
    let end = cell
        .iter()
        .rposition(|&b| b != b' ' && b != 0)
        .map_or(0, |i| i + 1);
    String::from_utf8_lossy(&cell[..end]).into_owned()
}

fn be_u32(b: &[u8]) -> usize {
    i32::from_be_bytes([b[0], b[1], b[2], b[3]]).max(0) as usize
}

fn be_u64(b: &[u8]) -> usize {
    i64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]).max(0) as usize
}

#[cfg(test)]
mod tests;
