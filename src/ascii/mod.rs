//! ASCII-table extension (§7.2): `TABLE`.
//!
//! Rows are fixed-length lines of ASCII text; each column occupies a fixed byte
//! range starting at `TBCOLn` (1-based), formatted per a Fortran `TFORMn` code
//! (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`). Decoded values reuse [`ColumnData`]
//! (`Text`/`I64`/`F64`); ASCII columns are always scalar.

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
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
    /// `TSCALn` / `TZEROn` for the physical plane (`physical = TZERO + TSCAL·raw`).
    pub tscale: f64,
    pub tzero: f64,
    /// `TNULLn`: the exact field text that marks an undefined value (§7.2.5).
    pub null: Option<String>,
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
        // §7.2.1: `0 ≤ TFIELDS ≤ 999` — also a guard, since `tfields` sizes the
        // column `Vec` and drives the `TFORMn` loop (an absurd value would abort).
        let tfields = match header.get_integer("TFIELDS") {
            Some(t) if (0..=999).contains(&t) => t as usize,
            Some(_) => return Err(FitsError::KeywordOutOfRange { name: "TFIELDS" }),
            None => return Err(FitsError::MissingKeyword { name: "TFIELDS" }),
        };

        let mut columns = Vec::with_capacity(tfields);
        for n in 1..=tfields {
            let tbcol = header
                .get_integer(key!("TBCOL{n}").as_str())
                .ok_or(FitsError::MissingKeyword { name: "TBCOLn" })?;
            let tform = header
                .get_text(key!("TFORM{n}").as_str())
                .ok_or(FitsError::MissingKeyword { name: "TFORMn" })?;
            let fmt = parse_ascii_tform(tform)?;
            let start = (tbcol.max(1) - 1) as usize;
            // §7.2.3: each field must lie within the row (`NAXIS1`). A column declared
            // past the row width is malformed — reject it rather than let `field()`
            // silently truncate to empty.
            if start.checked_add(fmt.width).is_none_or(|end| end > row_len) {
                return Err(FitsError::KeywordOutOfRange { name: "TBCOLn" });
            }
            columns.push(AsciiColumn {
                name: header
                    .get_text(key!("TTYPE{n}").as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(key!("TUNIT{n}").as_str())
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                kind: fmt.kind,
                start,
                width: fmt.width,
                decimals: fmt.decimals,
                tscale: header.get_real(key!("TSCAL{n}").as_str()).unwrap_or(1.0),
                tzero: header.get_real(key!("TZERO{n}").as_str()).unwrap_or(0.0),
                null: header
                    .get_text(key!("TNULL{n}").as_str())
                    .map(|s| s.trim().to_string()),
            });
        }

        // `nrows · row_len` from untrusted axes: check the product can't overflow
        // (a 32-bit-usize hazard `data_extent`'s u64 math wouldn't catch).
        let total = nrows.checked_mul(row_len).ok_or(FitsError::UnexpectedEof)?;
        if data.len() < total {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(AsciiTable {
            nrows,
            columns,
            row_len,
            bytes: data,
        })
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §7.2.2.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.columns.iter().position(|c| {
            c.name
                .as_deref()
                .is_some_and(|n| n.eq_ignore_ascii_case(name))
        })
    }

    /// Decode column `index` into a typed [`ColumnData`] (`Text`/`I64`/`F64`).
    /// A blank numeric field decodes to 0 (§7.2.5); a field equal to `TNULLn`
    /// (undefined) decodes to a 0 placeholder in this raw plane — use
    /// [`AsciiTable::read_column_physical`] to get `NaN` for undefined values.
    /// A non-blank, non-null unparseable field errors.
    pub fn read_column(&self, index: usize) -> Result<ColumnData> {
        let col = self.column(index)?;
        match col.kind {
            AsciiKind::Char => Ok(ColumnData::Text(
                (0..self.nrows)
                    .map(|r| self.field(col, r).to_string())
                    .collect(),
            )),
            AsciiKind::Integer => {
                let mut out = Vec::with_capacity(self.nrows);
                for r in 0..self.nrows {
                    let s = self.field(col, r);
                    out.push(if s.is_empty() || col.is_null(s) {
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
                    let s = self.field(col, r);
                    out.push(if s.is_empty() || col.is_null(s) {
                        0.0
                    } else {
                        parse_ascii_float(s, col.decimals).ok_or_else(|| {
                            FitsError::InvalidValue {
                                card: s.to_string(),
                            }
                        })?
                    });
                }
                Ok(ColumnData::F64(out))
            }
        }
    }

    /// Decode a numeric column into its physical `f64` plane: `TZEROn + TSCALn ×
    /// field` (§7.2.2). A blank field is 0 before scaling; a field equal to
    /// `TNULLn` is undefined and maps to `NaN`. Errors on a character column.
    pub fn read_column_physical(&self, index: usize) -> Result<Vec<f64>> {
        let col = self.column(index)?;
        if col.kind == AsciiKind::Char {
            return Err(FitsError::NonNumericColumn { code: 'A' });
        }
        let mut out = Vec::with_capacity(self.nrows);
        for r in 0..self.nrows {
            let s = self.field(col, r);
            if col.is_null(s) {
                out.push(f64::NAN);
                continue;
            }
            let raw = if s.is_empty() {
                0.0
            } else {
                parse_ascii_float(s, col.decimals).ok_or_else(|| FitsError::InvalidValue {
                    card: s.to_string(),
                })?
            };
            out.push(col.tzero + col.tscale * raw);
        }
        Ok(out)
    }

    fn column(&self, index: usize) -> Result<&AsciiColumn> {
        self.columns
            .get(index)
            .ok_or(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            })
    }

    /// The trimmed text of column `col` in row `r`.
    fn field(&self, col: &AsciiColumn, r: usize) -> &str {
        let row = &self.bytes[r * self.row_len..(r + 1) * self.row_len];
        let end = (col.start + col.width).min(row.len());
        let raw = if col.start < end {
            &row[col.start..end]
        } else {
            &[]
        };
        std::str::from_utf8(raw).unwrap_or("").trim()
    }
}

impl AsciiColumn {
    /// Whether the trimmed field text marks an undefined value (`TNULLn`).
    fn is_null(&self, field: &str) -> bool {
        self.null.as_deref() == Some(field)
    }
}

/// Parse a Fortran `Fw.d`/`Ew.d`/`Dw.d` field. When the mantissa carries no
/// explicit `.`, the decimal point is implied `decimals` digits from the right
/// (§7.2.1, deprecated): the integer mantissa is scaled by `10⁻ᵈ`.
fn parse_ascii_float(field: &str, decimals: usize) -> Option<f64> {
    let normalized = field.replace(['D', 'd'], "E");
    let (mantissa, exponent) = match split_mantissa_exponent(&normalized) {
        Some((m, e)) => (m, Some(e)),
        None => (normalized.as_str(), None),
    };
    let mut value: f64 = if mantissa.contains('.') || decimals == 0 {
        mantissa.parse().ok()?
    } else {
        mantissa.parse::<f64>().ok()? / 10f64.powi(decimals as i32)
    };
    if let Some(e) = exponent {
        value *= 10f64.powi(e.trim().parse::<i32>().ok()?);
    }
    Some(value)
}

/// Split a normalized (`D`→`E`) numeric string into mantissa and exponent text.
/// §7.2.5 rule 3: the exponent is introduced by `E`/`e`, **or** by a bare `+`/`-`
/// sign past the leading mantissa sign (Fortran's letter-less form, e.g.
/// `3.14159-2` = 3.14159 × 10⁻²).
fn split_mantissa_exponent(s: &str) -> Option<(&str, &str)> {
    if let Some(i) = s.find(['E', 'e']) {
        return Some((&s[..i], &s[i + 1..]));
    }
    s.char_indices()
        .find(|&(i, c)| i > 0 && (c == '+' || c == '-'))
        .map(|(i, _)| (&s[..i], &s[i..]))
}

/// A parsed ASCII `TFORMn`: element kind, field width, and decimal count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct AsciiFormat {
    kind: AsciiKind,
    width: usize,
    decimals: usize,
}

/// Parse an ASCII `TFORMn` (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`).
fn parse_ascii_tform(value: &str) -> Result<AsciiFormat> {
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
    Ok(AsciiFormat {
        kind,
        width,
        decimals,
    })
}

#[cfg(test)]
mod tests;
