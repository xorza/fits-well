//! ASCII-table writer schema, validation, and encoding.

use std::io::Write;

use crate::ascii::AsciiColumnData;
use crate::block::SPACE_FILL;
use crate::error::{FitsError, Result};
use crate::fits_writer::{
    FitsWriter, Opening, ROW_COUNT_FIXED, Scalable, accept_row_count, validate_scaling,
};
use crate::hdu::validate_table_field_count;
use crate::header_model::card::validate_ascii;
use crate::header_model::{Header, keywords, value};
use std::iter;

/// One column to write into an ASCII table: nullable typed data, the fixed field
/// width in characters, and the decimal count for floats.
#[derive(Debug, Clone)]
pub struct AsciiWriteColumn {
    pub(crate) name: String,
    pub(crate) unit: Option<String>,
    pub(crate) data: AsciiColumnData,
    pub(crate) width: usize,
    pub(crate) decimals: usize,
    /// Emit `TSCALn`/`TZEROn` (§7.2.2): `data` holds the stored field values and a
    /// reader recovers `TZEROn + TSCALn × field` physically.
    pub(crate) tscale: Option<f64>,
    pub(crate) tzero: Option<f64>,
    /// Emit `TNULLn`, the field text used to write `None` cells (§7.2.4).
    pub(crate) tnull: Option<String>,
}

