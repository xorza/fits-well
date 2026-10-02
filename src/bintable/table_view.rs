//! A binary table over borrowed bytes.

use crate::bintable::BinTableMetadata;
use crate::bintable::column::Column;
use crate::bintable::column_reader::ColumnReader;
use crate::bintable::descriptor::PqDescriptor;
use crate::bintable::table_schema::TableSchema;
use crate::bintable::tform_kind::TformKind;
#[cfg(feature = "compression")]
use crate::bintable::vla_column::VlaColumn;
use crate::column;
use crate::error::FitsError;
use crate::error::Result;

/// A binary table's columns and extent over a data unit it borrows — the reader's
/// in-place source bytes, a reused selection buffer, or an owned [`BinTable`]'s
/// storage. Every column decode goes through it.
///
/// [`BinTable`]: crate::bintable::BinTable
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableView<'a> {
    pub(crate) columns: &'a [Column],
    pub(crate) nrows: usize,
    /// Byte width of one row (`NAXIS1`).
    pub(crate) row_len: usize,
    pub(crate) heap_offset: usize,
    pub(crate) heap_end: usize,
    bytes: &'a [u8],
}

impl<'a> TableView<'a> {
    /// The table `schema` describes, over its data unit `bytes`. Errors when the
    /// bytes end before the heap does.
    pub(crate) fn new(schema: &'a TableSchema, bytes: &'a [u8]) -> Result<TableView<'a>> {
        if bytes.len() < schema.heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(TableView {
            columns: &schema.columns,
            nrows: schema.nrows,
            row_len: schema.row_len,
            heap_offset: schema.heap_offset,
            heap_end: schema.heap_end,
            bytes,
        })
    }

    /// A compacted row selection: `nrows` rows of `row_len` bytes, then a heap that
    /// runs to the end of `bytes`.
    ///
    /// # Panics
    /// When `bytes` is shorter than the rows — the selection builds both together.
    pub(crate) fn compact(
        columns: &'a [Column],
        nrows: usize,
        row_len: usize,
        bytes: &'a [u8],
    ) -> TableView<'a> {
        let heap_offset = nrows * row_len;
        assert!(
            heap_offset <= bytes.len(),
            "a compacted selection holds its rows"
        );
        TableView {
            columns,
            nrows,
            row_len,
            heap_offset,
            heap_end: bytes.len(),
            bytes,
        }
    }

    pub(crate) const fn metadata(&self) -> BinTableMetadata<'a> {
        BinTableMetadata {
            nrows: self.nrows,
            columns: self.columns,
        }
    }

    /// The fixed-width main table (`nrows × NAXIS1` bytes), excluding the heap.
    #[cfg(feature = "compression")]
    pub(crate) fn raw_rows(&self) -> &'a [u8] {
        &self.bytes[..self.nrows * self.row_len]
    }

    /// A handle to the variable-length column named `name`, or `None` when the table
    /// has no such column — the optional per-tile source and mask columns of a
    /// compressed image are all addressed this way.
    #[cfg(feature = "compression")]
    pub(crate) fn optional_vla_column(&self, name: &str) -> Result<Option<VlaColumn<'a>>> {
        match self.column_index(name) {
            Some(index) => Ok(Some(self.column_by_idx(index)?.vla_column()?)),
            None => Ok(None),
        }
    }

    #[cfg(feature = "compression")]
    pub(crate) fn column_index(&self, name: &str) -> Option<usize> {
        column::index_of(self.columns, name)
    }

    pub(crate) fn pq_payload(
        &self,
        descriptor: PqDescriptor,
        element_kind: TformKind,
    ) -> Result<&'a [u8]> {
        let range = descriptor.heap_range(element_kind, self.heap_offset, self.heap_end)?;
        Ok(&self.bytes[range])
    }

    pub(crate) fn column_by_idx(&self, index: usize) -> Result<ColumnReader<'a>> {
        column::validate_index(index, self.columns.len())?;
        Ok(ColumnReader::new(*self, index))
    }

    pub(crate) fn column_by_name(&self, name: &str) -> Result<ColumnReader<'a>> {
        let index = column::checked_index_of(self.columns, name)?;
        Ok(ColumnReader::new(*self, index))
    }

    /// The raw bytes of column `col` in row `r`.
    pub(crate) fn cell(&self, col: &Column, r: usize) -> &'a [u8] {
        let start = r * self.row_len + col.byte_offset;
        &self.bytes[start..start + col.tform.byte_width()]
    }

    pub(crate) fn pq_descriptor(&self, col: &Column, row: usize) -> Result<PqDescriptor> {
        if col.tform.repeat == 0 {
            Ok(PqDescriptor::EMPTY)
        } else {
            PqDescriptor::decode(
                self.cell(col, row),
                col.tform.kind == TformKind::ArrayDesc64,
            )
        }
    }

    pub(crate) fn cells(self, col: &'a Column) -> impl ExactSizeIterator<Item = &'a [u8]> + 'a {
        (0..self.nrows).map(move |row| self.cell(col, row))
    }
}
