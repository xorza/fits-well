//! ASCII-table writer schema, validation, and encoding.

use std::fmt::Write as _;
use std::iter;

use crate::ascii::ascii_text::AsciiText;
use crate::bitpix::Bitpix;
use crate::column::validate_table_field_count;
use crate::error::{AsciiValueDefect, FitsError, Result, TextOrigin};
use crate::fits_writer::column_scaling::{ColumnScaling, Scalable};
use crate::fits_writer::{ROW_COUNT_FIXED, accept_row_count};
use crate::hdu::prologue::{Opening, Prologue, Xtension};
use crate::header_model::card::validate_ascii;
use crate::header_model::{Header, keywords, value};

/// One column to write into an ASCII table: its name, its nullable values and the
/// `TFORMn` they are written with.
#[derive(Debug, Clone)]
pub struct AsciiWriteColumn {
    name: String,
    unit: Option<String>,
    cells: AsciiCells,
    /// Emit `TSCALn`/`TZEROn` (§7.2.2): the cells hold the stored field values and a
    /// reader recovers `TZEROn + TSCALn × field` physically.
    scaling: Option<ColumnScaling>,
    /// Emit `TNULLn`, the field text used to write `None` cells (§7.2.4).
    tnull: Option<String>,
}

/// The values of an ASCII column and the `TFORMn` they are written with.
#[derive(Debug, Clone)]
enum AsciiCells {
    /// `Aw`, left-aligned.
    Text { values: AsciiText, width: usize },
    /// `Iw`, right-aligned.
    Integer {
        values: Vec<Option<i64>>,
        width: usize,
    },
    /// `Fw.d`, `Ew.d` or `Dw.d`, right-aligned.
    Float {
        values: Vec<Option<f64>>,
        format: AsciiFloatFormat,
    },
}

/// The `TFORMn` of an ASCII float column (§7.2.5): the field width and the digits after
/// the decimal point, which every cell holds. A value is rounded to `decimals` digits,
/// and one too wide for `width` is refused. The cells are written as cfitsio writes
/// them through C's `%w.df` and `%w.dE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiFloatFormat {
    /// `Fw.d`: fixed point, as `-12.50`.
    F { width: usize, decimals: usize },
    /// `Ew.d`: one digit before the point and a signed exponent of two digits or more,
    /// as `-1.25E+01`.
    E { width: usize, decimals: usize },
    /// `Dw.d`: as `Ew.d`, with the exponent letter `D`.
    D { width: usize, decimals: usize },
}

impl AsciiFloatFormat {
    pub const fn width(self) -> usize {
        match self {
            Self::F { width, .. } | Self::E { width, .. } | Self::D { width, .. } => width,
        }
    }

    pub const fn decimals(self) -> usize {
        match self {
            Self::F { decimals, .. } | Self::E { decimals, .. } | Self::D { decimals, .. } => {
                decimals
            }
        }
    }

    const fn letter(self) -> char {
        match self {
            Self::F { .. } => 'F',
            Self::E { .. } => 'E',
            Self::D { .. } => 'D',
        }
    }

    /// The width of the narrowest positive cell: `0.` and the decimals, and for an
    /// exponent form `E+00` too; `None` past `usize::MAX`.
    const fn minimum_width(self) -> Option<usize> {
        let point = match self {
            Self::F { .. } => 2,
            Self::E { .. } | Self::D { .. } => 6,
        };
        self.decimals().checked_add(point)
    }

    /// Append `value`, which is finite, to `out`.
    fn write(self, value: f64, out: &mut String) {
        debug_assert!(value.is_finite(), "a float cell is finite");
        let start = out.len();
        let decimals = self.decimals();
        match self {
            Self::F { .. } => {
                write!(out, "{value:.decimals$}").expect("a `String` takes every write");
                if decimals == 0 {
                    out.push('.');
                }
            }
            Self::E { .. } | Self::D { .. } => {
                write!(out, "{value:.decimals$E}").expect("a `String` takes every write");
                let at = start
                    + out[start..]
                        .find('E')
                        .expect("the exponent form writes an `E`");
                let exponent: i32 = out[at + 1..]
                    .parse()
                    .expect("the exponent form writes an integer exponent");
                out.truncate(at);
                if decimals == 0 {
                    out.push('.');
                }
                let sign = if exponent < 0 { '-' } else { '+' };
                write!(out, "{}{sign}{:02}", self.letter(), exponent.unsigned_abs())
                    .expect("a `String` takes every write");
            }
        }
    }
}

impl AsciiCells {
    const fn len(&self) -> usize {
        match self {
            Self::Text { values, .. } => values.len(),
            Self::Integer { values, .. } => values.len(),
            Self::Float { values, .. } => values.len(),
        }
    }

    const fn width(&self) -> usize {
        match self {
            Self::Text { width, .. } | Self::Integer { width, .. } => *width,
            Self::Float { format, .. } => format.width(),
        }
    }

