//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. That
//! structure parses into a [`TableSchema`] of [`Column`] descriptors; decoding goes
//! through a [`ColumnReader`] (from [`BinTable::column_by_idx`] /
//! [`BinTable::column_by_name`]), whose methods yield typed [`ColumnData`]
//! ([`ColumnReader::raw`]), the `TSCALn`/`TZEROn` physical plane
//! ([`ColumnReader::physical`]), and `P`/`Q` variable-length arrays out of the heap
//! ([`ColumnReader::vla`]), including their numeric, complex, and exact unsigned
//! physical views.

pub(crate) mod bit_column;
pub(crate) mod character_field;
pub(crate) mod column;
pub(crate) mod column_data;
pub(crate) mod column_reader;
pub(crate) mod descriptor;
pub(crate) mod table_schema;
pub(crate) mod tdim;
pub(crate) mod tform;
pub(crate) mod tform_kind;
pub(crate) mod vla_column;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::column::Column;
use crate::table_impl::column_reader::ColumnReader;
use crate::table_impl::descriptor::PqDescriptor;
use crate::table_impl::table_schema::TableSchema;
use crate::table_impl::tform_kind::TformKind;
#[cfg(feature = "compression")]
use crate::table_impl::vla_column::VlaColumn;

/// A binary table's structure plus its data unit.
#[derive(Debug, Clone)]
pub struct BinTable {
    /// Everything the header alone determines — the same [`TableSchema`] that
    /// [`TableSchema::parse`] produces, held rather than re-derived.
    pub(crate) schema: TableSchema,
    /// The whole data unit (the `nrows * row_len` main table, then the heap and
    /// block fill). Fixed-width reads index the main-table prefix; `P`/`Q` columns
    /// follow their descriptors into the heap.
    bytes: Vec<u8>,
}

/// Immutable row and column metadata for a parsed binary table.
#[derive(Debug, Clone, Copy)]
pub struct BinTableMetadata<'a> {
    /// Number of rows in the table.
    pub nrows: usize,
    /// Validated column descriptors in `TFIELDS` order.
    pub columns: &'a [Column],
}

impl BinTable {
    /// Borrow the table's validated row count and column descriptors.
    pub fn metadata(&self) -> BinTableMetadata<'_> {
        BinTableMetadata {
            nrows: self.schema.nrows,
            columns: &self.schema.columns,
        }
    }

    /// Build a table from its header and owned data unit (`data` is the main
    /// table followed by the optional heap, as returned by the reader).
    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<BinTable> {
        let schema = TableSchema::parse(header)?;
        if data.len() < schema.heap_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(BinTable {
            schema,
            bytes: data,
        })
    }

    /// The fixed-width main table (`nrows × NAXIS1` bytes), excluding the heap.
    #[cfg(feature = "compression")]
    pub(crate) fn raw_rows(&self) -> Result<&[u8]> {
        let len = self
            .schema
            .nrows
            .checked_mul(self.schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.bytes.get(..len).ok_or(FitsError::DataSizeMismatch {
            expected: len,
            got: self.bytes.len(),
        })
    }

    /// A handle to the variable-length column named `name`, or `None` when the table
    /// has no such column — the optional per-tile source and mask columns of a
    /// compressed image are all addressed this way.
    #[cfg(feature = "compression")]
    pub(crate) fn optional_vla_column(&self, name: &str) -> Result<Option<VlaColumn<'_>>> {
        match self.column_index(name) {
            Some(index) => Ok(Some(self.column_by_idx(index)?.vla_column()?)),
            None => Ok(None),
        }
    }

    pub(crate) fn pq_payload(
        &self,
        descriptor: PqDescriptor,
        element_kind: TformKind,
    ) -> Result<&[u8]> {
        let range =
            descriptor.heap_range(element_kind, self.schema.heap_offset, self.schema.heap_end)?;
        Ok(&self.bytes[range])
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.schema.column_index(name)
    }

    /// A reader handle for the column at `index`. Decode through it — [`ColumnReader`]
    /// exposes `raw`/`physical`/`unsigned`/`complex`/`bits` and the `vla*` variants —
    /// without re-passing the column descriptor. Errors with
    /// [`FitsError::IndexOutOfBounds`] for a bad index.
    pub fn column_by_idx(&self, index: usize) -> Result<ColumnReader<'_>> {
        self.schema.validate_column_index(index)?;
        Ok(ColumnReader::new(self, index))
    }

    /// A reader handle for the column named `name` (`TTYPEn`, case-insensitive, §6.7).
    /// Errors with [`FitsError::ColumnNotFound`] if no such column exists.
    pub fn column_by_name(&self, name: &str) -> Result<ColumnReader<'_>> {
        let index = self.schema.column_index_checked(name)?;
        Ok(ColumnReader::new(self, index))
    }

    /// The raw bytes of column `col` in row `r`.
    fn cell(&self, col: &Column, r: usize) -> &[u8] {
        let start = r * self.schema.row_len + col.byte_offset;
        &self.bytes[start..start + col.tform.byte_width()]
    }

    fn pq_descriptor(&self, col: &Column, row: usize) -> Result<PqDescriptor> {
        if col.tform.repeat == 0 {
            Ok(PqDescriptor::EMPTY)
        } else {
            PqDescriptor::decode(
                self.cell(col, row),
                col.tform.kind == TformKind::ArrayDesc64,
            )
        }
    }

    fn cells<'a>(&'a self, col: &'a Column) -> impl ExactSizeIterator<Item = &'a [u8]> + 'a {
        (0..self.schema.nrows).map(move |row| self.cell(col, row))
    }
}

#[cfg(all(test, feature = "compression"))]
pub(crate) mod internals {
    use crate::table_impl::BinTable;
    use crate::table_impl::tform_kind::TformKind;

    pub(crate) fn set_column_kind(table: &mut BinTable, column: usize, kind: TformKind) {
        table.schema.columns[column].tform.kind = kind;
    }
}

#[cfg(test)]
mod tests;
