//! The per-tile source columns a compressed image stores its tiles in.

use crate::bintable::table_view::TableView;
use crate::bintable::vla_column::{VlaCell, VlaColumn};
use crate::error::CompressedDataDefect;
use crate::error::Result;

/// The three per-tile source columns (§10.1.3): the primary `COMPRESSED_DATA` and
/// the `GZIP_COMPRESSED_DATA` / `UNCOMPRESSED_DATA` fallbacks. Any of them may be
/// absent, and each tile takes the first with a non-empty cell.
#[derive(Debug, Clone, Copy)]
pub(super) struct TileSources<'a> {
    primary: Option<VlaColumn<'a>>,
    gzip_fallback: Option<VlaColumn<'a>>,
    uncompressed: Option<VlaColumn<'a>>,
}

/// The resolved source for one tile — which non-empty column holds its bytes.
#[derive(Debug, Clone, Copy)]
pub(super) enum TileSource<'a> {
    Compressed(VlaCell<'a>),
    Gzip(VlaCell<'a>),
    Uncompressed(VlaCell<'a>),
}

impl<'a> TileSources<'a> {
    pub(super) fn read(table: TableView<'a>) -> Result<Self> {
        Ok(TileSources {
            primary: table.optional_vla_column("COMPRESSED_DATA")?,
            gzip_fallback: table.optional_vla_column("GZIP_COMPRESSED_DATA")?,
            uncompressed: table.optional_vla_column("UNCOMPRESSED_DATA")?,
        })
    }

    /// The source of the tile in table row `row`: the primary `COMPRESSED_DATA` cell,
    /// then the gzip and uncompressed fallbacks, each read only when the ones before it
    /// are empty, as cfitsio reads them. Errors when every cell is empty.
    pub(super) fn source(&self, row: usize) -> Result<TileSource<'a>> {
        let cell = |column: Option<VlaColumn<'a>>| -> Result<Option<VlaCell<'a>>> {
            Ok(column
                .map(|column| column.cell(row))
                .transpose()?
                .filter(|cell| cell.element_count > 0))
        };
        if let Some(cell) = cell(self.primary)? {
            Ok(TileSource::Compressed(cell))
        } else if let Some(cell) = cell(self.gzip_fallback)? {
            Ok(TileSource::Gzip(cell))
        } else if let Some(cell) = cell(self.uncompressed)? {
            Ok(TileSource::Uncompressed(cell))
        } else {
            Err(CompressedDataDefect::EmptyTile.into())
        }
    }
}
