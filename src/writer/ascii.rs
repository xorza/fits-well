//! ASCII-table writer schema, validation, and encoding.

use std::borrow::Cow;
use std::io::Write;

use crate::ascii::AsciiColumnData;
use crate::block::SPACE_FILL;
use crate::error::{FitsError, Result};
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::header::card::validate_ascii;
use crate::header::value;
use crate::keyword::key;
use crate::writer::{FitsWriter, accept_row_count, merge_header_template, validate_scaling};

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
    pub fn new(name: impl Into<String>, data: AsciiColumnData, width: usize) -> AsciiWriteColumn {
        AsciiWriteColumn {
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

    pub fn with_unit(mut self, unit: impl Into<String>) -> AsciiWriteColumn {
        self.unit = Some(unit.into());
        self
    }

    pub fn with_decimals(mut self, decimals: usize) -> AsciiWriteColumn {
        self.decimals = decimals;
        self
    }

    pub fn scaled(mut self, tscale: f64, tzero: f64) -> AsciiWriteColumn {
        self.tscale = Some(tscale);
        self.tzero = Some(tzero);
        self
    }

    pub fn with_null(mut self, tnull: impl Into<String>) -> AsciiWriteColumn {
        self.tnull = Some(tnull.into());
        self
    }
}

/// Validated ASCII-table write value with the same inferred-row workflow as
/// [`TableBuilder`](crate::writer::table::TableBuilder).
#[derive(Debug, Clone, Default)]
pub struct AsciiTableBuilder {
    pub(crate) nrows: Option<usize>,
    pub(crate) columns: Vec<AsciiWriteColumn>,
}

impl AsciiTableBuilder {
    pub fn new() -> AsciiTableBuilder {
        AsciiTableBuilder::default()
    }

    pub fn with_rows(nrows: usize) -> AsciiTableBuilder {
        AsciiTableBuilder {
            nrows: Some(nrows),
            columns: Vec::new(),
        }
    }

    pub fn push(&mut self, column: AsciiWriteColumn) -> Result<&mut Self> {
        // An ASCII column is always scalar (§7.2), so its value count *is* its row
        // count and it always determines one.
        accept_row_count(&mut self.nrows, &column.name, Some(column.data.len()))?;
        self.columns.push(column);
        Ok(self)
    }

    pub fn column(mut self, column: AsciiWriteColumn) -> Result<AsciiTableBuilder> {
        self.push(column)?;
        Ok(self)
    }

    pub fn explicit(
        nrows: usize,
        columns: impl IntoIterator<Item = AsciiWriteColumn>,
    ) -> Result<AsciiTableBuilder> {
        let mut table = AsciiTableBuilder::with_rows(nrows);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }
}

impl<W: Write> FitsWriter<W> {
    /// Write an ASCII table as a `TABLE` extension (a dataless primary is written
    /// first if needed). Columns are packed left-to-right with no gaps; data is
    /// space-padded per §7.2.3.
    pub fn write_ascii_table(&mut self, table: &AsciiTableBuilder) -> Result<()> {
        self.write_ascii_table_template(table, None)
    }

    /// Write an ASCII table while preserving the non-structural cards from `header`.
    /// Mandatory table-layout and checksum cards are regenerated from `columns`.
    pub fn write_ascii_table_with_header(
        &mut self,
        table: &AsciiTableBuilder,
        header: &Header,
    ) -> Result<()> {
        self.write_ascii_table_template(table, Some(header))
    }

    fn write_ascii_table_template(
        &mut self,
        table: &AsciiTableBuilder,
        template: Option<&Header>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let nrows = table.nrows.unwrap_or(0);
        let columns = &table.columns;
        validate_table_field_count(columns.len())?;
        let mut tbcols = Vec::with_capacity(columns.len());
        let mut row_len = 0usize;
        for col in columns {
            validate_ascii_column(col)?;
            let count = col.data.len();
            if count != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: count,
                    declared: nrows,
                });
            }
            tbcols.push(row_len.checked_add(1).ok_or(FitsError::DataUnitOverflow)?); // 1-based start column
            row_len = row_len
                .checked_add(col.width)
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        let header = ascii_table_header(nrows, row_len, columns, &tbcols, template)?;
        self.scratch.clear();
        let total_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.scratch.reserve_exact(total_len);
        for r in 0..nrows {
            for col in columns {
                append_ascii_field(&mut self.scratch, col, r)?;
            }
        }
        self.finish_hdu(header, SPACE_FILL, true)
    }
}

