//! ASCII-table extension (§7.2): `TABLE`.
//!
//! Rows are fixed-length lines of ASCII text; each column occupies a fixed byte
//! range starting at `TBCOLn` (1-based), formatted per a Fortran `TFORMn` code
//! (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`). ASCII columns are always scalar, and
//! [`AsciiColumnData`] retains `TNULLn` cells distinctly from genuine values.

pub(crate) mod ascii_text;

use std::io::{self, Write};
use std::{fmt, result, str};

use crate::ascii::ascii_text::AsciiText;
use crate::column;
use crate::column::ColumnSelector;
use crate::column::Named;
use crate::column::{ColumnLabels, ColumnScaling, Keyed, TableShape};
use crate::error::Result;
use crate::error::{ExpectedColumn, FitsError, TextOrigin};
use crate::header_model::{Header, keywords};

/// The value type of an ASCII-table column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiKind {
    /// `Aw` — character string.
    Text,
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
    pub name: Option<String>,
    pub unit: Option<String>,
    pub kind: AsciiKind,
    pub(crate) byte_offset: usize,
    pub width: usize,
    /// Digits after the decimal point (`Fw.d`); 0 for non-floats.
    pub decimals: usize,
    tscale: Keyed<f64>,
    tzero: Keyed<f64>,
    tnull: Keyed<Option<String>>,
    tdisp: Keyed<Option<String>>,
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
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::PCOUNT,
            });
        }

        let mut columns = Vec::with_capacity(tfields);
        for n in 1..=tfields {
            let tbcol_keyword = keywords::TBCOL.indexed(n)?;
            let tbcol = header.require_usize(tbcol_keyword)?;
            let tform = header.require_text(keywords::TFORM.indexed(n)?)?;
            let fmt = parse_ascii_tform(tform)?;
            let start = tbcol.checked_sub(1).ok_or(FitsError::KeywordOutOfRange {
                keyword: tbcol_keyword,
            })?;
            // §7.2.3: each field must lie within the row (`NAXIS1`). A column declared
            // past the row width is malformed — reject it rather than let `field()`
            // silently truncate to empty.
            if start.checked_add(fmt.width).is_none_or(|end| end > row_len) {
                return Err(FitsError::KeywordOutOfRange {
                    keyword: tbcol_keyword,
                });
            }
            let labels = ColumnLabels::read(header, n)?;
            let scaling = ColumnScaling::read(header, n)?;
            let tnull_keyword = keywords::TNULL.indexed(n)?;
            let tdisp_keyword = keywords::TDISP.indexed(n)?;
            columns.push(AsciiColumn {
                name: labels.name,
                unit: labels.unit,
                kind: fmt.kind,
                byte_offset: start,
                width: fmt.width,
                decimals: fmt.decimals,
                tscale: scaling.tscale,
                tzero: scaling.tzero,
                tnull: column::keyed(tnull_keyword, || {
                    Ok(header
                        .get_text(tnull_keyword)?
                        .map(|tnull| tnull.trim().to_string()))
                }),
                tdisp: column::keyed(tdisp_keyword, || {
                    Ok(header.get_text(tdisp_keyword)?.map(str::to_string))
                }),
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
    pub fn column_index(&self, name: &str) -> Option<usize> {
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
        // (a 32-bit-usize hazard `data_extent`'s u64 math wouldn't catch).
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
    /// through it — [`AsciiColumnReader::raw`]/[`physical`](AsciiColumnReader::physical) —
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
        &self.rows[start..start + col.width]
    }
}

/// A handle to one column of an [`Ascii`], from [`Ascii::column`]. Decode through it without re-passing the
/// descriptor: [`raw`](Self::raw) for the typed values, [`physical`](Self::physical)
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

    /// Decode the stored fields into typed [`AsciiColumnData`]. A blank numeric
    /// field decodes to `Some(0)` (§7.2.5), while a field equal to `TNULLn` decodes
    /// to `None`. A non-blank, non-null unparseable field errors.
    pub fn raw(&self) -> Result<AsciiColumnData> {
        let table = self.table;
        let col = self.descriptor();
        let tnull = col.tnull()?;
        match col.kind {
            AsciiKind::Text => Ok(AsciiColumnData::Text(
                (0..table.schema.row_count)
                    .map(|r| {
                        let field = table.field(col, r);
                        (tnull != Some(field.trim())).then_some(field)
                    })
                    .collect(),
            )),
            AsciiKind::Integer => {
                let mut out = Vec::with_capacity(table.schema.row_count);
                for r in 0..table.schema.row_count {
                    out.push(
                        table
                            .defined_field(col, tnull, r)
                            .map(parse_integer)
                            .transpose()?,
                    );
                }
                Ok(AsciiColumnData::Integer(out))
            }
            AsciiKind::Float => {
                let mut out = Vec::with_capacity(table.schema.row_count);
                for r in 0..table.schema.row_count {
                    let value = table
                        .defined_field(col, tnull, r)
                        .map(|field| parse_float(field, col.decimals))
                        .transpose()?;
                    out.push(value);
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
        if col.kind == AsciiKind::Text {
            return Err(FitsError::ColumnKindMismatch {
                column: self.index,
                expected: ExpectedColumn::Numeric,
            });
        }
        let (tscale, tzero, tnull) = (col.tscale()?, col.tzero()?, col.tnull()?);
        let mut out = Vec::with_capacity(self.table.schema.row_count);
        for row in 0..self.table.schema.row_count {
            let value = match self.table.defined_field(col, tnull, row) {
                // §7.2.5: an undefined field has no value to scale.
                None => f64::NAN,
                Some(field) if col.kind == AsciiKind::Integer => {
                    tzero + tscale * parse_integer(field)? as f64
                }
                Some(field) => tzero + tscale * parse_float(field, col.decimals)?,
            };
            out.push(value);
        }
        Ok(out)
    }
}

impl Ascii {
    /// The trimmed text of the field of `col` in `row`, or `None` when it is the column's
    /// `TNULLn` marker `tnull` and so carries no value at all (§7.2.5).
    fn defined_field(&self, col: &AsciiColumn, tnull: Option<&str>, row: usize) -> Option<&str> {
        let field = self.field(col, row).trim();
        (tnull != Some(field)).then_some(field)
    }
}

/// Parse an `Iw` field. A blank one is a genuine zero, not an undefined value
/// (§7.2.5) — only `TNULLn` marks undefined, and [`Ascii::defined_field`] filtered it.
fn parse_integer(field: &str) -> Result<i64> {
    if field.is_empty() {
        return Ok(0);
    }
    field.parse().map_err(|_| FitsError::InvalidAsciiField {
        field: field.to_string(),
    })
}

/// Parse an `Fw.d`/`Ew.d`/`Dw.d` field, with the same blank rule as [`parse_integer`].
fn parse_float(field: &str, decimals: usize) -> Result<f64> {
    if field.is_empty() {
        return Ok(0.0);
    }
    parse_ascii_float(field, decimals).ok_or_else(|| FitsError::InvalidAsciiField {
        field: field.to_string(),
    })
}

impl AsciiColumn {
    /// Byte offset of this column from the start of a row (`TBCOLn − 1`).
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    /// `TSCALn`, 1 when absent: `physical = TZERO + TSCAL × field`.
    pub fn tscale(&self) -> Result<f64> {
        self.tscale.clone().map_err(|defect| defect.error())
    }

    /// `TZEROn`, 0 when absent.
    pub fn tzero(&self) -> Result<f64> {
        self.tzero.clone().map_err(|defect| defect.error())
    }

    /// `TNULLn`, trimmed: the field text that marks an undefined value (§7.2.5).
    pub fn tnull(&self) -> Result<Option<&str>> {
        match &self.tnull {
            Ok(tnull) => Ok(tnull.as_deref()),
            Err(defect) => Err(defect.error()),
        }
    }

    /// The raw `TDISPn` display recommendation (§7.2.2), if declared.
    pub fn tdisp(&self) -> Result<Option<&str>> {
        match &self.tdisp {
            Ok(tdisp) => Ok(tdisp.as_deref()),
            Err(defect) => Err(defect.error()),
        }
    }
}

impl Named for AsciiColumn {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

/// Parse a Fortran `Fw.d`/`Ew.d`/`Dw.d` field. When a point-less `Fw.d` field has
/// no exponent, the decimal point is implied `decimals` digits from the right
/// (§7.2.1, deprecated): the integer mantissa is scaled by `10⁻ᵈ`.
///
/// Mantissa and exponent are joined into one decimal literal and parsed once, so the
/// value is the f64 nearest the decimal the field spells — what strtod gives cfitsio
/// and astropy — rather than a parsed mantissa times a rounded power of ten.
fn parse_ascii_float(field: &str, decimals: usize) -> Option<f64> {
    match split_mantissa_exponent(field) {
        Some(Scientific { mantissa, exponent }) => {
            parse_literal(format_args!("{mantissa}e{}", exponent.trim()))
        }
        // The §7.2.1 implied decimal point is an `Fw.d` legacy. cfitsio/astropy apply it
        // only to a bare mantissa and parse an explicit-exponent field literally, so
        // `1E5` is 100000, not `1·10⁻ᵈ·10⁵` — match them, since the whole point is to
        // read the files those tools write.
        None if decimals != 0 && !field.contains('.') => {
            parse_literal(format_args!("{field}e-{decimals}"))
        }
        None => field.parse().ok(),
    }
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
        b'A' => AsciiKind::Text,
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
pub(crate) mod internals {
    use crate::ascii::{Ascii, AsciiSchema};
    use crate::error::Result;
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
