//! The per-tile source columns a compressed image stores its tiles in.

use crate::bintable::table_view::TableView;
use crate::bintable::vla_column::VlaColumn;
use crate::compress::decode::tile_cells::TileCells;
use crate::error::Result;

/// The three per-tile source columns (§10.1.3): the primary `COMPRESSED_DATA` and
/// the `GZIP_COMPRESSED_DATA` / `UNCOMPRESSED_DATA` fallbacks. Any of them may be
/// absent, and each tile picks the first with a non-empty cell.
#[derive(Debug, Clone, Copy)]
pub(super) struct TileSources<'a> {
    primary: Option<VlaColumn<'a>>,
    gzip_fallback: Option<VlaColumn<'a>>,
    uncompressed: Option<VlaColumn<'a>>,
}

impl<'a> TileSources<'a> {
    pub(super) fn read(table: TableView<'a>) -> Result<TileSources<'a>> {
        Ok(TileSources {
            primary: table.optional_vla_column("COMPRESSED_DATA")?,
            gzip_fallback: table.optional_vla_column("GZIP_COMPRESSED_DATA")?,
            uncompressed: table.optional_vla_column("UNCOMPRESSED_DATA")?,
        })
    }

    /// The three candidate cells for one table row.
    pub(super) fn cells(&self, row: usize) -> Result<TileCells<'a>> {
        Ok(TileCells {
            primary: self.primary.map(|column| column.cell(row)).transpose()?,
            gzip: self
                .gzip_fallback
                .map(|column| column.cell(row))
                .transpose()?,
            uncompressed: self
                .uncompressed
                .map(|column| column.cell(row))
                .transpose()?,
        })
    }
}