fn validate_ascii_column(col: &AsciiWriteColumn) -> Result<()> {
    validate_ascii(&col.name, "ASCII column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "ASCII column unit")?;
    }
    let marker = col.tnull.as_deref().map(str::trim);
    if let Some(marker) = &col.tnull {
        validate_ascii(marker, "ASCII null marker")?;
        if marker.trim().is_empty() || marker.len() > col.width {
            return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
        }
    }
    validate_scaling(
        col.tscale,
        col.tzero,
        !matches!(&col.data, AsciiColumnData::Text(_)),
    )?;
    if col.data.has_null() && marker.is_none() {
        return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
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

/// One rendered ASCII-table field: the text to write, whether it pads on the right,
/// and whether it is the `TNULLn` marker rather than a value.
#[derive(Debug)]
struct AsciiField<'a> {
    text: Cow<'a, str>,
    left_aligned: bool,
    is_null: bool,
}

impl<'a> AsciiField<'a> {
    /// A null cell renders its validated `TNULLn` marker, left-aligned whatever the
    /// column's type.
    fn null(col: &'a AsciiWriteColumn) -> AsciiField<'a> {
        AsciiField {
            text: Cow::Borrowed(
                col.tnull
                    .as_deref()
                    .expect("null ASCII cells require a validated TNULLn marker"),
            ),
            left_aligned: true,
            is_null: true,
        }
    }

    fn value(text: Cow<'a, str>, left_aligned: bool) -> AsciiField<'a> {
        AsciiField {
            text,
            left_aligned,
            is_null: false,
        }
    }
}

/// Render row `row` of `col` and validate it: width, restricted ASCII, and that a
/// genuine value does not collide with the column's null marker.
fn ascii_field<'a>(col: &'a AsciiWriteColumn, row: usize) -> Result<AsciiField<'a>> {
    let field = match &col.data {
        AsciiColumnData::Text(values) => match &values[row] {
            Some(value) => AsciiField::value(Cow::Borrowed(value.as_str()), true),
            None => AsciiField::null(col),
        },
        AsciiColumnData::Integer(values) => match values[row] {
            Some(value) => AsciiField::value(Cow::Owned(value.to_string()), false),
            None => AsciiField::null(col),
        },
        AsciiColumnData::Float(values) => match values[row] {
            Some(value) => AsciiField::value(Cow::Owned(ascii_float_text(col, row, value)?), false),
            None => AsciiField::null(col),
        },
    };
    validate_ascii_field_width(col, row, field.text.len())?;
    if let AsciiColumnData::Text(values) = &col.data
        && let Some(value) = &values[row]
    {
        validate_ascii(value, "ASCII text cell")?;
    }
    if !field.is_null {
        // Integer and float text never carries surrounding spaces, so trimming is a
        // no-op there and applies the §7.2.4 comparison to a text cell's own value.
        validate_ascii_null_collision(field.text.trim(), col.tnull.as_deref().map(str::trim))?;
    }
    Ok(field)
}

/// Format a finite float to the column's declared decimals, checking up front that
/// its sign and precision can fit the field at all.
fn ascii_float_text(col: &AsciiWriteColumn, row: usize, value: f64) -> Result<String> {
    if !value.is_finite() {
        return Err(FitsError::InvalidValue {
            card: "ASCII float cells must be finite; use None for null".to_string(),
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
        Err(FitsError::InvalidValue {
            card: "ASCII value equals its TNULLn marker".to_string(),
        })
    } else {
        Ok(())
    }
}

/// `TABLE` extension header (§7.2) for the given columns and computed `TBCOLn`s.
fn ascii_table_header(
    nrows: usize,
    row_len: usize,
    columns: &[AsciiWriteColumn],
    tbcols: &[usize],
    template: Option<&Header>,
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .comment_internal("XTENSION", "ASCII table extension");
    header.set_internal("BITPIX", 8).set_internal("NAXIS", 2);
    header
        .set_internal("NAXIS1", value::fits_i64(row_len)?)
        .comment_internal("NAXIS1", "width of table in characters");
    header
        .set_internal("NAXIS2", value::fits_i64(nrows)?)
        .comment_internal("NAXIS2", "number of rows");
    header.set_internal("PCOUNT", 0).set_internal("GCOUNT", 1);
    header
        .set_internal("TFIELDS", value::fits_i64(columns.len())?)
        .comment_internal("TFIELDS", "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set_internal(key!("TBCOL{n}").as_str(), value::fits_i64(tbcols[i])?);
        header.set_internal(key!("TFORM{n}").as_str(), ascii_tform(col));
        header.set_internal(key!("TTYPE{n}").as_str(), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set_internal(key!("TUNIT{n}").as_str(), unit.as_str());
        }
        if let Some(tscale) = col.tscale {
            header.set_internal(key!("TSCAL{n}").as_str(), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set_internal(key!("TZERO{n}").as_str(), tzero);
        }
        if let Some(tnull) = &col.tnull {
            header.set_internal(key!("TNULL{n}").as_str(), tnull.as_str());
        }
    }
    merge_header_template(&mut header, template);
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
    let bytes = field.text.as_bytes();
    let pad = col.width - bytes.len();
    if field.left_aligned {
        out.extend_from_slice(bytes);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(bytes);
    }
    Ok(())
}
