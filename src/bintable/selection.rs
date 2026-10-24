//! A binary table's row and column selection: the data unit of a table that holds just
//! the selected rows, read through a [`Fetch`] from the file or from decoded tiles.

use std::ops::Range;

use crate::bintable::column::Column;
use crate::bintable::column_data::ColumnData;
use crate::bintable::column_reader::ColumnReader;
use crate::bintable::descriptor::PqDescriptor;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::column::{ColumnSelector, Named, index_of};
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::ragged::Ragged;

/// Raw data for one selected table column over a row range.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SelectedData {
    Fixed(ColumnData),
    Variable(Ragged<ColumnData>),
}

/// One decoded column in a ranged table selection.
#[derive(Debug, Clone)]
pub struct SelectedColumn {
    pub descriptor: Column,
    pub data: SelectedData,
}

impl SelectedColumn {
    fn new(column: &ColumnReader<'_>) -> Result<Self> {
        let descriptor = column.descriptor().clone();
        let data = if descriptor.tform.descriptor_width().is_some() {
            SelectedData::Variable(column.vla()?)
        } else {
            SelectedData::Fixed(column.raw()?)
        };
        Ok(Self { descriptor, data })
    }

    /// Decode `column` into this column's own buffers, which it reuses when they hold
    /// the same kind of data.
    fn refill(&mut self, column: &ColumnReader<'_>) -> Result<()> {
        self.descriptor.clone_from(column.descriptor());
        let variable = self.descriptor.tform.descriptor_width().is_some();
        match &mut self.data {
            SelectedData::Variable(rows) if variable => column.vla_into(rows),
            SelectedData::Fixed(values) if !variable => column.raw_into(values),
            _ => {
                *self = Self::new(column)?;
                Ok(())
            }
        }
    }
}

impl Named for SelectedColumn {
    fn name(&self) -> Option<&str> {
        self.descriptor.name()
    }
}

/// Selected columns decoded over a zero-based half-open row range.
#[derive(Debug, Clone)]
pub struct Selection {
    pub rows: Range<usize>,
    pub columns: Vec<SelectedColumn>,
}

impl Selection {
    /// Refill this selection with the `columns` of `table`, which holds the compacted
    /// `rows`, reusing the buffers of the columns it held.
    pub(crate) fn refill(
        &mut self,
        table: TableView<'_>,
        rows: Range<usize>,
        columns: &[usize],
    ) -> Result<()> {
        self.rows = rows;
        self.columns.truncate(columns.len());
        for (pos, &index) in columns.iter().enumerate() {
            let column = table.column(index)?;
            if let Some(selected) = self.columns.get_mut(pos) {
                selected.refill(&column)?;
            } else {
                self.columns.push(SelectedColumn::new(&column)?);
            }
        }
        Ok(())
    }

    /// The selected column that `column` names: an index of the table's columns, or a
    /// `TTYPEn` name compared case-insensitively. `None` when the selection does not
    /// hold it.
    pub fn column<'c>(&self, column: impl Into<ColumnSelector<'c>>) -> Option<&SelectedColumn> {
        match column.into() {
            ColumnSelector::Index(index) => self
                .columns
                .iter()
                .find(|selected| selected.descriptor.number == index + 1),
            ColumnSelector::Name(name) => {
                index_of(&self.columns, name).map(|pos| &self.columns[pos])
            }
        }
    }
}

/// Where a selection reads a table's data unit from: the file, through the reader, or
/// tiles decoded into memory. Ranges are relative to the start of the data unit.
pub(crate) trait Fetch {
    /// Append the bytes of `range` to `out`.
    fn append(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()>;

    /// The bytes of `range`, borrowed.
    fn bytes(&mut self, range: Range<usize>) -> Result<&[u8]>;
}

impl Fetch for &[u8] {
    fn append(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()> {
        out.extend_from_slice(self.get(range).ok_or(FitsError::UnexpectedEof)?);
        Ok(())
    }

    fn bytes(&mut self, range: Range<usize>) -> Result<&[u8]> {
        self.get(range).ok_or(FitsError::UnexpectedEof)
    }
}

/// Appends through a [`Fetch`], with each range that starts where the last one ends held
/// back and joined to it, so a run of adjacent ranges costs one fetch.
#[derive(Debug)]
struct Merged<'f, F> {
    fetch: &'f mut F,
    pending: Option<Range<usize>>,
}

impl<'f, F: Fetch> Merged<'f, F> {
    const fn new(fetch: &'f mut F) -> Self {
        Self {
            fetch,
            pending: None,
        }
    }

