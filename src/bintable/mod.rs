//! Binary-table (`BINTABLE`) reading (§7.3).
//!
//! A binary table is `NAXIS2` rows of `NAXIS1` bytes; each of `TFIELDS` columns
//! occupies a fixed byte range in every row, typed by its `TFORMn` code. That
//! structure parses into a [`Schema`] of [`Column`] descriptors; decoding goes
//! through a [`ColumnReader`] (from [`Table::column`]), whose methods yield typed [`ColumnData`]
//! ([`ColumnReader::raw`]), the `TSCALn`/`TZEROn` physical plane
//! ([`ColumnReader::physical`]), and `P`/`Q` variable-length arrays out of the heap
//! ([`ColumnReader::vla`]), including their numeric, complex, and exact unsigned
//! physical views.
//!
//! [`Column`]: crate::bintable::column::Column
//! [`ColumnData`]: crate::bintable::column_data::ColumnData

pub(crate) mod bit_column;
pub(crate) mod character_field;
pub(crate) mod column;
pub(crate) mod column_data;
pub(crate) mod column_reader;
pub(crate) mod column_view;
pub(crate) mod descriptor;
pub(crate) mod schema;
pub(crate) mod table_view;
pub(crate) mod tdim;
pub(crate) mod tform;
pub(crate) mod tform_kind;
pub(crate) mod vla_column;

use crate::bintable::column_reader::ColumnReader;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::column::ColumnSelector;
use crate::error::Result;

/// A binary table's structure plus its data unit.
#[derive(Debug, Clone)]
pub struct Table {
    /// Everything the header alone determines — the same [`Schema`] that
    /// [`Schema::parse`] produces, held rather than re-derived.
    pub(crate) schema: Schema,
    /// The whole data unit (the `row_count * row_len` main table, then the heap and
    /// block fill). Fixed-width reads index the main-table prefix; `P`/`Q` columns
    /// follow their descriptors into the heap.
    bytes: Vec<u8>,
}

impl Table {
    /// The table `schema` describes, over its owned data unit `data` (the main table
    /// followed by the optional heap). Errors when the data ends before the heap does.
    pub(crate) fn new(schema: Schema, data: Vec<u8>) -> Result<Self> {
        TableView::new(&schema, &data)?;
        Ok(Self {
            schema,
            bytes: data,
        })
    }

    /// The table over its own storage — what every decode reads through.
    pub(crate) fn view(&self) -> TableView<'_> {
        TableView::new(&self.schema, &self.bytes).expect("`new` checked the heap extent")
    }

    /// The table's structure: its row count, row width and validated columns.
    pub const fn schema(&self) -> &Schema {
        &self.schema
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        self.schema.column_index(name)
    }

    /// A reader handle for the column `column` selects, by index or `TTYPEn` name. Decode
    /// through it — [`ColumnReader`] exposes `raw`/`physical`/`unsigned`/`complex`/`bits`
    /// and the `vla*` variants — without re-passing the column descriptor. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last column, and with
    /// [`FitsError::ColumnNotFound`] for a name no column carries.
    ///
    /// [`FitsError::IndexOutOfBounds`]: crate::FitsError::IndexOutOfBounds
    /// [`FitsError::ColumnNotFound`]: crate::FitsError::ColumnNotFound
    pub fn column<'n>(&self, column: impl Into<ColumnSelector<'n>>) -> Result<ColumnReader<'_>> {
        self.view().column(column)
    }
}

#[cfg(feature = "internals")]
pub(crate) mod internals {
    use crate::bintable::Table;
    use crate::bintable::schema::Schema;
    #[cfg(all(test, feature = "compression"))]
    use crate::bintable::tform_kind::TformKind;
    use crate::error::Result;
    use crate::header_model::{Header, keywords};

    impl Table {
        /// Build a table from its header and owned data unit (`data` is the main
        /// table followed by the optional heap, as returned by the reader).
        pub(crate) fn from_data(header: &Header, data: Vec<u8>) -> Result<Self> {
            Self::new(Schema::parse(header)?, data)
        }
    }

    /// A minimal `BINTABLE` header for `tforms`, sized by the caller — the fixture
    /// every read-path test builds its table from.
    pub(crate) fn table_header(naxis1: usize, naxis2: usize, tforms: &[&str]) -> Header {
        let mut h = Header::new();
        h.set_internal(keywords::XTENSION, "BINTABLE")
            .set_internal(keywords::BITPIX, 8)
            .set_internal(keywords::NAXIS, 2)
            .set_internal("NAXIS1", i64::try_from(naxis1).unwrap())
            .set_internal("NAXIS2", i64::try_from(naxis2).unwrap())
            .set_internal(keywords::PCOUNT, 0)
            .set_internal(keywords::GCOUNT, 1)
            .set_internal(keywords::TFIELDS, i64::try_from(tforms.len()).unwrap());
        for (i, tform) in tforms.iter().enumerate() {
            h.set_internal(keywords::TFORM.indexed(i + 1).unwrap(), *tform);
        }
        h
    }

    #[cfg(all(test, feature = "compression"))]
    pub(crate) fn set_column_kind(table: &mut Table, column: usize, kind: TformKind) {
        table.schema.columns[column].tform.kind = kind;
    }
}

#[cfg(test)]
mod tests;