    fn has_null(&self) -> bool {
        match self {
            Self::Text { values, .. } => values.has_null(),
            Self::Integer { values, .. } => values.contains(&None),
            Self::Float { values, .. } => values.contains(&None),
        }
    }

    const fn scalable(&self) -> Scalable {
        match self {
            Self::Text { .. } => Scalable::Unscaled,
            Self::Integer { .. } | Self::Float { .. } => Scalable::Numeric,
        }
    }

    fn tform(&self) -> String {
        match self {
            Self::Text { width, .. } => format!("A{width}"),
            Self::Integer { width, .. } => format!("I{width}"),
            Self::Float { format, .. } => {
                format!(
                    "{}{}.{}",
                    format.letter(),
                    format.width(),
                    format.decimals()
                )
            }
        }
    }
}

impl AsciiWriteColumn {
    /// An `Aw` column of `width` characters.
    pub fn text(name: impl Into<String>, values: AsciiText, width: usize) -> Self {
        Self::of(name.into(), AsciiCells::Text { values, width })
    }

    /// An `Iw` column of `width` characters.
    pub fn integer(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<i64>>,
        width: usize,
    ) -> Self {
        Self::of(
            name.into(),
            AsciiCells::Integer {
                values: values.into_iter().collect(),
                width,
            },
        )
    }

    /// A float column written with `format`.
    pub fn float(
        name: impl Into<String>,
        values: impl IntoIterator<Item = Option<f64>>,
        format: AsciiFloatFormat,
    ) -> Self {
        Self::of(
            name.into(),
            AsciiCells::Float {
                values: values.into_iter().collect(),
                format,
            },
        )
    }

    const fn of(name: String, cells: AsciiCells) -> Self {
        Self {
            name,
            unit: None,
            cells,
            scaling: None,
            tnull: None,
        }
    }

    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    #[must_use]
    pub const fn with_scaling(mut self, tscale: f64, tzero: f64) -> Self {
        self.scaling = Some(ColumnScaling { tscale, tzero });
        self
    }

    #[must_use]
    pub fn with_null(mut self, tnull: impl Into<String>) -> Self {
        self.tnull = Some(tnull.into());
        self
    }

    /// Errors when this column, the table's column `n`, cannot be written.
    fn validate(&self, n: usize) -> Result<()> {
        let tnull = FitsError::KeywordOutOfRange {
            keyword: keywords::TNULL.indexed(n)?,
        };
        validate_ascii(&self.name, || TextOrigin::ColumnName)?;
        if let Some(unit) = &self.unit {
            validate_ascii(unit, || TextOrigin::ColumnUnit)?;
        }
        if let Some(marker) = &self.tnull {
            validate_ascii(marker, || TextOrigin::NullMarker)?;
            if marker.trim().is_empty() || marker.len() > self.cells.width() {
                return Err(tnull);
            }
        }
        if let Some(scaling) = self.scaling {
            scaling.validate(self.cells.scalable(), n)?;
        }
        if self.cells.has_null() && self.tnull.is_none() {
            return Err(tnull);
        }
        if let AsciiCells::Float { format, .. } = self.cells {
            let minimum_width = format.minimum_width().unwrap_or(usize::MAX);
            if minimum_width > format.width() {
                return Err(FitsError::AsciiFloatFormatTooNarrow {
                    column: self.name.clone(),
                    width: format.width(),
                    minimum_width,
                });
            }
        }
        Ok(())
    }

    /// Render row `row` into `number` when it is a defined number, and give its field
    /// after checking it: width, restricted ASCII, and that a defined value does not
    /// collide with the column's null marker.
    fn field<'a>(&'a self, row: usize, number: &'a mut String) -> Result<AsciiField<'a>> {
        number.clear();
        let field = match &self.cells {
            AsciiCells::Text { values, .. } => match values.row(row) {
                Some(value) => {
                    validate_ascii(value, || TextOrigin::Cell)?;
                    AsciiField::Text(value)
                }
                None => self.null(),
            },
            AsciiCells::Integer { values, .. } => match values[row] {
                Some(value) => {
                    write!(number, "{value}").expect("a `String` takes every write");
                    AsciiField::Number(number)
                }
                None => self.null(),
            },
            AsciiCells::Float { values, format } => match values[row] {
                Some(value) if !value.is_finite() => {
                    return Err(FitsError::InvalidAsciiValue {
                        column: self.name.clone(),
                        row,
                        defect: AsciiValueDefect::NonFinite,
                    });
                }
                Some(value) => {
                    format.write(value, number);
                    AsciiField::Number(number)
                }
                None => self.null(),
            },
        };
        let width = self.cells.width();
        if field.text().len() > width {
            return Err(FitsError::AsciiFieldTooWide {
                column: self.name.clone(),
                row,
                width,
                minimum_width: field.text().len(),
            });
        }
        // A number never carries surrounding spaces, so trimming applies the §7.2.4
        // comparison to a text cell's own value.
        if !matches!(field, AsciiField::Null(_))
            && self.tnull.as_deref().map(str::trim) == Some(field.text().trim())
        {
            return Err(FitsError::InvalidAsciiValue {
                column: self.name.clone(),
                row,
                defect: AsciiValueDefect::NullMarker,
            });
        }
        Ok(field)
    }

