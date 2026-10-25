//! A binary table over borrowed bytes.

use crate::bintable::column::Column;
use crate::bintable::column_reader::ColumnReader;
use crate::bintable::descriptor::PqDescriptor;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::schema::Schema;
use crate::bintable::table_extent::TableExtent;
#[cfg(feature = "compression")]
use crate::bintable::vla_column::VlaColumn;
#[cfg(feature = "compression")]
use crate::column;
use crate::column::ColumnSelector;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// A binary table's columns and extent over a data unit it borrows — the reader's
/// in-place source bytes, a reused selection buffer, or an owned [`Table`]'s
/// storage. Every column decode goes through it.
///
/// [`Table`]: crate::bintable::Table
#[derive(Debug, Clone, Copy)]
pub(crate) struct TableView<'a> {
    pub(crate) columns: &'a [Column],
    pub(crate) extent: TableExtent,
    bytes: &'a [u8],
}

impl<'a> TableView<'a> {
    /// The table `schema` describes, over its data unit `bytes`. Errors when the
    /// bytes end before the heap does.
    pub(crate) fn new(schema: &'a Schema, bytes: &'a [u8]) -> Result<Self> {
        if bytes.len() < schema.extent.heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(TableView {
            columns: &schema.columns,
            extent: schema.extent,
            bytes,
        })
    }

    /// A compacted selection of `row_count` rows of the table `schema` describes, then a
    /// heap that runs to the end of `bytes`.
    pub(crate) fn compact(schema: &'a Schema, row_count: usize, bytes: &'a [u8]) -> Self {
        TableView {
            columns: &schema.columns,
            extent: schema.extent.compacted(row_count, bytes.len()),
            bytes,
        }
    }

    /// A handle to the variable-length column named `name`, or `None` when the table
    /// has no such column — the optional per-tile source and mask columns of a
    /// compressed image are all addressed this way.
    #[cfg(feature = "compression")]
    pub(crate) fn optional_vla_column(&self, name: &str) -> Result<Option<VlaColumn<'a>>> {
        match self.column_index(name)? {
            Some(index) => Ok(Some(self.column(index)?.vla_column()?)),
            None => Ok(None),
        }
    }

    #[cfg(feature = "compression")]
    pub(crate) fn column_index(&self, name: &str) -> Result<Option<usize>> {
        column::index_of(self.columns, name)
    }

    /// The heap bytes `descriptor`'s array of `element_kind` spans; `None` when it runs
    /// past the heap.
    pub(crate) fn pq_payload(
        &self,
        descriptor: PqDescriptor,
        element_kind: ElementKind,
    ) -> Option<&'a [u8]> {
        let range =
            descriptor.heap_range(element_kind, self.extent.heap_offset, self.extent.heap_end)?;
        Some(&self.bytes[range])
    }

    pub(crate) fn column<'n>(
        &self,
        column: impl Into<ColumnSelector<'n>>,
    ) -> Result<ColumnReader<'a>> {
        let index = column.into().resolve(self.columns)?;
        Ok(ColumnReader::new(*self, index))
    }

    /// The raw bytes of column `col` in row `r`.
    pub(crate) fn cell(&self, col: &Column, r: usize) -> &'a [u8] {
        let start = r * self.extent.row_len + col.byte_offset;
        &self.bytes[start..start + col.tform.byte_width()]
    }

    /// The descriptor of array column `col` in row `row`.
    pub(crate) fn pq_descriptor(&self, col: &Column, row: usize) -> Result<PqDescriptor> {
        let width = col
            .tform
            .descriptor_width()
            .expect("a descriptor is read from an array column");
        if col.tform.repeat() == 0 {
            Ok(PqDescriptor::EMPTY)
        } else {
            PqDescriptor::decode(self.cell(col, row), width)
        }
    }

    pub(crate) fn cells(self, col: &'a Column) -> impl ExactSizeIterator<Item = &'a [u8]> + 'a {
        // The width once for the column, not from the `TFORMn` again for each row.
        let width = col.tform.byte_width();
        let (bytes, row_len, offset) = (self.bytes, self.extent.row_len, col.byte_offset);
        (0..self.extent.row_count).map(move |row| {
            let start = row * row_len + offset;
            &bytes[start..start + width]
        })
    }
}

#[cfg(all(test, feature = "compression"))]
pub(crate) mod internals {
    use crate::bintable::table_view::TableView;

    impl<'a> TableView<'a> {
        /// The fixed-width main table (`row_count × NAXIS1` bytes), excluding the heap.
        pub(crate) fn main_rows(&self) -> &'a [u8] {
            &self.bytes[..self.extent.row_count * self.extent.row_len]
        }
    }
}
