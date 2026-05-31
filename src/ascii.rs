//! ASCII-table extension (§7.2): `TABLE`.
//!
//! Rows are fixed-length lines of ASCII text; each column occupies a fixed byte
//! range starting at `TBCOLn` (1-based), formatted per a Fortran `TFORMn` code
//! (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`). Decoded values reuse [`ColumnData`]
//! (`Text`/`I64`/`F64`); ASCII columns are always scalar.

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table::ColumnData;

/// The value type of an ASCII-table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiKind {
    /// `Aw` — character string.
    Char,
    /// `Iw` — decimal integer.
    Integer,
    /// `Fw.d` / `Ew.d` / `Dw.d` — floating point.
    Float,
}

/// One ASCII-table column.
#[derive(Debug, Clone)]
pub struct AsciiColumn {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub kind: AsciiKind,
    /// 0-based byte offset of the field within a row (`TBCOLn − 1`).
    pub start: usize,
    pub width: usize,
    /// Digits after the decimal point (`Fw.d`); 0 for non-floats.
    pub decimals: usize,
}

/// A parsed ASCII table plus its row bytes.
#[derive(Debug, Clone)]
pub struct AsciiTable {
    pub nrows: usize,
    pub columns: Vec<AsciiColumn>,
    row_len: usize,
    bytes: Vec<u8>,
}