    /// Append the bytes of `range` to `out`, now or with the ranges it joins.
    fn push(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()> {
        if range.is_empty() {
            return Ok(());
        }
        match &mut self.pending {
            Some(pending) if pending.end == range.start => pending.end = range.end,
            _ => {
                self.flush(out)?;
                self.pending = Some(range);
            }
        }
        Ok(())
    }

    /// The length `out` will have once the held-back range lands: where the next pushed
    /// byte goes.
    fn next_offset(&self, out: &[u8]) -> usize {
        out.len() + self.pending.as_ref().map_or(0, ExactSizeIterator::len)
    }

    /// Append the held-back range.
    fn flush(&mut self, out: &mut Vec<u8>) -> Result<()> {
        match self.pending.take() {
            Some(pending) => self.fetch.append(pending, out),
            None => Ok(()),
        }
    }
}

/// The rows of a table a selection copies, in output order.
#[derive(Debug, Clone)]
pub(crate) enum TableRows<'a> {
    Range(Range<usize>),
    #[cfg_attr(
        not(any(test, feature = "compression")),
        expect(dead_code, reason = "only tile reads and the tests pick rows")
    )]
    Picked(&'a [usize]),
}

impl TableRows<'_> {
    pub(crate) fn len(&self) -> usize {
        match self {
            TableRows::Range(rows) => rows.len(),
            TableRows::Picked(rows) => rows.len(),
        }
    }
}

/// The rows and columns of the table `schema` describes that a selection copies; every
/// column when `columns` is `None`.
#[derive(Debug, Clone)]
pub(crate) struct TableSelection<'a> {
    pub(crate) schema: &'a Schema,
    pub(crate) rows: TableRows<'a>,
    pub(crate) columns: Option<&'a [bool]>,
}

impl TableSelection<'_> {
    /// The bytes [`Self::copy_into`] writes: the selected rows, and the heap arrays of
    /// their selected columns, which the descriptors in `main`, the table's whole main
    /// table, address.
    #[cfg(feature = "compression")]
    pub(crate) fn copied_len(&self, main: &[u8]) -> Result<usize> {
        let Self {
            schema,
            rows,
            columns,
        } = self;
        let row_len = schema.extent.row_len;
        let mut len = rows
            .len()
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let mut add = |row: usize| -> Result<()> {
            validate_row(row, schema.extent.row_count)?;
            for (index, column) in schema.columns.iter().enumerate() {
                if column.tform.descriptor_width().is_none()
                    || column.tform.repeat() == 0
                    || columns.is_some_and(|columns| !columns[index])
                {
                    continue;
                }
                let slot = row * row_len + column.byte_offset;
                let descriptor = &main[slot..slot + column.tform.byte_width()];
                let span = VlaCellSpan::new(schema, column, descriptor)?;
                len = len
                    .checked_add(span.heap_range.len())
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
            Ok(())
        };
        match rows {
            TableRows::Range(rows) => {
                validate_row_range(rows, schema.extent.row_count)?;
                rows.clone().try_for_each(&mut add)?;
            }
            TableRows::Picked(rows) => rows.iter().try_for_each(|&row| add(row))?,
        }
        Ok(len)
    }

    /// Replace `out` with the selected rows, followed by the heap arrays those rows
    /// address, packed in row order — the data unit of a table holding just those rows.
    /// A `P`/`Q` column the selection leaves out keeps an empty descriptor and no heap
    /// bytes. Fetches nothing else.
    pub(crate) fn copy_into(self, fetch: &mut impl Fetch, out: &mut Vec<u8>) -> Result<()> {
        let Self {
            schema,
            rows,
            columns,
        } = self;
        debug_assert!(columns.is_none_or(|columns| columns.len() == schema.columns.len()));
        let row_len = schema.extent.row_len;
        out.clear();
        let row_count = rows.len();
        let main_len = row_count
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let mut reads = Merged::new(fetch);
        match rows {
            TableRows::Range(rows) => {
                validate_row_range(&rows, schema.extent.row_count)?;
                reads.push(rows.start * row_len..rows.end * row_len, out)?;
            }
            TableRows::Picked(rows) => {
                for &row in rows {
                    validate_row(row, schema.extent.row_count)?;
                    reads.push(row * row_len..(row + 1) * row_len, out)?;
                }
            }
        }
        reads.flush(out)?;
        for row in 0..row_count {
            for (index, column) in schema.columns.iter().enumerate() {
                let Some(width) = column.tform.descriptor_width() else {
                    continue;
                };
                if column.tform.repeat() == 0 {
                    continue;
                }
                let slot = row * row_len + column.byte_offset;
                let slot = slot..slot + column.tform.byte_width();
                if columns.is_some_and(|columns| !columns[index]) {
                    PqDescriptor::EMPTY.write(width, &mut out[slot])?;
                    continue;
                }
                let span = VlaCellSpan::new(schema, column, &out[slot.clone()])?;
                let heap_offset = reads.next_offset(out) - main_len;
                reads.push(span.heap_range, out)?;
                PqDescriptor {
                    count: span.count,
                    offset: heap_offset,
                }
                .write(width, &mut out[slot])?;
            }
        }
        reads.flush(out)
    }
}

