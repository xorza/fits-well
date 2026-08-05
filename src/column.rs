//! Addressing a table column by name or index, shared by both table forms.
//!
//! §6.7 (binary) and §7.2.2 (ASCII) give the two forms the same rule: a column is
//! named by its `TTYPEn` value, compared without regard to case. The two table
//! models are otherwise unrelated, so the rule lives here rather than being spelled
//! once per model — and once more in the reader, which resolves names against a
//! schema parsed without ever reading a table.

use crate::error::FitsError;
use crate::error::Result;

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
pub(crate) fn validate_index(index: usize, len: usize) -> Result<()> {
    if index >= len {
        return Err(FitsError::ColumnIndexOutOfBounds { index, len });
    }
    Ok(())
}