impl AsciiWriteColumn {
    /// Construct an ASCII-table column. Integer and text columns ignore decimal
    /// precision; float columns default to zero digits after the decimal point.
    pub fn new(name: impl Into<String>, data: AsciiColumnData, width: usize) -> Self {
        Self {
            name: name.into(),
            unit: None,
            data,
            width,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    #[must_use]
    pub const fn with_decimals(mut self, decimals: usize) -> Self {
        self.decimals = decimals;
        self
    }

    #[must_use]
    pub const fn with_scaling(mut self, tscale: f64, tzero: f64) -> Self {
        self.tscale = Some(tscale);
        self.tzero = Some(tzero);
        self
    }

    #[must_use]
    pub fn with_null(mut self, tnull: impl Into<String>) -> Self {
        self.tnull = Some(tnull.into());
        self
    }
}

/// Validated ASCII-table write value with the same inferred-row workflow as
/// [`Builder`](crate::fits_writer::builder::Builder).
#[derive(Debug, Clone, Default)]
pub struct AsciiBuilder {
    pub(crate) row_count: Option<usize>,
    pub(crate) columns: Vec<AsciiWriteColumn>,
}

impl AsciiBuilder {
    pub const fn new() -> Self {
        Self {
            row_count: None,
            columns: Vec::new(),
        }
    }

    pub const fn with_rows(row_count: usize) -> Self {
        Self {
            row_count: Some(row_count),
            columns: Vec::new(),
        }
    }

    pub fn push(&mut self, column: AsciiWriteColumn) -> Result<&mut Self> {
        // An ASCII column is always scalar (§7.2), so its value count *is* its row
        // count and it always determines one.
        accept_row_count(&mut self.row_count, &column.name, Some(column.data.len()))?;
        self.columns.push(column);
        Ok(self)
    }

    pub fn column(mut self, column: AsciiWriteColumn) -> Result<Self> {
        self.push(column)?;
        Ok(self)
    }

    pub fn with_columns(
        row_count: usize,
        columns: impl IntoIterator<Item = AsciiWriteColumn>,
    ) -> Result<Self> {
        let mut table = Self::with_rows(row_count);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }
}

pub(super) fn write_template<W: Write>(
    writer: &mut FitsWriter<W>,
    table: &AsciiBuilder,
    template: Option<&Header>,
) -> Result<()> {
    writer.ensure_writable()?;
    let row_count = table.row_count.unwrap_or(0);
    let columns = &table.columns;
    validate_table_field_count(columns.len())?;
    let mut tbcols = Vec::with_capacity(columns.len());
    let mut row_len = 0usize;
    for (i, col) in columns.iter().enumerate() {
        validate_ascii_column(col, i + 1)?;
        assert_eq!(col.data.len(), row_count, "{ROW_COUNT_FIXED}");
        tbcols.push(row_len.checked_add(1).ok_or(FitsError::DataUnitOverflow)?);
        row_len = row_len
            .checked_add(col.width)
            .ok_or(FitsError::DataUnitOverflow)?;
    }
    let header = ascii_table_header(row_count, row_len, columns, &tbcols)?;
    writer.scratch.clear();
    let total_len = row_count
        .checked_mul(row_len)
        .ok_or(FitsError::DataUnitOverflow)?;
    writer.scratch.reserve_exact(total_len);
    for r in 0..row_count {
        for col in columns {
            append_ascii_field(&mut writer.scratch, col, r)?;
        }
    }
    writer.finish_hdu(header, template, SPACE_FILL, Opening::EmptyPrimary)
}

/// Errors when `col`, the table's column `n`, cannot be written.
fn validate_ascii_column(col: &AsciiWriteColumn, n: usize) -> Result<()> {
    let tnull = FitsError::KeywordOutOfRange {
        keyword: keywords::TNULL.indexed(n)?,
    };
    validate_ascii(&col.name, "ASCII column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "ASCII column unit")?;
    }
    if let Some(marker) = &col.tnull {
        validate_ascii(marker, "ASCII null marker")?;
        if marker.trim().is_empty() || marker.len() > col.width {
            return Err(tnull);
        }
    }
    let scalable = match &col.data {
        AsciiColumnData::Text(_) => Scalable::Unscaled,
        AsciiColumnData::Integer(_) | AsciiColumnData::Float(_) => Scalable::Numeric,
    };
    validate_scaling(col.tscale, col.tzero, scalable, n)?;
    if col.data.has_null() && col.tnull.is_none() {
        return Err(tnull);
    }
    Ok(())
}

fn validate_ascii_field_width(
    col: &AsciiWriteColumn,
    row: usize,
    minimum_width: usize,
) -> Result<()> {
    if minimum_width > col.width {
        return Err(FitsError::AsciiFieldTooWide {
            column: col.name.clone(),
            row,
            width: col.width,
            minimum_width,
        });
    }
    Ok(())
}

/// One rendered ASCII-table field.
#[derive(Debug)]
enum AsciiField<'a> {
    /// A text cell's value, left-aligned.
    Text(&'a str),
    /// An integer or float cell's value, right-aligned.
    Number(String),
    /// The column's validated `TNULLn` marker, left-aligned whatever the column's type.
    Null(&'a str),
}

impl<'a> AsciiField<'a> {
    fn null(col: &'a AsciiWriteColumn) -> Self {
        Self::Null(
            col.tnull
                .as_deref()
                .expect("null ASCII cells require a validated TNULLn marker"),
        )
    }

    fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Null(text) => text,
            Self::Number(text) => text,
        }
    }
}

/// Render row `row` of `col` and validate it: width, restricted ASCII, and that a
/// genuine value does not collide with the column's null marker.
fn ascii_field(col: &AsciiWriteColumn, row: usize) -> Result<AsciiField<'_>> {
    let field = match &col.data {
        AsciiColumnData::Text(values) => match values.get(row) {
            Some(value) => AsciiField::Text(value),
            None => AsciiField::null(col),
        },
        AsciiColumnData::Integer(values) => match values[row] {
            Some(value) => AsciiField::Number(value.to_string()),
            None => AsciiField::null(col),
        },
        AsciiColumnData::Float(values) => match values[row] {
            Some(value) => AsciiField::Number(ascii_float_text(col, row, value)?),
            None => AsciiField::null(col),
        },
    };
    validate_ascii_field_width(col, row, field.text().len())?;
    if let AsciiColumnData::Text(values) = &col.data
        && let Some(value) = values.get(row)
    {
        validate_ascii(value, "ASCII text cell")?;
    }
    if !matches!(field, AsciiField::Null(_)) {
        // Integer and float text never carries surrounding spaces, so trimming is a
        // no-op there and applies the §7.2.4 comparison to a text cell's own value.
        validate_ascii_null_collision(field.text().trim(), col.tnull.as_deref().map(str::trim))?;
    }
    Ok(field)
}

