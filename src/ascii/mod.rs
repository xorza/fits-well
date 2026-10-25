//! ASCII-table extension (§7.2): `TABLE`.
//!
//! Rows are fixed-length lines of ASCII text; each column occupies a fixed byte
//! range starting at `TBCOLn` (1-based), formatted per a Fortran `TFORMn` code
//! (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`). ASCII columns are always scalar, and
//! [`AsciiColumnData`] retains `TNULLn` cells distinctly from genuine values.

pub(crate) mod ascii_text;
pub(crate) mod ascii_tform;
#[cfg(feature = "internals")]
pub(crate) mod bench;

use crate::fits_error::KeywordDefect;
use std::io::{self, Write};
use std::{fmt, mem, result, str};

use crate::ascii::ascii_text::AsciiText;
use crate::ascii::ascii_tform::AsciiTform;
use crate::column;
use crate::column::ColumnSelector;
use crate::column::Named;
use crate::column::{ColumnKeywords, TableShape};
use crate::fits_error::Result;
use crate::fits_error::{ExpectedColumn, FitsError, TextOrigin};
use crate::header_model::{Header, keywords};
use crate::image_model::physical_sample::PhysicalSample;

/// A decoded ASCII-table column in row order. `None` is an undefined field
/// selected by `TNULLn`; blank numeric fields remain genuine zero values (§7.2.5).
#[derive(Debug, Clone, PartialEq)]
pub enum AsciiColumnData {
    /// `Aw` — each field's text, its trailing blanks dropped and its leading ones kept.
    Text(AsciiText),
    /// `Iw` — stored integers before `TSCALn`/`TZEROn`.
    Integer(Vec<Option<i64>>),
    /// `Fw.d` / `Ew.d` / `Dw.d` — stored floating-point values before scaling.
    Float(Vec<Option<f64>>),
}

impl AsciiColumnData {
    /// Number of rows. An ASCII column is always scalar (§7.2), so this is both the
    /// value count and the row count.
    pub const fn len(&self) -> usize {
        match self {
            Self::Text(values) => values.len(),
            Self::Integer(values) => values.len(),
            Self::Float(values) => values.len(),
        }
    }

    pub const fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub const fn as_text(&self) -> Option<&AsciiText> {
        if let Self::Text(values) = self {
            Some(values)
        } else {
            None
        }
    }

    pub const fn as_integer(&self) -> Option<&[Option<i64>]> {
        if let Self::Integer(values) = self {
            Some(values.as_slice())
        } else {
            None
        }
    }

    pub const fn as_float(&self) -> Option<&[Option<f64>]> {
        if let Self::Float(values) = self {
            Some(values.as_slice())
        } else {
            None
        }
    }
}

/// `AsciiColumnData` from its values, and back: the reverse gives the data back when it
/// holds another type.
macro_rules! impl_ascii_column_data_conversions {
    ($variant:ident, $type:ty) => {
        impl From<$type> for AsciiColumnData {
            fn from(values: $type) -> Self {
                Self::$variant(values)
            }
        }

        impl TryFrom<AsciiColumnData> for $type {
            type Error = AsciiColumnData;

            fn try_from(data: AsciiColumnData) -> result::Result<Self, AsciiColumnData> {
                match data {
                    AsciiColumnData::$variant(values) => Ok(values),
                    other => Err(other),
                }
            }
        }
    };
}

impl_ascii_column_data_conversions!(Text, AsciiText);
impl_ascii_column_data_conversions!(Integer, Vec<Option<i64>>);
impl_ascii_column_data_conversions!(Float, Vec<Option<f64>>);

/// One ASCII-table column.
#[derive(Debug, Clone)]
pub struct AsciiColumn {
    pub tform: AsciiTform,
    pub(crate) byte_offset: usize,
    keywords: ColumnKeywords<String>,
}

/// A parsed ASCII table plus its row text.
#[derive(Debug, Clone)]
pub struct Ascii {
    schema: AsciiSchema,
    /// The `row_count × row_len` field region, validated as ASCII on parse. Holding it as
    /// a `str` makes a field read an infallible slice at a known char boundary.
    rows: String,
}

