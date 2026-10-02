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
pub(crate) mod table_view;
pub(crate) mod tdim;
pub(crate) mod tform;
pub(crate) mod tform_kind;
pub(crate) mod vla_column;

use crate::error::Result;
#[cfg(any(feature = "compression", feature = "internals", test))]
use crate::header::Header;
use crate::table_impl::column::Column;
use crate::table_impl::column_reader::ColumnReader;
use crate::table_impl::table_schema::TableSchema;
use crate::table_impl::table_view::TableView;

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
    /// The table `schema` describes, over its owned data unit `data` (the main table
    /// followed by the optional heap). Errors when the data ends before the heap does.
    pub(crate) fn new(schema: TableSchema, data: Vec<u8>) -> Result<BinTable> {
        TableView::new(&schema, &data)?;
        Ok(BinTable {
            schema,
            bytes: data,
        })
    }

    /// Build a table from its header and owned data unit (`data` is the main
    /// table followed by the optional heap, as returned by the reader).
    #[cfg(any(feature = "compression", feature = "internals", test))]
    pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<BinTable> {
        BinTable::new(TableSchema::parse(header)?, data)
    }

    /// The table over its own storage — what every decode reads through.
    pub(crate) fn view(&self) -> TableView<'_> {
        TableView::new(&self.schema, &self.bytes).expect("`new` checked the heap extent")
    }

    /// Borrow the table's validated row count and column descriptors.
    pub fn metadata(&self) -> BinTableMetadata<'_> {
        self.view().metadata()
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
        self.view().column_by_idx(index)
    }

    /// A reader handle for the column named `name` (`TTYPEn`, case-insensitive, §6.7).
    /// Errors with [`FitsError::ColumnNotFound`] if no such column exists.
    pub fn column_by_name(&self, name: &str) -> Result<ColumnReader<'_>> {
        self.view().column_by_name(name)
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::header::Header;
    #[cfg(feature = "compression")]
    use crate::table_impl::BinTable;
    #[cfg(feature = "compression")]
    use crate::table_impl::tform_kind::TformKind;

    /// A minimal `BINTABLE` header for `tforms`, sized by the caller — the fixture
    /// every read-path test builds its table from.
    pub(crate) fn table_header(naxis1: usize, naxis2: usize, tforms: &[&str]) -> Header {
        let mut h = Header::new();
        h.set_internal("XTENSION", "BINTABLE")
            .set_internal("BITPIX", 8)
            .set_internal("NAXIS", 2)
            .set_internal("NAXIS1", naxis1 as i64)
            .set_internal("NAXIS2", naxis2 as i64)
            .set_internal("PCOUNT", 0)
            .set_internal("GCOUNT", 1)
            .set_internal("TFIELDS", tforms.len() as i64);
        for (i, tform) in tforms.iter().enumerate() {
            h.set_internal(&format!("TFORM{}", i + 1), *tform);
        }
        h
    }

    #[cfg(feature = "compression")]
    pub(crate) fn set_column_kind(table: &mut BinTable, column: usize, kind: TformKind) {
        table.schema.columns[column].tform.kind = kind;
    }
}

#[cfg(test)]
mod tests;
