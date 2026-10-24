//! [`TileIndex`]: a tiled image's container rows, read once for every read of the image.

use crate::allocation;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::compression::decode::tile_value::TileColumn;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// The main table of a tiled image's container: a row a tile, which holds the tile's
/// descriptors into the heap and its per-tile parameters. Read when the image is opened
/// and kept, so a read of some tiles fetches only their heap cells, and the bytes a read
/// holds are known before it starts.
#[derive(Debug, Default)]
pub(crate) struct TileIndex {
    /// The data offset of the HDU whose rows these are, which a reopened image matches.
    key: Option<u64>,
    rows: Vec<u8>,
    zscale: TileColumn<f64>,
    zzero: TileColumn<f64>,
    zblank: TileColumn<i64>,
}

impl TileIndex {
    /// Hold the rows of the container `schema` describes, the data unit at `key`, which
    /// `read_rows` appends to the buffer it is given, unless they are held already.
    pub(crate) fn load(
        &mut self,
        key: u64,
        schema: &Schema,
        read_rows: impl FnOnce(usize, &mut Vec<u8>) -> Result<()>,
    ) -> Result<()> {
        if self.key == Some(key) {
            return Ok(());
        }
        self.key = None;
        let len = main_len(schema)?;
        self.rows.clear();
        allocation::try_reserve_exact(&mut self.rows, len)?;
        read_rows(len, &mut self.rows)?;
        self.read_columns(schema)?;
        self.key = Some(key);
        Ok(())
    }

    fn read_columns(&mut self, schema: &Schema) -> Result<()> {
        let Self {
            rows,
            zscale,
            zzero,
            zblank,
            ..
        } = self;
        let table = TableView::compact(schema, schema.extent.row_count, rows);
        zscale.read_real(table, "ZSCALE")?;
        zzero.read_real(table, "ZZERO")?;
        zblank.read_integer(table, "ZBLANK")
    }

    /// The rows as a table of the container `schema` describes, with no heap.
    pub(crate) fn table<'a>(&'a self, schema: &'a Schema) -> TableView<'a> {
        TableView::compact(schema, schema.extent.row_count, &self.rows)
    }

    /// The main table's bytes, a row a tile.
    pub(crate) fn rows(&self) -> &[u8] {
        &self.rows
    }

    pub(super) fn zscale(&self) -> Option<&[f64]> {
        self.zscale.values()
    }

    pub(super) fn zzero(&self) -> Option<&[f64]> {
        self.zzero.values()
    }

    pub(super) fn zblank(&self) -> Option<&[i64]> {
        self.zblank.values()
    }

    /// The heap bytes the index holds.
    pub(crate) const fn heap_bytes(&self) -> usize {
        self.rows.capacity()
            + self.zscale.heap_bytes()
            + self.zzero.heap_bytes()
            + self.zblank.heap_bytes()
    }
}

/// The bytes of the main table `schema` describes.
fn main_len(schema: &Schema) -> Result<usize> {
    schema
        .extent
        .row_count
        .checked_mul(schema.extent.row_len)
        .ok_or(FitsError::DataUnitOverflow)
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::schema::Schema;
    use crate::bintable::table_view::TableView;
    use crate::compression::decode::tile_index::TileIndex;
    use crate::fits_error::Result;

    impl TileIndex {
        /// The index of `table`, a container already in memory, which `schema` describes.
        pub(crate) fn of_table(schema: &Schema, table: TableView<'_>) -> Result<Self> {
            let mut index = Self::default();
            index.rows.extend_from_slice(table.main_rows());
            index.read_columns(schema)?;
            Ok(index)
        }
    }
}