/// An ASCII table's structure: its row count, row width and validated columns, the
/// counterpart of a binary table's [`Schema`](crate::table::Schema).
#[derive(Debug, Clone)]
pub struct AsciiSchema {
    pub(crate) row_count: usize,
    pub(crate) row_len: usize,
    /// Validated column descriptors in `TFIELDS` order.
    pub columns: Vec<AsciiColumn>,
}

impl AsciiSchema {
    /// Parse the complete ASCII-table schema from its header, without its data unit.
    pub fn parse(header: &Header) -> Result<Self> {
        let TableShape {
            row_len,
            row_count,
            field_count: tfields,
        } = TableShape::read(header)?;
        // §7.2.1: an ASCII table has no heap.
        if header.require_integer(keywords::PCOUNT)? != 0 {
            return Err(FitsError::InvalidKeywordValue {
                keyword: keywords::PCOUNT,
                defect: KeywordDefect::Structure,
            });
        }

        let mut columns = Vec::with_capacity(tfields);
        for n in 1..=tfields {
            let tbcol_keyword = keywords::TBCOL.indexed(n)?;
            let tbcol = header.require_usize(tbcol_keyword)?;
            let tform = header.require_text(keywords::TFORM.indexed(n)?)?;
            let tform = tform.parse::<AsciiTform>()?;
            let start = tbcol.checked_sub(1).ok_or(FitsError::InvalidKeywordValue {
                keyword: tbcol_keyword,
                defect: KeywordDefect::Range,
            })?;
            // §7.2.3: each field must lie within the row (`NAXIS1`). A column declared
            // past the row width is malformed — reject it rather than let `field()`
            // silently truncate to empty.
            if start
                .checked_add(tform.width())
                .is_none_or(|end| end > row_len)
            {
                return Err(FitsError::InvalidKeywordValue {
                    keyword: tbcol_keyword,
                    defect: KeywordDefect::Structure,
                });
            }
            columns.push(AsciiColumn {
                tform,
                byte_offset: start,
                keywords: ColumnKeywords::read(header, n, |tnull| {
                    Ok(header
                        .get_text(tnull)?
                        .map(|tnull| tnull.trim().to_string()))
                })?,
            });
        }
        Ok(Self {
            row_count,
            row_len,
            columns,
        })
    }

    pub const fn row_count(&self) -> usize {
        self.row_count
    }

    /// Byte width of one row (`NAXIS1`).
    pub const fn row_len(&self) -> usize {
        self.row_len
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §7.2.2.
    pub fn column_index(&self, name: &str) -> Result<Option<usize>> {
        column::index_of(&self.columns, name)
    }
}

impl Ascii {
    /// The table's structure: its row count, row width and validated columns.
    pub const fn schema(&self) -> &AsciiSchema {
        &self.schema
    }

    /// The table `schema` describes, over its data unit `data`. Errors when the data
    /// ends before the rows do, and when the rows are not ASCII text.
    pub(crate) fn new(schema: AsciiSchema, data: Vec<u8>) -> Result<Self> {
        // `row_count · row_len` from untrusted axes: check the product can't overflow
        // (a 32-bit-usize hazard `HduKind::data_bytes`' u64 math wouldn't catch).
        let total = schema
            .row_count
            .checked_mul(schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
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
            return Err(FitsError::InvalidAscii {
                text: TextOrigin::TableData,
            });
        }
        Ok(Self {
            schema,
            rows: String::from_utf8(data).expect("ASCII bytes are valid UTF-8"),
        })
    }

    /// A reader handle for the column `column` selects, by index or `TTYPEn` name. Decode
    /// through it — [`AsciiColumnReader::raw_into`]/[`physical_into`](AsciiColumnReader::physical_into) —
    /// without re-passing the descriptor. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last column, and with
    /// [`FitsError::ColumnNotFound`] for a name no column carries.
    pub fn column<'n>(
        &self,
        column: impl Into<ColumnSelector<'n>>,
    ) -> Result<AsciiColumnReader<'_>> {
        let index = column.into().resolve(&self.schema.columns)?;
        Ok(AsciiColumnReader { table: self, index })
    }

    /// The complete fixed-width text of column `col` in row `r`. Infallible: the field
    /// region was validated as ASCII on parse, so every `TBCOLn` offset is a character
    /// boundary. `from_data` rejected the non-ASCII bytes that could otherwise
    /// masquerade as a blank field and silently decode to 0 in a numeric column.
    fn field(&self, col: &AsciiColumn, r: usize) -> &str {
        let start = r * self.schema.row_len + col.byte_offset;
        &self.rows[start..start + col.tform.width()]
    }
}

