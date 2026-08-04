//! ASCII-table extension (§7.2): `TABLE`.
//!
//! Rows are fixed-length lines of ASCII text; each column occupies a fixed byte
//! range starting at `TBCOLn` (1-based), formatted per a Fortran `TFORMn` code
//! (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`). ASCII columns are always scalar, and
//! [`AsciiColumnData`] retains `TNULLn` cells distinctly from genuine values.

use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::keyword::key;

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

/// A decoded ASCII-table column in row order. `None` is an undefined field
/// selected by `TNULLn`; blank numeric fields remain genuine zero values (§7.2.5).
#[derive(Debug, Clone, PartialEq)]
pub enum AsciiColumnData {
    /// `Aw` — the complete fixed-width field text, including padding.
    Text(Vec<Option<String>>),
    /// `Iw` — stored integers before `TSCALn`/`TZEROn`.
    Integer(Vec<Option<i64>>),
    /// `Fw.d` / `Ew.d` / `Dw.d` — stored floating-point values before scaling.
    Float(Vec<Option<f64>>),
}

impl AsciiColumnData {
    /// Number of rows. An ASCII column is always scalar (§7.2), so this is both the
    /// value count and the row count.
    pub fn len(&self) -> usize {
        match self {
            AsciiColumnData::Text(values) => values.len(),
            AsciiColumnData::Integer(values) => values.len(),
            AsciiColumnData::Float(values) => values.len(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Whether any row is undefined, and so needs a `TNULLn` marker to be writable.
    pub(crate) fn has_null(&self) -> bool {
        match self {
            AsciiColumnData::Text(values) => values.iter().any(Option::is_none),
            AsciiColumnData::Integer(values) => values.iter().any(Option::is_none),
            AsciiColumnData::Float(values) => values.iter().any(Option::is_none),
        }
    }
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

/// A parsed ASCII table plus its row text.
#[derive(Debug, Clone)]
pub struct AsciiTable {
    nrows: usize,
    columns: Vec<AsciiColumn>,
    row_len: usize,
    /// The `nrows × row_len` field region, validated as ASCII on parse. Holding it as
    /// a `str` makes a field read an infallible slice at a known char boundary.
    rows: String,
}

/// Immutable row and column metadata for a parsed ASCII table.
#[derive(Debug, Clone, Copy)]
pub struct AsciiTableMetadata<'a> {
    /// Number of rows in the table.
    pub nrows: usize,
    /// Validated column descriptors in `TFIELDS` order.
    pub columns: &'a [AsciiColumn],
}

impl AsciiTable {
    /// Borrow the table's validated row count and column descriptors.
    pub fn metadata(&self) -> AsciiTableMetadata<'_> {
        AsciiTableMetadata {
            nrows: self.nrows,
            columns: &self.columns,
        }
    }

    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<AsciiTable> {
        let row_len = header.required_usize("NAXIS1", "NAXIS1")?;
        let nrows = header.required_usize("NAXIS2", "NAXIS2")?;
        // §7.2.1: `0 ≤ TFIELDS ≤ 999` — also a guard, since `tfields` sizes the
        // column `Vec` and drives the `TFORMn` loop (an absurd value would abort).
        let tfields = header.required_usize("TFIELDS", "TFIELDS")?;
        validate_table_field_count(tfields)?;

        let mut columns = Vec::with_capacity(tfields);
        for n in 1..=tfields {
            let tbcol = header.required_usize(key!("TBCOL{n}").as_str(), "TBCOLn")?;
            let tform = header.required_text(key!("TFORM{n}").as_str(), "TFORMn")?;
            let fmt = parse_ascii_tform(tform)?;
            let start = tbcol
                .checked_sub(1)
                .ok_or(FitsError::KeywordOutOfRange { name: "TBCOLn" })?;
            // §7.2.3: each field must lie within the row (`NAXIS1`). A column declared
            // past the row width is malformed — reject it rather than let `field()`
            // silently truncate to empty.
            if start.checked_add(fmt.width).is_none_or(|end| end > row_len) {
                return Err(FitsError::KeywordOutOfRange { name: "TBCOLn" });
            }
            columns.push(AsciiColumn {
                name: header
                    .get_text(key!("TTYPE{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(key!("TUNIT{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                kind: fmt.kind,
                start,
                width: fmt.width,
                decimals: fmt.decimals,
                tscale: header.get_real(key!("TSCAL{n}").as_str())?.unwrap_or(1.0),
                tzero: header.get_real(key!("TZERO{n}").as_str())?.unwrap_or(0.0),
                null: header
                    .get_text(key!("TNULL{n}").as_str())?
                    .map(|s| s.trim().to_string()),
            });
        }

        // `nrows · row_len` from untrusted axes: check the product can't overflow
        // (a 32-bit-usize hazard `data_extent`'s u64 math wouldn't catch).
        let total = nrows.checked_mul(row_len).ok_or(FitsError::UnexpectedEof)?;
        if data.len() < total {
            return Err(FitsError::UnexpectedEof);
        }
        // §7.2: the data unit is ASCII text. Checking the whole field region once —
        // as `Card::parse` does for a header record — means every later field read is
        // a plain slice: no per-cell UTF-8 validation, and `TBCOLn` byte offsets are
        // guaranteed to land on character boundaries. It also stops a corrupt byte in
        // one column from being discovered only when that column happens to be read.
        let mut data = data;
        data.truncate(total);
        if !data.is_ascii() {
            return Err(FitsError::InvalidValue {
                card: "non-ASCII bytes in ASCII-table data".to_string(),
            });
        }
        Ok(AsciiTable {
            nrows,
            columns,
            row_len,
            rows: String::from_utf8(data).expect("ASCII bytes are valid UTF-8"),
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

    fn column_index_checked(&self, name: &str) -> Result<usize> {
        self.column_index(name)
            .ok_or_else(|| FitsError::ColumnNotFound {
                name: name.to_string(),
            })
    }

    /// A reader handle for the column at `index`. Decode through it —
    /// [`AsciiColumnReader::raw`]/[`physical`](AsciiColumnReader::physical) — without
    /// re-passing the descriptor. Errors with [`FitsError::ColumnIndexOutOfBounds`].
    pub fn column_by_idx(&self, index: usize) -> Result<AsciiColumnReader<'_>> {
        if index >= self.columns.len() {
            return Err(FitsError::ColumnIndexOutOfBounds {
                index,
                len: self.columns.len(),
            });
        }
        Ok(AsciiColumnReader { table: self, index })
    }

    /// A reader handle for the column named `name` (`TTYPEn`, case-insensitive, §7.2.2).
    /// Errors with [`FitsError::ColumnNotFound`] if no such column exists.
    pub fn column_by_name(&self, name: &str) -> Result<AsciiColumnReader<'_>> {
        let index = self.column_index_checked(name)?;
        Ok(AsciiColumnReader { table: self, index })
    }

    /// The complete fixed-width text of column `col` in row `r`. Infallible: the field
    /// region was validated as ASCII on parse, so every `TBCOLn` offset is a character
    /// boundary. `from_data` rejected the non-ASCII bytes that could otherwise
    /// masquerade as a blank field and silently decode to 0 in a numeric column.
    fn field(&self, col: &AsciiColumn, r: usize) -> &str {
        let row = &self.rows[r * self.row_len..(r + 1) * self.row_len];
        let end = (col.start + col.width).min(row.len());
        if col.start < end {
            &row[col.start..end]
        } else {
            ""
        }
    }
}

/// A handle to one column of an [`AsciiTable`], from [`AsciiTable::column_by_idx`] or
/// [`AsciiTable::column_by_name`]. Decode through it without re-passing the
/// descriptor: [`raw`](Self::raw) for the typed values, [`physical`](Self::physical)
/// for the scaled plane. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub struct AsciiColumnReader<'a> {
    table: &'a AsciiTable,
    index: usize,
}

impl<'a> AsciiColumnReader<'a> {
    /// The column's [`AsciiColumn`] descriptor.
    pub fn descriptor(&self) -> &'a AsciiColumn {
        &self.table.columns[self.index]
    }

    /// Decode the stored fields into typed [`AsciiColumnData`]. A blank numeric
    /// field decodes to `Some(0)` (§7.2.5), while a field equal to `TNULLn` decodes
    /// to `None`. A non-blank, non-null unparseable field errors.
    pub fn raw(&self) -> Result<AsciiColumnData> {
        let table = self.table;
        let col = self.descriptor();
        match col.kind {
            AsciiKind::Char => Ok(AsciiColumnData::Text(
                (0..table.nrows)
                    .map(|r| {
                        let field = table.field(col, r);
                        (!col.is_null(field.trim())).then(|| field.to_string())
                    })
                    .collect(),
            )),
            AsciiKind::Integer => {
                let mut out = Vec::with_capacity(table.nrows);
                for r in 0..table.nrows {
                    out.push(match parse_numeric_field(table, col, r)? {
                        None => None,
                        Some(ParsedNumeric::Integer(value)) => Some(value),
                        Some(ParsedNumeric::Float(_)) => {
                            unreachable!("integer columns parse integer fields")
                        }
                    });
                }
                Ok(AsciiColumnData::Integer(out))
            }
            AsciiKind::Float => {
                let mut out = Vec::with_capacity(table.nrows);
                for r in 0..table.nrows {
                    out.push(match parse_numeric_field(table, col, r)? {
                        None => None,
                        Some(ParsedNumeric::Float(value)) => Some(value),
                        Some(ParsedNumeric::Integer(_)) => {
                            unreachable!("float columns parse float fields")
                        }
                    });
                }
                Ok(AsciiColumnData::Float(out))
            }
        }
    }

    /// The numeric column on its physical `f64` plane: `TZEROn + TSCALn × field`
    /// (§7.2.2). A blank field is 0 before scaling; a field equal to `TNULLn` is
    /// undefined and maps to `NaN`. Errors on a character column.
    pub fn physical(&self) -> Result<Vec<f64>> {
        let col = self.descriptor();
        if col.kind == AsciiKind::Char {
            return Err(FitsError::NonNumericColumn { code: 'A' });
        }
        let physical = |value| col.tzero + col.tscale * value;
        let mut out = Vec::with_capacity(self.table.nrows);
        for row in 0..self.table.nrows {
            let value = match parse_numeric_field(self.table, col, row)? {
                None => f64::NAN,
                Some(ParsedNumeric::Integer(value)) => physical(value as f64),
                Some(ParsedNumeric::Float(value)) => physical(value),
            };
            out.push(value);
        }
        Ok(out)
    }
}

#[derive(Debug, Clone, Copy)]
enum ParsedNumeric {
    Integer(i64),
    Float(f64),
}

fn parse_numeric_field(
    table: &AsciiTable,
    col: &AsciiColumn,
    row: usize,
) -> Result<Option<ParsedNumeric>> {
    let field = table.field(col, row).trim();
    if col.is_null(field) {
        return Ok(None);
    }
    Ok(Some(match col.kind {
        AsciiKind::Integer if field.is_empty() => ParsedNumeric::Integer(0),
        AsciiKind::Integer => {
            ParsedNumeric::Integer(field.parse().map_err(|_| FitsError::InvalidValue {
                card: field.to_string(),
            })?)
        }
        AsciiKind::Float if field.is_empty() => ParsedNumeric::Float(0.0),
        AsciiKind::Float => {
            ParsedNumeric::Float(parse_ascii_float(field, col.decimals).ok_or_else(|| {
                FitsError::InvalidValue {
                    card: field.to_string(),
                }
            })?)
        }
        AsciiKind::Char => unreachable!("numeric field parser rejects character columns"),
    }))
}

impl AsciiColumn {
    /// Whether the trimmed field text marks an undefined value (`TNULLn`).
    fn is_null(&self, field: &str) -> bool {
        self.null.as_deref() == Some(field)
    }
}

/// Parse a Fortran `Fw.d`/`Ew.d`/`Dw.d` field. When a point-less `Fw.d` field has
/// no exponent, the decimal point is implied `decimals` digits from the right
/// (§7.2.1, deprecated): the integer mantissa is scaled by `10⁻ᵈ`.
fn parse_ascii_float(field: &str, decimals: usize) -> Option<f64> {
    let (mantissa, exponent) = match split_mantissa_exponent(field) {
        Some((m, e)) => (m, Some(e)),
        None => (field, None),
    };
    // The §7.2.1 implied decimal point is an `Fw.d` legacy. cfitsio/astropy apply it
    // only to a bare mantissa and parse an explicit-exponent field literally (strtod),
    // so `1E5` is 100000, not `1·10⁻ᵈ·10⁵` — match them, since the whole point is to
    // read the files those tools write.
    let implied = exponent.is_none() && decimals != 0 && !mantissa.contains('.');
    let mut value: f64 = if implied {
        mantissa.parse::<f64>().ok()? / 10f64.powi(decimals as i32)
    } else {
        mantissa.parse().ok()?
    };
    if let Some(e) = exponent {
        value *= 10f64.powi(e.trim().parse::<i32>().ok()?);
    }
    Some(value)
}

/// Split a numeric string into mantissa and exponent text. The exponent is
/// introduced by `E`/`e` or the Fortran double-precision `D`/`d` (§7.2.1), **or** by
/// a bare `+`/`-` sign past the leading mantissa sign (the letter-less form, §7.2.5
/// rule 3, e.g. `3.14159-2` = 3.14159 × 10⁻²). Matching `D`/`d` here means the parse
/// never has to normalize the field into a fresh `String` first.
fn split_mantissa_exponent(s: &str) -> Option<(&str, &str)> {
    if let Some(i) = s.find(['E', 'e', 'D', 'd']) {
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
