//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. This
//! module parses that structure into [`Column`] descriptors and decodes the
//! fixed-width fields into typed [`ColumnData`]. Variable-length arrays (`P`/`Q`
//! descriptors into the heap) and per-column `TSCALn`/`TZEROn` physical scaling
//! are parsed-for-layout but not yet decoded — see [`ColumnData`].

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

/// A parsed `TFORMn` value: a repeat count and an element kind. The `rTa` form's
/// trailing `a` (e.g. the `(emax)` of a variable-length array) is not retained.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Tform {
    pub repeat: usize,
    pub kind: TformKind,
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
        Ok(Tform { repeat, kind })
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
    /// `TSCALn` (default 1.0) — not yet applied by [`BinTable::read_column`].
    pub tscale: f64,
    /// `TZEROn` (default 0.0) — not yet applied by [`BinTable::read_column`].
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
    /// The whole data unit (the `nrows * row_len` main table, then the heap and
    /// block fill). Column reads index the main-table prefix; the heap is kept
    /// for future `P`/`Q` variable-length-array decoding.
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
        Ok(BinTable {
            nrows,
            columns,
            row_len,
            bytes: data,
        })
    }

    /// The index of the first column with this (case-sensitive) name.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns
            .iter()
            .position(|c| c.name.as_deref() == Some(name))
    }

    /// Decode the column at `index` into a typed, row-flattened [`ColumnData`].
    pub fn read_column(&self, index: usize) -> Result<ColumnData> {
        let col = self
            .columns
            .get(index)
            .ok_or(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            })?;
        Ok(match col.tform.kind {
            TformKind::Char => ColumnData::Text(
                (0..self.nrows)
                    .map(|r| trim_text(self.cell(col, r)))
                    .collect(),
            ),
            TformKind::Logical => {
                ColumnData::Logical(self.flatten(col).iter().map(|&b| b == b'T').collect())
            }
            TformKind::Byte | TformKind::Bit => ColumnData::Bytes(self.flatten(col)),
            TformKind::I16 => ColumnData::I16(self.elems(col, i16::from_be_bytes)),
            TformKind::I32 => ColumnData::I32(self.elems(col, i32::from_be_bytes)),
            TformKind::I64 => ColumnData::I64(self.elems(col, i64::from_be_bytes)),
            TformKind::F32 => ColumnData::F32(self.elems(col, f32::from_be_bytes)),
            TformKind::F64 => ColumnData::F64(self.elems(col, f64::from_be_bytes)),
            TformKind::ComplexF32 => ColumnData::ComplexF32(self.elems(col, |b: [u8; 8]| {
                (
                    f32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                    f32::from_be_bytes([b[4], b[5], b[6], b[7]]),
                )
            })),
            TformKind::ComplexF64 => ColumnData::ComplexF64(self.elems(col, |b: [u8; 16]| {
                (
                    f64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]),
                    f64::from_be_bytes([b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]]),
                )
            })),
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
                return Err(FitsError::UnsupportedColumn {
                    tform: format!("{}{}", col.tform.repeat, col.tform.kind.code()),
                });
            }
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

    /// Decode every `N`-byte big-endian element of `col` across all rows.
    fn elems<const N: usize, T>(&self, col: &Column, conv: fn([u8; N]) -> T) -> Vec<T> {
        let mut out = Vec::with_capacity(self.nrows * col.tform.repeat);
        for r in 0..self.nrows {
            for chunk in self.cell(col, r).chunks_exact(N) {
                out.push(conv(
                    chunk.try_into().expect("chunks_exact yields N-byte arrays"),
                ));
            }
        }
        out
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::FitsReader;
    use std::fs::File;

    fn table_header(naxis1: usize, naxis2: usize, tforms: &[&str]) -> Header {
        let mut h = Header::new();
        h.set("XTENSION", "BINTABLE")
            .set("BITPIX", 8)
            .set("NAXIS", 2)
            .set("NAXIS1", naxis1 as i64)
            .set("NAXIS2", naxis2 as i64)
            .set("PCOUNT", 0)
            .set("GCOUNT", 1)
            .set("TFIELDS", tforms.len() as i64);
        for (i, tform) in tforms.iter().enumerate() {
            h.set(&format!("TFORM{}", i + 1), *tform);
        }
        h
    }

    #[test]
    fn parses_tform_repeat_and_kind() {
        let cases = [
            ("8A", 8, TformKind::Char),
            ("3D", 3, TformKind::F64),
            ("0D", 0, TformKind::F64),
            ("1J", 1, TformKind::I32),
            ("E", 1, TformKind::F32), // bare code ⇒ repeat 1
            ("16X", 16, TformKind::Bit),
            ("1PE(5)", 1, TformKind::ArrayDesc32),
        ];
        for (s, repeat, kind) in cases {
            assert_eq!(Tform::parse(s).unwrap(), Tform { repeat, kind }, "{s}");
        }
        assert!(matches!(
            Tform::parse("9Z"),
            Err(FitsError::InvalidTform { .. })
        ));
        assert!(matches!(
            Tform::parse(""),
            Err(FitsError::InvalidTform { .. })
        ));
    }

    #[test]
    fn byte_width_handles_arrays_bits_and_descriptors() {
        assert_eq!(Tform::parse("8A").unwrap().byte_width(), 8);
        assert_eq!(Tform::parse("3D").unwrap().byte_width(), 24);
        assert_eq!(Tform::parse("0D").unwrap().byte_width(), 0);
        assert_eq!(Tform::parse("16X").unwrap().byte_width(), 2); // 16 bits = 2 bytes
        assert_eq!(Tform::parse("9X").unwrap().byte_width(), 2); //  9 bits = 2 bytes
        assert_eq!(Tform::parse("1P").unwrap().byte_width(), 8); // 32-bit descriptor
    }

    #[test]
    fn decodes_fixed_width_columns_from_hand_built_data() {
        // 1J (i32) | 2E (two f32) | 3A (string)  →  row width 4 + 8 + 3 = 15.
        let header = table_header(15, 2, &["1J", "2E", "3A"]);
        let mut data = Vec::new();
        for (j, e0, e1, text) in [(1i32, 1.0f32, 2.0f32, b"ABC"), (2, 3.0, 4.0, b"DE ")] {
            data.extend_from_slice(&j.to_be_bytes());
            data.extend_from_slice(&e0.to_be_bytes());
            data.extend_from_slice(&e1.to_be_bytes());
            data.extend_from_slice(text);
        }

        let table = BinTable::from_data(&header, data).unwrap();
        assert_eq!(table.nrows, 2);
        assert_eq!(
            table
                .columns
                .iter()
                .map(|c| c.byte_offset)
                .collect::<Vec<_>>(),
            vec![0, 4, 12]
        );
        assert_eq!(table.read_column(0).unwrap(), ColumnData::I32(vec![1, 2]));
        assert_eq!(
            table.read_column(1).unwrap(),
            ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0])
        );
        assert_eq!(
            table.read_column(2).unwrap(),
            ColumnData::Text(vec!["ABC".into(), "DE".into()]) // trailing space trimmed
        );
    }

    #[test]
    fn zero_repeat_column_decodes_to_empty() {
        let header = table_header(4, 1, &["0D", "1J"]);
        let data = 7i32.to_be_bytes().to_vec();
        let table = BinTable::from_data(&header, data).unwrap();
        assert_eq!(table.read_column(0).unwrap(), ColumnData::F64(vec![]));
        assert_eq!(table.read_column(1).unwrap(), ColumnData::I32(vec![7]));
    }

    #[test]
    fn variable_length_array_column_is_reported_unsupported() {
        let header = table_header(8, 1, &["1PE(3)"]);
        let table = BinTable::from_data(&header, vec![0u8; 8]).unwrap();
        assert!(matches!(
            table.read_column(0),
            Err(FitsError::UnsupportedColumn { .. })
        ));
    }

    #[test]
    fn row_width_mismatch_is_an_error() {
        // Declared NAXIS1 = 99 but the one column is only 4 bytes wide.
        let header = table_header(99, 1, &["1J"]);
        assert!(matches!(
            BinTable::from_data(&header, vec![0u8; 4]),
            Err(FitsError::RowWidthMismatch {
                computed: 4,
                declared: 99
            })
        ));
    }

    #[test]
    fn out_of_bounds_column_is_an_error() {
        let header = table_header(4, 1, &["1J"]);
        let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
        assert!(matches!(
            table.read_column(9),
            Err(FitsError::ColumnIndexOutOfBounds { index: 9, len: 1 })
        ));
    }

    #[test]
    fn reads_the_real_aips_antenna_table() {
        let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
        let mut reader = FitsReader::open(file).unwrap();
        let table = reader.read_table(1).unwrap();

        assert_eq!(table.nrows, 28);
        assert_eq!(table.columns.len(), 12);
        // ANNAME = 8A, STABXYZ = 3D, ORBPARM = 0D, NOSTA = 1J ...
        assert_eq!(table.columns[0].name.as_deref(), Some("ANNAME"));
        assert_eq!(
            table.columns[0].tform,
            Tform {
                repeat: 8,
                kind: TformKind::Char
            }
        );
        assert_eq!(
            table.columns[1].tform,
            Tform {
                repeat: 3,
                kind: TformKind::F64
            }
        );
        assert_eq!(
            table.columns[2].tform,
            Tform {
                repeat: 0,
                kind: TformKind::F64
            }
        );
        // The 0D ORBPARM column contributes no width, so NOSTA shares its offset.
        assert_eq!(table.columns[2].byte_offset, 32);
        assert_eq!(table.columns[3].byte_offset, 32);
        assert_eq!(table.columns[1].unit.as_deref(), Some("METERS"));

        // Decoded element counts: one ANNAME string per row, 3 doubles per row, none for 0D.
        match table.read_column(0).unwrap() {
            ColumnData::Text(v) => assert_eq!(v.len(), 28),
            other => panic!("ANNAME should be Text, got {other:?}"),
        }
        match table.read_column(1).unwrap() {
            ColumnData::F64(v) => assert_eq!(v.len(), 28 * 3),
            other => panic!("STABXYZ should be F64, got {other:?}"),
        }
        assert_eq!(table.read_column(2).unwrap(), ColumnData::F64(vec![]));
        assert_eq!(table.column_index("NOSTA"), Some(3));
    }

    #[test]
    fn read_table_rejects_non_bintable_hdus() {
        let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
        let mut reader = FitsReader::open(file).unwrap();
        // HDU 0 is a random-groups primary, not a binary table.
        assert!(matches!(reader.read_table(0), Err(FitsError::NotABinTable)));
    }
}
