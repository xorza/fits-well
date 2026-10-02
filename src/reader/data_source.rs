//! The reader's byte source, paired with the staging buffer a seeking source reads
//! through.

use std::ops::Range;

use crate::allocation;
use crate::data::shape_product;
use crate::data::validate_image_region;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::image_geometry::ImageGeometry;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;
use crate::table_impl::column::Column;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::descriptor::PqDescriptor;
use crate::table_impl::table_schema::TableSchema;
#[cfg(feature = "compression")]
use crate::table_impl::table_view::TableView;
use crate::table_impl::tform_kind::TformKind;

/// A [`Source`] and its staging buffer. Kept apart from the HDU records so a read can
/// borrow a header or schema while it fetches bytes.
#[derive(Debug)]
pub(crate) struct DataSource<S> {
    source: S,
    /// Reused staging buffer for the seeking-source reads: a stream source copies
    /// each fetched range here (an in-memory source borrows instead, so this stays
    /// empty). Grows once to the largest range fetched, then holds.
    scratch: Vec<u8>,
}

/// The rows of a table a selection copies, in output order.
#[derive(Debug, Clone)]
pub(crate) enum TableRows<'a> {
    Range(Range<usize>),
    #[cfg_attr(
        not(feature = "compression"),
        expect(dead_code, reason = "only tile reads pick rows")
    )]
    Picked(&'a [usize]),
}

impl TableRows<'_> {
    fn len(&self) -> usize {
        match self {
            TableRows::Range(rows) => rows.len(),
            TableRows::Picked(rows) => rows.len(),
        }
    }
}

impl<S: Source> DataSource<S> {
    pub(crate) const fn new(source: S, scratch: Vec<u8>) -> DataSource<S> {
        DataSource { source, scratch }
    }

    pub(crate) fn into_source(self) -> S {
        self.source
    }

    /// The `len` bytes at `offset`, borrowed — see [`Source::slice`].
    pub(crate) fn slice(&mut self, offset: u64, len: usize) -> Result<&[u8]> {
        self.source.slice(offset, len, &mut self.scratch)
    }