/// One cell of the table `schema` describes.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableCell<'a> {
    pub(crate) schema: &'a Schema,
    pub(crate) row: usize,
    pub(crate) column: usize,
}

impl TableCell<'_> {
    /// Decode the cell, fetching its bytes and, for a `P`/`Q` column, its heap array, and
    /// nothing else.
    pub(crate) fn read(self, fetch: &mut impl Fetch) -> Result<ColumnData> {
        let Self {
            schema,
            row,
            column,
        } = self;
        validate_row(row, schema.extent.row_count)?;
        let column = &schema.columns[column];
        let start = row * schema.extent.row_len + column.byte_offset;
        let cell = fetch.bytes(start..start + column.tform.byte_width())?;
        if column.tform.descriptor_width().is_none() {
            return Ok(column.decode_cell(cell));
        }
        if column.tform.repeat() == 0 {
            return column.decode_vla_cell(&[], 0);
        }
        let span = VlaCellSpan::new(schema, column, cell)?;
        let bytes = fetch.bytes(span.heap_range)?;
        column.decode_vla_cell(bytes, span.count)
    }
}

/// Where a table cell's `P`/`Q` descriptor points: its element count and the heap
/// bytes it spans, relative to the data unit.
#[derive(Debug)]
struct VlaCellSpan {
    count: usize,
    heap_range: Range<usize>,
}

impl VlaCellSpan {
    /// The span the descriptor `descriptor_bytes` of the array `column` gives.
    fn new(schema: &Schema, column: &Column, descriptor_bytes: &[u8]) -> Result<Self> {
        let width = column
            .tform
            .descriptor_width()
            .expect("a span is read from an array column");
        let descriptor = PqDescriptor::decode(descriptor_bytes, width)?;
        Ok(Self {
            count: descriptor.count,
            heap_range: descriptor.heap_range(
                column.tform.element(),
                schema.extent.heap_offset,
                schema.extent.heap_end,
            )?,
        })
    }
}

const fn validate_row(row: usize, len: usize) -> Result<()> {
    if row >= len {
        return Err(FitsError::RowRangeOutOfBounds {
            start: row,
            end: row.saturating_add(1),
            len,
        });
    }
    Ok(())
}

pub(crate) const fn validate_row_range(rows: &Range<usize>, len: usize) -> Result<()> {
    if rows.start > rows.end || rows.end > len {
        return Err(FitsError::RowRangeOutOfBounds {
            start: rows.start,
            end: rows.end,
            len,
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::ops::Range;

    use crate::bintable::internals::table_header;
    use crate::bintable::schema::Schema;
    use crate::bintable::selection::{Fetch, TableRows, TableSelection};
    use crate::fits_error::Result;

    /// A data unit in memory that records each range a selection fetches.
    #[derive(Debug)]
    struct Recorded<'a> {
        bytes: &'a [u8],
        appends: Vec<Range<usize>>,
    }

    impl Fetch for Recorded<'_> {
        fn append(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()> {
            self.appends.push(range.clone());
            self.bytes.append(range, out)
        }

        fn bytes(&mut self, range: Range<usize>) -> Result<&[u8]> {
            self.bytes.bytes(range)
        }
    }

    /// Rows that follow one another are fetched as one: rows 0, 1 and 3 of four rows of 2
    /// bytes are the fetches 0..4 and 6..8.
    #[test]
    fn adjacent_rows_are_fetched_together() {
        let schema = Schema::parse(&table_header(2, 4, &["2B"])).unwrap();
        let bytes: Vec<u8> = (0..8).collect();
        let mut fetch = Recorded {
            bytes: &bytes,
            appends: Vec::new(),
        };
        let mut out = Vec::new();
        let selection = TableSelection {
            schema: &schema,
            rows: TableRows::Picked(&[0, 1, 3]),
            columns: None,
        };
        selection.copy_into(&mut fetch, &mut out).unwrap();
        assert_eq!(out, [0, 1, 2, 3, 6, 7]);
        assert_eq!(fetch.appends, [0..4, 6..8]);
    }
}