/// A handle to one column of an [`Ascii`], from [`Ascii::column`]. Decode through it without re-passing the
/// descriptor: [`raw_into`](Self::raw_into) for the typed values, [`physical_into`](Self::physical_into)
/// for the scaled plane. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub struct AsciiColumnReader<'a> {
    table: &'a Ascii,
    index: usize,
}

impl<'a> AsciiColumnReader<'a> {
    /// The column's [`AsciiColumn`] descriptor.
    pub fn descriptor(&self) -> &'a AsciiColumn {
        &self.table.schema.columns[self.index]
    }

    /// Decode the stored fields into `out` as typed [`AsciiColumnData`], whose buffers it
    /// reuses when `out` holds this column's kind. A blank numeric field decodes to
    /// `Some(0)` (§7.2.5), while a field equal to `TNULLn` decodes to `None`. A non-blank,
    /// non-null unparseable field errors, and leaves `out` an empty column of this kind.
    pub fn raw_into(&self, out: &mut AsciiColumnData) -> Result<()> {
        let table = self.table;
        let col = self.descriptor();
        let tnull = col.tnull()?;
        let rows = 0..table.schema.row_count;
        match col.tform {
            AsciiTform::Text { .. } => {
                let mut text = match mem::replace(out, AsciiColumnData::Text(AsciiText::default()))
                {
                    AsciiColumnData::Text(text) => text,
                    _ => AsciiText::default(),
                };
                text.refill(rows.map(|r| {
                    let field = table.field(col, r);
                    (tnull != Some(field.trim())).then_some(field)
                }));
                *out = AsciiColumnData::Text(text);
            }
            AsciiTform::Integer { .. } => {
                let mut values = match mem::replace(out, AsciiColumnData::Integer(Vec::new())) {
                    AsciiColumnData::Integer(values) => values,
                    _ => Vec::new(),
                };
                values.clear();
                values.reserve(rows.len());
                for r in rows {
                    values.push(
                        table
                            .defined_field(col, tnull, r)
                            .map(|field| {
                                parse_integer(field).ok_or_else(|| invalid(self.index, r, field))
                            })
                            .transpose()?,
                    );
                }
                *out = AsciiColumnData::Integer(values);
            }
            AsciiTform::Float(format) => {
                let mut values = match mem::replace(out, AsciiColumnData::Float(Vec::new())) {
                    AsciiColumnData::Float(values) => values,
                    _ => Vec::new(),
                };
                values.clear();
                values.reserve(rows.len());
                for r in rows {
                    let value = table
                        .defined_field(col, tnull, r)
                        .map(|field| {
                            parse_float(field, format.decimals())
                                .ok_or_else(|| invalid(self.index, r, field))
                        })
                        .transpose()?;
                    values.push(value);
                }
                *out = AsciiColumnData::Float(values);
            }
        }
        Ok(())
    }

    /// Append the numeric column on its physical plane to `out`: `TZEROn + TSCALn ×
    /// field` (§7.2.2). A blank field is 0 before scaling; a field equal to `TNULLn` is
    /// undefined and maps to `NaN`. The arithmetic runs in `f64`, and an `f32` buffer
    /// narrows its result. Errors on a character column.
    pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>) -> Result<()> {
        let col = self.descriptor();
        if matches!(col.tform, AsciiTform::Text { .. }) {
            return Err(FitsError::ColumnKindMismatch {
                column: self.index,
                expected: ExpectedColumn::Numeric,
            });
        }
        let scaling = FieldScaling::of(col)?;
        out.reserve(self.table.schema.row_count);
        for row in 0..self.table.schema.row_count {
            out.push(T::from_f64(
                self.table.physical_value(self.index, scaling, row)?,
            ));
        }
        Ok(())
    }
}