    fn null(&self) -> AsciiField<'_> {
        AsciiField::Null(
            self.tnull
                .as_deref()
                .expect("null ASCII cells require a validated TNULLn marker"),
        )
    }

    /// Append row `row`'s field, padded to the column's width, to `out`.
    fn append_field(&self, row: usize, number: &mut String, out: &mut Vec<u8>) -> Result<()> {
        let width = self.cells.width();
        let field = self.field(row, number)?;
        let bytes = field.text().as_bytes();
        let pad = iter::repeat_n(b' ', width - bytes.len());
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
}

/// One rendered ASCII-table field.
#[derive(Debug)]
enum AsciiField<'a> {
    /// A text cell's value, left-aligned.
    Text(&'a str),
    /// An integer or float cell's value, right-aligned.
    Number(&'a str),
    /// The column's validated `TNULLn` marker, left-aligned whatever the column's type.
    Null(&'a str),
}

impl AsciiField<'_> {
    const fn text(&self) -> &str {
        match self {
            Self::Text(text) | Self::Number(text) | Self::Null(text) => text,
        }
    }
}

/// Validated ASCII-table write value with the same inferred-row workflow as
/// [`Builder`](crate::fits_writer::builder::Builder).
#[derive(Debug, Clone, Default)]
pub struct AsciiBuilder {
    row_count: Option<usize>,
    columns: Vec<AsciiWriteColumn>,
}

/// The extent of the table an [`AsciiBuilder`] writes.
#[derive(Debug)]
struct AsciiLayout {
    row_count: usize,
    row_len: usize,
    /// Each column's 1-based `TBCOLn`.
    tbcols: Vec<usize>,
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
        accept_row_count(&mut self.row_count, &column.name, Some(column.cells.len()))?;
        self.columns.push(column);
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

    /// The extent of the table this writes, checked before any byte is.
    fn layout(&self) -> Result<AsciiLayout> {
        let row_count = self.row_count.unwrap_or(0);
        validate_table_field_count(self.columns.len())?;
        let mut tbcols = Vec::with_capacity(self.columns.len());
        let mut row_len = 0usize;
        for (i, col) in self.columns.iter().enumerate() {
            col.validate(i + 1)?;
            assert_eq!(col.cells.len(), row_count, "{ROW_COUNT_FIXED}");
            tbcols.push(row_len.checked_add(1).ok_or(FitsError::DataUnitOverflow)?);
            row_len = row_len
                .checked_add(col.cells.width())
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        Ok(AsciiLayout {
            row_count,
            row_len,
            tbcols,
        })
    }

    /// The `TABLE` extension header (§7.2) of the table laid out as `layout`.
    fn header(&self, layout: &AsciiLayout) -> Result<Header> {
        let mut header = Prologue {
            opening: Opening::Extension(Xtension::Table),
            bitpix: Bitpix::U8,
            axes: &[layout.row_len, layout.row_count],
            pcount: 0,
        }
        .header()?;
        header
            .set_comment_internal(keywords::NAXIS.indexed(1)?, "width of table in characters")
            .set_comment_internal(keywords::NAXIS.indexed(2)?, "number of rows");
        header
            .set_internal(keywords::TFIELDS, value::fits_i64(self.columns.len())?)
            .set_comment_internal(keywords::TFIELDS, "number of columns");
        for (i, (col, &tbcol)) in self.columns.iter().zip(&layout.tbcols).enumerate() {
            let n = i + 1;
            header.set_internal(keywords::TBCOL.indexed(n)?, value::fits_i64(tbcol)?);
            header.set_internal(keywords::TFORM.indexed(n)?, col.cells.tform());
            header.set_internal(keywords::TTYPE.indexed(n)?, col.name.as_str());
            if let Some(unit) = &col.unit {
                header.set_internal(keywords::TUNIT.indexed(n)?, unit.as_str());
            }
            if let Some(scaling) = col.scaling {
                header.set_internal(keywords::TSCAL.indexed(n)?, scaling.tscale);
                header.set_internal(keywords::TZERO.indexed(n)?, scaling.tzero);
            }
            if let Some(tnull) = &col.tnull {
                header.set_internal(keywords::TNULL.indexed(n)?, tnull.as_str());
            }
        }
        Ok(header)
    }

    /// The `TABLE` header, with the data unit written to `out`.
    pub(super) fn encode(&self, out: &mut Vec<u8>) -> Result<Header> {
        let layout = self.layout()?;
        let header = self.header(&layout)?;
        out.clear();
        out.reserve_exact(
            layout
                .row_count
                .checked_mul(layout.row_len)
                .ok_or(FitsError::DataUnitOverflow)?,
        );
        let mut number = String::new();
        for row in 0..layout.row_count {
            for col in &self.columns {
                col.append_field(row, &mut number, out)?;
            }
        }
        Ok(header)
    }
}
