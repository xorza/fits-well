//! Addressing a table column by name or index, shared by both table forms.
//!
//! §6.7 (binary) and §7.2.2 (ASCII) give the two forms the same rule: a column is
//! named by its `TTYPEn` value, compared without regard to case. The two table
//! models are otherwise unrelated, so the rule lives here rather than being spelled
//! once per model — and once more in the reader, which resolves names against a
//! schema parsed without ever reading a table.

use std::result;
use std::sync::Arc;

use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;

/// A table column that may carry a `TTYPEn` name.
pub(crate) trait Named {
    /// The column's `TTYPEn` value, or `None` where the card is absent or empty.
    fn name(&self) -> Option<&str>;
}

/// The index of the first column whose `TTYPEn` matches `name`, compared
/// case-insensitively. Unnamed columns never match, including on an empty `name`.
pub(crate) fn index_of<C: Named>(columns: &[C], name: &str) -> Option<usize> {
    columns.iter().position(|column| {
        column
            .name()
            .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
    })
}

/// [`index_of`], reporting an absent column as [`FitsError::ColumnNotFound`].
pub(crate) fn checked_index_of<C: Named>(columns: &[C], name: &str) -> Result<usize> {
    index_of(columns, name).ok_or_else(|| FitsError::ColumnNotFound {
        name: name.to_string(),
    })
}

/// Bounds-check a zero-based column index against a table's column count.
pub(crate) const fn validate_index(index: usize, len: usize) -> Result<()> {
    if index >= len {
        return Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Column,
            index,
            len,
        });
    }
    Ok(())
}

/// A table column: its zero-based index, or its `TTYPEn` name, compared
/// case-insensitively (§6.7, §7.2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnSelector<'a> {
    Index(usize),
    Name(&'a str),
}

impl From<usize> for ColumnSelector<'_> {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl<'a> From<&'a str> for ColumnSelector<'a> {
    fn from(name: &'a str) -> Self {
        Self::Name(name)
    }
}

impl<'a> From<&'a String> for ColumnSelector<'a> {
    fn from(name: &'a String) -> Self {
        Self::Name(name)
    }
}

impl From<&Self> for ColumnSelector<'_> {
    fn from(selector: &Self) -> Self {
        *selector
    }
}

impl ColumnSelector<'_> {
    /// The index of the column this selects among `columns`. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last column, and with
    /// [`FitsError::ColumnNotFound`] for a name no column carries.
    pub(crate) fn resolve<C: Named>(self, columns: &[C]) -> Result<usize> {
        match self {
            Self::Index(index) => {
                validate_index(index, columns.len())?;
                Ok(index)
            }
            Self::Name(name) => checked_index_of(columns, name),
        }
    }
}

/// The most columns a table holds (§7.2.1, §7.3.1: `0 ≤ TFIELDS ≤ 999`).
pub(crate) const MAX_TABLE_FIELDS: usize = 999;

/// Errors when a table of `count` columns has more than [`MAX_TABLE_FIELDS`].
pub(crate) const fn validate_table_field_count(count: usize) -> Result<()> {
    if count > MAX_TABLE_FIELDS {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::TFIELDS,
        });
    }
    Ok(())
}

/// The rows and the column count that both table forms read first: `NAXIS1`,
/// `NAXIS2` and `TFIELDS`, once the structure both share is checked.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableShape {
    pub(crate) row_len: usize,
    pub(crate) row_count: usize,
    pub(crate) field_count: usize,
}

impl TableShape {
    /// The shape of the table `header` describes. A table is bytes in two axes, one
    /// group of them (§7.2.1, §7.3.1), so `BITPIX = 8`, `NAXIS = 2` and `GCOUNT = 1`, as
    /// the data unit the scan sizes from them holds the table's rows. `TFIELDS` is
    /// checked against [`MAX_TABLE_FIELDS`] here, as it sizes the column list and drives
    /// the `TFORMn` loop.
    pub(crate) fn read(header: &Header) -> Result<Self> {
        for (keyword, required) in [
            (keywords::BITPIX, 8),
            (keywords::NAXIS, 2),
            (keywords::GCOUNT, 1),
        ] {
            if header.require_integer(keyword)? != required {
                return Err(FitsError::KeywordOutOfRange { keyword });
            }
        }
        let field_count = header.require_usize(keywords::TFIELDS)?;
        validate_table_field_count(field_count)?;
        Ok(Self {
            row_len: header.require_usize(keywords::NAXIS.indexed(1)?)?,
            row_count: header.require_usize(keywords::NAXIS.indexed(2)?)?,
            field_count,
        })
    }
}

/// The `TTYPEn` name and `TUNITn` unit of a column, which both table forms read the
/// same way: an absent or empty card is `None`.
#[derive(Debug)]
pub(crate) struct ColumnLabels {
    pub(crate) name: Option<String>,
    pub(crate) unit: Option<String>,
}

impl ColumnLabels {
    /// The labels of column `n` (1-based).
    pub(crate) fn read(header: &Header, n: usize) -> Result<Self> {
        let text = |keyword: Keyword| -> Result<Option<String>> {
            Ok(header
                .get_text(keyword)?
                .filter(|value| !value.is_empty())
                .map(str::to_string))
        };
        Ok(Self {
            name: text(keywords::TTYPE.indexed(n)?)?,
            unit: text(keywords::TUNIT.indexed(n)?)?,
        })
    }
}

/// An optional column keyword whose value did not read, kept on its column, as cfitsio
/// and astropy keep the column: a read that needs the keyword fails with
/// [`FitsError::InvalidColumnKeyword`], and the other reads of the table do not.
#[derive(Debug, Clone)]
pub(crate) struct KeywordDefect {
    keyword: Keyword,
    error: Arc<FitsError>,
}

/// A column keyword's value, or its defect.
pub(crate) type Keyed<T> = result::Result<T, KeywordDefect>;

impl KeywordDefect {
    /// The error a read that needs the keyword gives.
    pub(crate) fn error(&self) -> FitsError {
        FitsError::InvalidColumnKeyword {
            keyword: self.keyword,
            source: Arc::clone(&self.error),
        }
    }
}

/// The value `read` gives for `keyword`, or its defect.
pub(crate) fn keyed<T>(keyword: Keyword, read: impl FnOnce() -> Result<T>) -> Keyed<T> {
    read().map_err(|error| KeywordDefect {
        keyword,
        error: Arc::new(error),
    })
}

/// The `TSCALn` and `TZEROn` of a column (§7.2.2, §7.3.2): 1 and 0 when absent.
#[derive(Debug, Clone)]
pub(crate) struct ColumnScaling {
    pub(crate) tscale: Keyed<f64>,
    pub(crate) tzero: Keyed<f64>,
}

impl ColumnScaling {
    pub(crate) const IDENTITY: Self = Self {
        tscale: Ok(1.0),
        tzero: Ok(0.0),
    };

    /// The scaling of column `n` (1-based).
    pub(crate) fn read(header: &Header, n: usize) -> Result<Self> {
        let real = |keyword: Keyword, absent: f64| {
            keyed(keyword, || Ok(header.get_real(keyword)?.unwrap_or(absent)))
        };
        Ok(Self {
            tscale: real(keywords::TSCAL.indexed(n)?, 1.0),
            tzero: real(keywords::TZERO.indexed(n)?, 0.0),
        })
    }
}