    /// The `len` bytes at `offset` in a fresh owned buffer.
    pub(crate) fn read_owned(&mut self, offset: u64, len: usize) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.source.read_append(offset, len, &mut bytes)?;
        Ok(bytes)
    }

    /// The binary table `hdu` holds, viewed in place over the source (or this
    /// source's staging buffer).
    #[cfg(feature = "compression")]
    pub(crate) fn table<'a>(&'a mut self, hdu: &'a Hdu) -> Result<TableView<'a>> {
        let schema = hdu.table_schema()?;
        let len = usize::try_from(hdu.data_bytes).map_err(|_| FitsError::DataUnitTooLarge {
            bytes: hdu.data_bytes,
        })?;
        TableView::new(schema, self.slice(hdu.data_offset, len)?)
    }

    /// The rows `rows` of the binary table `hdu` holds, compacted into `out` — see
    /// [`DataSource::select_table`].
    #[cfg(feature = "compression")]
    pub(crate) fn table_rows<'a>(
        &mut self,
        hdu: &'a Hdu,
        rows: &[usize],
        out: &'a mut Vec<u8>,
    ) -> Result<TableView<'a>> {
        let schema = hdu.table_schema()?;
        self.select_table(hdu.data_offset, schema, TableRows::Picked(rows), None, out)?;
        Ok(TableView::compact(
            &schema.columns,
            rows.len(),
            schema.row_len,
            out,
        ))
    }

    /// Write the shape `ranges` selects from the plain image `hdu` holds to `shape`,
    /// and its stored big-endian samples to `bytes`, reading only the selected runs.
    pub(crate) fn plain_section(
        &mut self,
        hdu: &Hdu,
        image: &ImageGeometry,
        ranges: &[Range<usize>],
        shape: &mut Vec<usize>,
        bytes: &mut Vec<u8>,
    ) -> Result<()> {
        validate_image_region(ranges, &image.shape, shape)?;
        let element_size = image.bitpix.elem_size();
        let nbytes = shape_product(shape)?
            .checked_mul(element_size)
            .ok_or(FitsError::DataUnitOverflow)?;
        bytes.clear();
        allocation::try_reserve(bytes, nbytes)?;
        visit_image_region_runs(&image.shape, ranges, shape, element_size, |run| {
            self.source
                .read_append(offset_at(hdu.data_offset, run.offset)?, run.len, bytes)
        })?;
        debug_assert_eq!(bytes.len(), nbytes);
        Ok(())
    }

    /// Replace `out` with `rows` of the table `schema` describes at `data_offset`,
    /// followed by the heap arrays those rows address, packed in row order — the data
    /// unit of a table holding just those rows. A `P`/`Q` column that `columns` does
    /// not select keeps an empty descriptor and no heap bytes. Reads nothing else.
    pub(crate) fn select_table(
        &mut self,
        data_offset: u64,
        schema: &TableSchema,
        rows: TableRows<'_>,
        columns: Option<&[bool]>,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        debug_assert!(columns.is_none_or(|columns| columns.len() == schema.columns.len()));
        out.clear();
        let row_count = rows.len();
        let main_len = row_count
            .checked_mul(schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        match rows {
            TableRows::Range(rows) => {
                validate_row_range(&rows, schema.nrows)?;
                if !rows.is_empty() {
                    let offset = row_offset(data_offset, rows.start, schema.row_len)?;
                    self.source.read_append(offset, main_len, out)?;
                }
            }
            TableRows::Picked(rows) => {
                for &row in rows {
                    validate_row(row, schema.nrows)?;
                    let offset = row_offset(data_offset, row, schema.row_len)?;
                    self.source.read_append(offset, schema.row_len, out)?;
                }
            }
        }
        for row in 0..row_count {
            for (index, column) in schema.columns.iter().enumerate() {
                let wide = match column.tform.kind {
                    TformKind::ArrayDesc32 => false,
                    TformKind::ArrayDesc64 => true,
                    _ => continue,
                };
                if column.tform.repeat == 0 {
                    continue;
                }
                let slot = row * schema.row_len + column.byte_offset;
                let slot = slot..slot + column.tform.byte_width();
                if columns.is_some_and(|columns| !columns[index]) {
                    write_pq_descriptor(&mut out[slot], wide, 0, 0)?;
                    continue;
                }
                let span = VlaCellSpan::new(schema, column, &out[slot.clone()], wide)?;
                let heap_offset = out.len() - main_len;
                self.source.read_append(
                    offset_at(data_offset, span.heap_range.start)?,
                    span.heap_range.len(),
                    out,
                )?;
                write_pq_descriptor(&mut out[slot], wide, span.count as u64, heap_offset as u64)?;
            }
        }
        Ok(())
    }

    /// Decode the cell at `row` of column `column` without reading other rows.
    pub(crate) fn read_table_cell(
        &mut self,
        data_offset: u64,
        schema: &TableSchema,
        row: usize,
        column: usize,
    ) -> Result<ColumnData> {
        validate_row(row, schema.nrows)?;
        let column = &schema.columns[column];
        let cell_offset = offset_at(
            row_offset(data_offset, row, schema.row_len)?,
            column.byte_offset,
        )?;
        let width = column.tform.byte_width();
        let wide = match column.tform.kind {
            TformKind::ArrayDesc32 => false,
            TformKind::ArrayDesc64 => true,
            _ => return Ok(column.decode_cell(self.slice(cell_offset, width)?)),
        };
        if column.tform.repeat == 0 {
            return column.decode_vla_cell(&[], 0);
        }
        let span = VlaCellSpan::new(schema, column, self.slice(cell_offset, width)?, wide)?;
        let bytes = self.slice(
            offset_at(data_offset, span.heap_range.start)?,
            span.heap_range.len(),
        )?;
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
    fn new(
        schema: &TableSchema,
        column: &Column,
        descriptor_bytes: &[u8],
        wide: bool,
    ) -> Result<VlaCellSpan> {
        let descriptor = PqDescriptor::decode(descriptor_bytes, wide)?;
        let element_type = column
            .tform
            .vla_elem
            .expect("validated VLA format carries an element type");
        Ok(VlaCellSpan {
            count: descriptor.count,
            heap_range: descriptor.heap_range(element_type, schema.heap_offset, schema.heap_end)?,
        })
    }
}

/// `base + delta`, rejecting a sum that cannot address a source byte.
fn offset_at(base: u64, delta: usize) -> Result<u64> {
    u64::try_from(delta)
        .ok()
        .and_then(|delta| base.checked_add(delta))
        .ok_or(FitsError::DataUnitOverflow)
}

fn validate_row(row: usize, len: usize) -> Result<()> {
    if row >= len {
        return Err(FitsError::RowRangeOutOfBounds {
            start: row,
            end: row.saturating_add(1),
            len,
        });
    }
    Ok(())
}

/// Byte offset of table `row`'s first byte, in a data unit starting at `base`.
///
/// The row product is taken in `u64` rather than `usize` so a 32-bit host rejects
/// an out-of-range row instead of wrapping into a plausible-looking offset.
fn row_offset(base: u64, row: usize, row_len: usize) -> Result<u64> {
    u64::try_from(row)
        .ok()
        .and_then(|row| row.checked_mul(row_len as u64))
        .and_then(|within| base.checked_add(within))
        .ok_or(FitsError::DataUnitOverflow)
}

pub(crate) fn validate_row_range(rows: &Range<usize>, len: usize) -> Result<()> {
    if rows.start > rows.end || rows.end > len {
        return Err(FitsError::RowRangeOutOfBounds {
            start: rows.start,
            end: rows.end,
            len,
        });
    }
    Ok(())
}

#[derive(Debug, Clone, Copy)]
struct ByteRun {
    offset: usize,
    len: usize,
}

fn visit_image_region_runs(
    shape: &[usize],
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    element_size: usize,
    mut visit: impl FnMut(ByteRun) -> Result<()>,
) -> Result<()> {
    debug_assert_eq!(shape.len(), ranges.len());
    debug_assert_eq!(shape.len(), selected_shape.len());
    if selected_shape.is_empty() || selected_shape.contains(&0) {
        return Ok(());
    }
    let row_count = selected_shape[1..]
        .iter()
        .try_fold(1usize, |count, &len| count.checked_mul(len))
        .ok_or(FitsError::DataUnitOverflow)?;
    let row_bytes = selected_shape[0]
        .checked_mul(element_size)
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut pending: Option<ByteRun> = None;
    for row_index in 0..row_count {
        let mut remainder = row_index;
        let mut element_offset = ranges[0].start;
        let mut stride = shape[0];
        for axis in 1..shape.len() {
            let coordinate = ranges[axis].start + remainder % selected_shape[axis];
            remainder /= selected_shape[axis];
            element_offset = coordinate
                .checked_mul(stride)
                .and_then(|value| element_offset.checked_add(value))
                .ok_or(FitsError::DataUnitOverflow)?;
            if axis + 1 < shape.len() {
                stride = stride
                    .checked_mul(shape[axis])
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
        }
        let next = ByteRun {
            offset: element_offset
                .checked_mul(element_size)
                .ok_or(FitsError::DataUnitOverflow)?,
            len: row_bytes,
        };
        if let Some(previous) = pending.as_mut() {
            let end = previous
                .offset
                .checked_add(previous.len)
                .ok_or(FitsError::DataUnitOverflow)?;
            if end == next.offset {
                previous.len = previous
                    .len
                    .checked_add(next.len)
                    .ok_or(FitsError::DataUnitOverflow)?;
                continue;
            }
        }
        if let Some(previous) = pending.replace(next) {
            visit(previous)?;
        }
    }
    if let Some(run) = pending {
        visit(run)?;
    }
    Ok(())
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::reader::data_source::DataSource;

    /// The staging buffer a seeking source reads through.
    pub(crate) fn scratch<S>(data: &DataSource<S>) -> &Vec<u8> {
        &data.scratch
    }
}
