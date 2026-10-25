//! A tile parameter that a header keyword fixes for every tile, or a table column gives
//! for each (§10.1.1, Table 36).

use crate::bintable::element_kind::ElementKind;
use crate::bintable::table_view::TableView;
use crate::fits_error::ExpectedColumn;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// One tile parameter, resolved as cfitsio's `imcomp_get_compressed_image_par` resolves
/// it: the keyword when the header has one, else the column, else neither.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum TileValue<'a, T> {
    Absent,
    Fixed(T),
    PerTile(&'a [T]),
}

impl<'a, T: Copy> TileValue<'a, T> {
    /// The parameter a `keyword` value and a `column` give.
    pub(super) const fn new(keyword: Option<T>, column: Option<&'a [T]>) -> Self {
        match (keyword, column) {
            (Some(value), _) => Self::Fixed(value),
            (None, Some(values)) => Self::PerTile(values),
            (None, None) => Self::Absent,
        }
    }

    /// The parameter, or `fallback` when neither the keyword nor the column gives it.
    #[must_use]
    pub(super) const fn or(self, fallback: Option<T>) -> Self {
        match (self, fallback) {
            (Self::Absent, Some(value)) => Self::Fixed(value),
            (resolved, _) => resolved,
        }
    }

    pub(super) const fn is_absent(&self) -> bool {
        matches!(self, Self::Absent)
    }

    /// The value for tile `tile`.
    pub(super) fn at(&self, tile: usize) -> Option<T> {
        match self {
            Self::Absent => None,
            Self::Fixed(value) => Some(*value),
            Self::PerTile(values) => values.get(tile).copied(),
        }
    }
}

/// A per-tile parameter column of a tiled image's container: its values in row order, or
/// none when the container has no such column.
#[derive(Debug, Default)]
pub(super) struct TileColumn<T> {
    present: bool,
    values: Vec<T>,
}

impl<T> TileColumn<T> {
    /// The values, `None` when the container has no such column.
    pub(super) fn values(&self) -> Option<&[T]> {
        self.present.then_some(self.values.as_slice())
    }

    /// The heap bytes the values take.
    pub(super) const fn heap_bytes(&self) -> usize {
        self.values.capacity() * size_of::<T>()
    }
}

impl TileColumn<f64> {
    /// Refill with the column `name` of `table`, any numeric `TFORM` widened to `f64` as
    /// cfitsio reads it (`TDOUBLE`).
    pub(super) fn read_real(&mut self, table: TableView<'_>, name: &str) -> Result<()> {
        self.read(table, name, ExpectedColumn::Numeric, |kind, bytes| {
            Some(match kind {
                ElementKind::U8 => f64::from(bytes[0]),
                ElementKind::I16 => f64::from(i16::from_be_bytes(bytes.try_into().ok()?)),
                ElementKind::I32 => f64::from(i32::from_be_bytes(bytes.try_into().ok()?)),
                ElementKind::I64 => i64::from_be_bytes(bytes.try_into().ok()?) as f64,
                ElementKind::F32 => f64::from(f32::from_be_bytes(bytes.try_into().ok()?)),
                ElementKind::F64 => f64::from_be_bytes(bytes.try_into().ok()?),
                _ => return None,
            })
        })
    }
}

impl TileColumn<i64> {
    /// Refill with the column `name` of `table`, any integer `TFORM` widened to `i64`.
    pub(super) fn read_integer(&mut self, table: TableView<'_>, name: &str) -> Result<()> {
        self.read(table, name, ExpectedColumn::Integer, |kind, bytes| {
            Some(match kind {
                ElementKind::U8 => i64::from(bytes[0]),
                ElementKind::I16 => i64::from(i16::from_be_bytes(bytes.try_into().ok()?)),
                ElementKind::I32 => i64::from(i32::from_be_bytes(bytes.try_into().ok()?)),
                ElementKind::I64 => i64::from_be_bytes(bytes.try_into().ok()?),
                _ => return None,
            })
        })
    }
}

impl<T> TileColumn<T> {
    /// Refill with the elements of column `name` of `table`, each of the `expected` kinds
    /// `value` decodes from its big-endian bytes, in row order.
    fn read(
        &mut self,
        table: TableView<'_>,
        name: &str,
        expected: ExpectedColumn,
        value: impl Fn(ElementKind, &[u8]) -> Option<T>,
    ) -> Result<()> {
        self.values.clear();
        let Some(index) = table.column_index(name)? else {
            self.present = false;
            return Ok(());
        };
        self.present = true;
        let column = &table.columns[index];
        let kind = column.tform.element();
        let refused = FitsError::ColumnKindMismatch {
            column: index,
            expected,
        };
        // One value a tile: a cell of any other repeat holds no single tile's value.
        if column.tform.repeat() != 1 {
            return Err(refused);
        }
        let size = match kind {
            ElementKind::U8
            | ElementKind::I16
            | ElementKind::I32
            | ElementKind::I64
            | ElementKind::F32
            | ElementKind::F64 => kind.elem_size(),
            _ => return Err(refused),
        };
        self.values
            .reserve_exact(table.extent.row_count * column.tform.repeat());
        for cell in table.cells(column) {
            for element in cell.chunks_exact(size) {
                let Some(element) = value(kind, element) else {
                    return Err(refused);
                };
                self.values.push(element);
            }
        }
        Ok(())
    }
}
