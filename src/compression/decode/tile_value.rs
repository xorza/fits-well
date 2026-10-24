//! A tile parameter that a header keyword fixes for every tile, or a table column gives
//! for each (§10.1.1, Table 36).

use crate::bintable::column_data::ColumnData;
use crate::bintable::table_view::TableView;
use crate::fits_error::ExpectedColumn;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// One tile parameter, resolved as cfitsio's `imcomp_get_compressed_image_par` resolves
/// it: the keyword when the header has one, else the column, else neither.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum TileValue<T> {
    Absent,
    Fixed(T),
    PerTile(Vec<T>),
}

impl<T: Copy> TileValue<T> {
    /// The parameter a `keyword` value and a `column` give.
    pub(super) fn new(keyword: Option<T>, column: Option<Vec<T>>) -> Self {
        match (keyword, column) {
            (Some(value), _) => Self::Fixed(value),
            (None, Some(values)) => Self::PerTile(values),
            (None, None) => Self::Absent,
        }
    }

    /// The parameter, or `fallback` when neither the keyword nor the column gives it.
    #[must_use]
    pub(super) fn or(self, fallback: Option<T>) -> Self {
        match (self, fallback) {
            (Self::Absent, Some(value)) => Self::Fixed(value),
            (resolved, _) => resolved,
        }
    }

    pub(super) const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    /// The value for the tile of table row `row`.
    pub(super) fn at(&self, row: usize) -> Option<T> {
        match self {
            Self::Absent => None,
            Self::Fixed(value) => Some(*value),
            Self::PerTile(values) => values.get(row).copied(),
        }
    }
}

/// The column `name` of `table`, any numeric `TFORM` widened to `f64` as cfitsio reads it
/// (`TDOUBLE`), or `None` when the table has no such column.
pub(super) fn real_column(table: TableView<'_>, name: &str) -> Result<Option<Vec<f64>>> {
    let Some(index) = table.column_index(name) else {
        return Ok(None);
    };
    let values = match table.column(index)?.raw()? {
        ColumnData::U8(v) => v.into_iter().map(f64::from).collect(),
        ColumnData::I16(v) => v.into_iter().map(f64::from).collect(),
        ColumnData::I32(v) => v.into_iter().map(f64::from).collect(),
        ColumnData::I64(v) => v.into_iter().map(|v| v as f64).collect(),
        ColumnData::F32(v) => v.into_iter().map(f64::from).collect(),
        ColumnData::F64(v) => v,
        _ => {
            return Err(FitsError::ColumnKindMismatch {
                column: index,
                expected: ExpectedColumn::Numeric,
            });
        }
    };
    Ok(Some(values))
}

/// The column `name` of `table`, any integer `TFORM` widened to `i64`, or `None` when the
/// table has no such column.
pub(super) fn integer_column(table: TableView<'_>, name: &str) -> Result<Option<Vec<i64>>> {
    let Some(index) = table.column_index(name) else {
        return Ok(None);
    };
    let values = match table.column(index)?.raw()? {
        ColumnData::U8(v) => v.into_iter().map(i64::from).collect(),
        ColumnData::I16(v) => v.into_iter().map(i64::from).collect(),
        ColumnData::I32(v) => v.into_iter().map(i64::from).collect(),
        ColumnData::I64(v) => v,
        _ => {
            return Err(FitsError::ColumnKindMismatch {
                column: index,
                expected: ExpectedColumn::Integer,
            });
        }
    };
    Ok(Some(values))
}
