//! Addressing a table column by name or index, shared by both table forms.
//!
//! §6.7 (binary) and §7.2.2 (ASCII) give the two forms the same rule: a column is
//! named by its `TTYPEn` value, compared without regard to case. The two table
//! models are otherwise unrelated, so the rule lives here rather than being spelled
//! once per model — and once more in the reader, which resolves names against a
//! schema parsed without ever reading a table.

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
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

/// The `TSCALn` and `TZEROn` of a column (§7.2.2, §7.3.2): 1 and 0 when absent.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ColumnScaling {
    pub(crate) tscale: f64,
    pub(crate) tzero: f64,
}

impl ColumnScaling {
    pub(crate) const IDENTITY: Self = Self {
        tscale: 1.0,
        tzero: 0.0,
    };

    /// The scaling of column `n` (1-based).
    pub(crate) fn read(header: &Header, n: usize) -> Result<Self> {
        Ok(Self {
            tscale: header.get_real(keywords::TSCAL.indexed(n)?)?.unwrap_or(1.0),
            tzero: header.get_real(keywords::TZERO.indexed(n)?)?.unwrap_or(0.0),
        })
    }
}