/// A numeric column's scaling and null marker, read once for all its fields.
#[derive(Debug, Clone, Copy)]
struct FieldScaling<'a> {
    tscale: f64,
    tzero: f64,
    tnull: Option<&'a str>,
}

impl<'a> FieldScaling<'a> {
    fn of(col: &'a AsciiColumn) -> Result<Self> {
        Ok(Self {
            tscale: col.tscale()?,
            tzero: col.tzero()?,
            tnull: col.tnull()?,
        })
    }
}

impl Ascii {
    /// The physical value of the field of the numeric column `column` in `row`, as
    /// [`AsciiColumnReader::physical_into`] gives it.
    fn physical_value(&self, column: usize, scaling: FieldScaling<'_>, row: usize) -> Result<f64> {
        let FieldScaling {
            tscale,
            tzero,
            tnull,
        } = scaling;
        let col = &self.schema.columns[column];
        let Some(field) = self.defined_field(col, tnull, row) else {
            // §7.2.5: an undefined field has no value to scale.
            return Ok(f64::NAN);
        };
        let value = match col.tform {
            AsciiTform::Integer { .. } => parse_integer(field).map(|value| value as f64),
            AsciiTform::Float(format) => parse_float(field, format.decimals()),
            AsciiTform::Text { .. } => unreachable!("a numeric column's value"),
        };
        let value = value.ok_or_else(|| invalid(column, row, field))?;
        Ok(tscale.mul_add(value, tzero))
    }

    /// The trimmed text of the field of `col` in `row`, or `None` when it is the column's
    /// `TNULLn` marker `tnull` and so carries no value at all (§7.2.5).
    fn defined_field(&self, col: &AsciiColumn, tnull: Option<&str>, row: usize) -> Option<&str> {
        let field = self.field(col, row).trim();
        (tnull != Some(field)).then_some(field)
    }
}

/// Parse an `Iw` field. A blank one is a genuine zero, not an undefined value
/// (§7.2.5) — only `TNULLn` marks undefined, and [`Ascii::defined_field`] filtered it.
fn parse_integer(field: &str) -> Option<i64> {
    if field.is_empty() {
        return Some(0);
    }
    field.parse().ok()
}

/// The refusal of `field`, of `column` in `row`, which holds no number of its format.
fn invalid(column: usize, row: usize, field: &str) -> FitsError {
    FitsError::InvalidAsciiField {
        column,
        row,
        field: field.to_string(),
    }
}

/// Parse an `Fw.d`/`Ew.d`/`Dw.d` field, with the same blank rule as [`parse_integer`].
fn parse_float(field: &str, decimals: usize) -> Option<f64> {
    if field.is_empty() {
        return Some(0.0);
    }
    parse_ascii_float(field, decimals)
}

impl AsciiColumn {
    /// Byte offset of this column from the start of a row (`TBCOLn − 1`).
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    /// The `TTYPEn` name, `None` when absent or empty.
    pub fn name(&self) -> Result<Option<&str>> {
        self.keywords.name()
    }

    /// The `TUNITn` unit, `None` when absent or empty.
    pub fn unit(&self) -> Result<Option<&str>> {
        self.keywords.unit()
    }

    /// `TSCALn`, 1 when absent: `physical = TZERO + TSCAL × field`.
    pub fn tscale(&self) -> Result<f64> {
        self.keywords.tscale()
    }

    /// `TZEROn`, 0 when absent.
    pub fn tzero(&self) -> Result<f64> {
        self.keywords.tzero()
    }

    /// `TNULLn`, trimmed: the field text that marks an undefined value (§7.2.5).
    pub fn tnull(&self) -> Result<Option<&str>> {
        Ok(self.keywords.tnull()?.map(String::as_str))
    }

    /// The raw `TDISPn` display recommendation (§7.2.2), if declared.
    pub fn tdisp(&self) -> Result<Option<&str>> {
        self.keywords.tdisp()
    }
}

impl Named for AsciiColumn {
    fn name(&self) -> Result<Option<&str>> {
        self.keywords.name()
    }
}