/// Format a finite float to the column's declared decimals, checking up front that
/// its sign and precision can fit the field at all.
fn ascii_float_text(col: &AsciiWriteColumn, row: usize, value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(FitsError::InvalidAsciiValue {
            reason: "float cells must be finite; use None for null",
        });
    }
    let sign_width = usize::from(value.is_sign_negative());
    let minimum_width = if col.decimals == 0 {
        1 + sign_width
    } else {
        if col.decimals > col.width {
            validate_ascii_field_width(col, row, col.decimals)?;
        }
        col.decimals
            .checked_add(2 + sign_width)
            .ok_or(FitsError::DataUnitOverflow)?
    };
    validate_ascii_field_width(col, row, minimum_width)?;
    Ok(format!("{:.*}", col.decimals, value))
}

fn validate_ascii_null_collision(value: &str, marker: Option<&str>) -> Result<()> {
    if marker == Some(value) {
        Err(FitsError::InvalidAsciiValue {
            reason: "the value equals its column's TNULLn marker",
        })
    } else {
        Ok(())
    }
}

/// `TABLE` extension header (§7.2) for the given columns and computed `TBCOLn`s.
fn ascii_table_header(
    row_count: usize,
    row_len: usize,
    columns: &[AsciiWriteColumn],
    tbcols: &[usize],
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set_internal(keywords::XTENSION, "TABLE")
        .set_comment_internal(keywords::XTENSION, "ASCII table extension");
    header
        .set_internal(keywords::BITPIX, 8)
        .set_internal(keywords::NAXIS, 2);
    header
        .set_internal(keywords::NAXIS.indexed(1)?, value::fits_i64(row_len)?)
        .set_comment_internal(keywords::NAXIS.indexed(1)?, "width of table in characters");
    header
        .set_internal(keywords::NAXIS.indexed(2)?, value::fits_i64(row_count)?)
        .set_comment_internal(keywords::NAXIS.indexed(2)?, "number of rows");
    header
        .set_internal(keywords::PCOUNT, 0)
        .set_internal(keywords::GCOUNT, 1);
    header
        .set_internal(keywords::TFIELDS, value::fits_i64(columns.len())?)
        .set_comment_internal(keywords::TFIELDS, "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set_internal(keywords::TBCOL.indexed(n)?, value::fits_i64(tbcols[i])?);
        header.set_internal(keywords::TFORM.indexed(n)?, ascii_tform(col));
        header.set_internal(keywords::TTYPE.indexed(n)?, col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set_internal(keywords::TUNIT.indexed(n)?, unit.as_str());
        }
        if let Some(tscale) = col.tscale {
            header.set_internal(keywords::TSCAL.indexed(n)?, tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set_internal(keywords::TZERO.indexed(n)?, tzero);
        }
        if let Some(tnull) = &col.tnull {
            header.set_internal(keywords::TNULL.indexed(n)?, tnull.as_str());
        }
    }
    Ok(header)
}

fn ascii_tform(col: &AsciiWriteColumn) -> String {
    match col.data {
        AsciiColumnData::Text(_) => format!("A{}", col.width),
        AsciiColumnData::Integer(_) => format!("I{}", col.width),
        AsciiColumnData::Float(_) => format!("F{}.{}", col.width, col.decimals),
    }
}

fn append_ascii_field(out: &mut Vec<u8>, col: &AsciiWriteColumn, r: usize) -> Result<()> {
    let field = ascii_field(col, r)?;
    let bytes = field.text().as_bytes();
    let pad = iter::repeat_n(b' ', col.width - bytes.len());
    match field {
        AsciiField::Text(_) | AsciiField::Null(_) => {
            out.extend_from_slice(bytes);
            out.extend(pad);
        }
        AsciiField::Number(_) => {
            out.extend(pad);
            out.extend_from_slice(bytes);
        }
    }
    Ok(())
}