impl AsciiTable {
    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<AsciiTable> {
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
        for n in 1..=tfields {
            let tbcol = header
                .get_integer(&format!("TBCOL{n}"))
                .ok_or(FitsError::MissingKeyword { name: "TBCOLn" })?;
            let tform = header
                .get_text(&format!("TFORM{n}"))
                .ok_or(FitsError::MissingKeyword { name: "TFORMn" })?;
            let (kind, width, decimals) = parse_ascii_tform(tform)?;
            columns.push(AsciiColumn {
                name: header
                    .get_text(&format!("TTYPE{n}"))
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(&format!("TUNIT{n}"))
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                kind,
                start: (tbcol.max(1) - 1) as usize,
                width,
                decimals,
            });
        }

        if data.len() < nrows * row_len {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(AsciiTable {
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

    /// Decode column `index` into a typed [`ColumnData`] (`Text`/`I64`/`F64`).
    /// A blank numeric field decodes to 0; a non-blank unparseable field errors.
    pub fn read_column(&self, index: usize) -> Result<ColumnData> {
        let col = self
            .columns
            .get(index)
            .ok_or(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            })?;
        let field = |r: usize| -> &str {
            let row = &self.bytes[r * self.row_len..(r + 1) * self.row_len];
            let end = (col.start + col.width).min(row.len());
            let raw = if col.start < end {
                &row[col.start..end]
            } else {
                &[]
            };
            std::str::from_utf8(raw).unwrap_or("").trim()
        };
        match col.kind {
            AsciiKind::Char => Ok(ColumnData::Text(
                (0..self.nrows).map(field).map(str::to_string).collect(),
            )),
            AsciiKind::Integer => {
                let mut out = Vec::with_capacity(self.nrows);
                for r in 0..self.nrows {
                    let s = field(r);
                    out.push(if s.is_empty() {
                        0
                    } else {
                        s.parse().map_err(|_| FitsError::InvalidValue {
                            card: s.to_string(),
                        })?
                    });
                }
                Ok(ColumnData::I64(out))
            }
            AsciiKind::Float => {
                let mut out = Vec::with_capacity(self.nrows);
                for r in 0..self.nrows {
                    let s = field(r);
                    out.push(if s.is_empty() {
                        0.0
                    } else {
                        // FITS reals allow a Fortran `D` exponent.
                        s.replace(['D', 'd'], "E")
                            .parse()
                            .map_err(|_| FitsError::InvalidValue {
                                card: s.to_string(),
                            })?
                    });
                }
                Ok(ColumnData::F64(out))
            }
        }
    }
}

/// Parse an ASCII `TFORMn` (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`) into kind, width,
/// and decimal count.
fn parse_ascii_tform(value: &str) -> Result<(AsciiKind, usize, usize)> {
    let s = value.trim();
    let invalid = || FitsError::InvalidTform {
        tform: value.to_string(),
    };
    let letter = s.bytes().next().ok_or_else(invalid)?;
    let kind = match letter {
        b'A' => AsciiKind::Char,
        b'I' => AsciiKind::Integer,
        b'F' | b'E' | b'D' => AsciiKind::Float,
        _ => return Err(invalid()),
    };
    let rest = &s[1..];
    let (width, decimals) = match rest.split_once('.') {
        Some((w, d)) => (
            w.trim().parse().map_err(|_| invalid())?,
            d.trim().parse().map_err(|_| invalid())?,
        ),
        None => (rest.trim().parse().map_err(|_| invalid())?, 0),
    };
    Ok((kind, width, decimals))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::FitsReader;
    use crate::writer::AsciiWriteColumn;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

    #[test]
    fn parses_ascii_tform_codes() {
        assert_eq!(parse_ascii_tform("A8").unwrap(), (AsciiKind::Char, 8, 0));
        assert_eq!(
            parse_ascii_tform("I10").unwrap(),
            (AsciiKind::Integer, 10, 0)
        );
        assert_eq!(parse_ascii_tform("F8.2").unwrap(), (AsciiKind::Float, 8, 2));
        assert_eq!(
            parse_ascii_tform("E15.7").unwrap(),
            (AsciiKind::Float, 15, 7)
        );
        assert_eq!(
            parse_ascii_tform("D25.17").unwrap(),
            (AsciiKind::Float, 25, 17)
        );
        assert!(parse_ascii_tform("Z3").is_err());
    }

    #[test]
    fn decodes_hand_built_ascii_rows() {
        // Two columns: name `A4` at col 1, value `I6` at col 5 → row width 10.
        let mut header = Header::new();
        header
            .set("XTENSION", "TABLE")
            .set("BITPIX", 8)
            .set("NAXIS", 2)
            .set("NAXIS1", 10)
            .set("NAXIS2", 2)
            .set("PCOUNT", 0)
            .set("GCOUNT", 1)
            .set("TFIELDS", 2)
            .set("TBCOL1", 1)
            .set("TFORM1", "A4")
            .set("TTYPE1", "NAME")
            .set("TBCOL2", 5)
            .set("TFORM2", "I6")
            .set("TTYPE2", "COUNT");
        let data = b"abc    123def    -45".to_vec(); // "abc " + "   123" ; "def " + "   -45"
        let table = AsciiTable::from_data(&header, data).unwrap();
        assert_eq!(table.nrows, 2);
        assert_eq!(table.columns[1].start, 4);
        assert_eq!(
            table.read_column(0).unwrap(),
            ColumnData::Text(vec!["abc".into(), "def".into()])
        );
        assert_eq!(
            table.read_column(1).unwrap(),
            ColumnData::I64(vec![123, -45])
        );
    }

    #[test]
    fn ascii_table_round_trips_through_write_and_read() {
        let columns = vec![
            AsciiWriteColumn {
                name: "NAME".into(),
                unit: None,
                data: ColumnData::Text(vec!["alpha".into(), "beta".into()]),
                width: 6,
                decimals: 0,
            },
            AsciiWriteColumn {
                name: "N".into(),
                unit: Some("count".into()),
                data: ColumnData::I64(vec![7, -3]),
                width: 5,
                decimals: 0,
            },
            AsciiWriteColumn {
                name: "X".into(),
                unit: None,
                data: ColumnData::F64(vec![1.5, -2.25]),
                width: 8,
                decimals: 2,
            },
        ];
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_ascii_table(2, &columns).unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

        assert_eq!(r.hdus.len(), 2); // auto dataless primary + the TABLE
        assert_eq!(r.hdus[1].kind, crate::HduKind::AsciiTable);
        let t = r.read_ascii_table(1).unwrap();
        assert_eq!(
            t.read_column(0).unwrap(),
            ColumnData::Text(vec!["alpha".into(), "beta".into()])
        );
        assert_eq!(t.read_column(1).unwrap(), ColumnData::I64(vec![7, -3]));
        assert_eq!(t.read_column(2).unwrap(), ColumnData::F64(vec![1.5, -2.25]));
    }
}