/// Parse a Fortran `Fw.d`/`Ew.d`/`Dw.d` field. A field whose mantissa has no decimal
/// point takes the implied one, `decimals` digits from the right, with or without an
/// exponent (§7.2.5; design.md, "An ASCII real field with no point takes its implied
/// point"), as cfitsio's `fffstrr8` reads it: `1E5` in `E10.2` is 1000.
///
/// Mantissa and exponent are joined into one decimal literal and parsed once, so the
/// value is the f64 nearest the decimal the field spells — what strtod gives cfitsio —
/// rather than a parsed mantissa times a rounded power of ten.
fn parse_ascii_float(field: &str, decimals: usize) -> Option<f64> {
    let Scientific { mantissa, exponent } = split_mantissa_exponent(field).unwrap_or(Scientific {
        mantissa: field,
        exponent: "",
    });
    let implied = if mantissa.contains('.') {
        0
    } else {
        i64::try_from(decimals).ok()?
    };
    let exponent = match exponent.trim() {
        "" => 0,
        exponent => exponent.parse::<i64>().ok()?,
    };
    let value = parse_literal(format_args!(
        "{mantissa}e{}",
        exponent.checked_sub(implied)?
    ));
    // §7.2.5 numbers are finite: `nan`, `inf` and an overflowing exponent spell none.
    value.filter(|value: &f64| value.is_finite())
}

/// Parses a formatted decimal literal, formatting into a stack buffer when it fits, so a
/// cell costs no allocation.
fn parse_literal(literal: fmt::Arguments<'_>) -> Option<f64> {
    let mut stack = [0; 96];
    let mut cursor = io::Cursor::new(&mut stack[..]);
    if cursor.write_fmt(literal).is_ok() {
        let len = usize::try_from(cursor.position()).expect("at most the buffer length");
        return str::from_utf8(&stack[..len])
            .expect("`write_fmt` writes whole `str`s")
            .parse()
            .ok();
    }
    fmt::format(literal).parse().ok()
}

/// Split a numeric string into mantissa and exponent text. The exponent is
/// introduced by `E`/`e` or the Fortran double-precision `D`/`d` (§7.2.1), **or** by
/// a bare `+`/`-` sign past the leading mantissa sign (the letter-less form, §7.2.5
/// rule 3, e.g. `3.14159-2` = 3.14159 × 10⁻²). Matching `D`/`d` here means the parse
/// never has to normalize the field into a fresh `String` first.
fn split_mantissa_exponent(s: &str) -> Option<Scientific<'_>> {
    if let Some(i) = s.find(['E', 'e', 'D', 'd']) {
        return Some(Scientific {
            mantissa: &s[..i],
            exponent: &s[i + 1..],
        });
    }
    s.char_indices()
        .find(|&(i, c)| i > 0 && (c == '+' || c == '-'))
        .map(|(i, _)| Scientific {
            mantissa: &s[..i],
            exponent: &s[i..],
        })
}

/// A number's text split at its exponent: the mantissa, and the exponent with its sign.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Scientific<'a> {
    mantissa: &'a str,
    exponent: &'a str,
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::ascii::{Ascii, AsciiSchema};
    use crate::fits_error::Result;
    use crate::header_model::Header;
    use crate::header_model::keywords;
    use crate::internals::cards;

    impl Ascii {
        /// The table `header` describes, over its data unit `data`.
        pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<Self> {
            Self::new(AsciiSchema::parse(header)?, data)
        }
    }

    /// A minimal ASCII `TABLE` header: rows of `naxis1` characters, and each column
    /// at its 1-based `TBCOLn` with its `TFORMn`.
    pub(crate) fn ascii_table_header(
        naxis1: usize,
        naxis2: usize,
        columns: &[(usize, &str)],
    ) -> Header {
        let axes = [naxis1, naxis2].map(|length| i64::try_from(length).unwrap());
        let mut header = cards::structural(Some("TABLE"), 8, &axes);
        header.set_internal(keywords::TFIELDS, i64::try_from(columns.len()).unwrap());
        for (index, &(tbcol, tform)) in columns.iter().enumerate() {
            header
                .set_internal(format!("TBCOL{}", index + 1), i64::try_from(tbcol).unwrap())
                .set_internal(format!("TFORM{}", index + 1), tform);
        }
        header
    }
}

#[cfg(test)]
mod tests;
