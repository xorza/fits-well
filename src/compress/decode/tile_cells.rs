//! One tile's candidate source cells and the fallback order between them.

use crate::error::FitsError;
use crate::error::Result;
use crate::table_impl::vla_column::VlaCell;

/// One tile's three candidate source cells, read from
/// [`TileSources`](crate::compress::decode::tile_sources::TileSources). The tile is
/// decoded from the first non-empty one: the primary `COMPRESSED_DATA` (via
/// `ZCMPTYPE`), else gzip'd `GZIP_COMPRESSED_DATA`, else raw `UNCOMPRESSED_DATA`.
#[derive(Debug, Clone, Copy)]
pub(super) struct TileCells<'a> {
    pub(super) primary: Option<VlaCell<'a>>,
    pub(super) gzip: Option<VlaCell<'a>>,
    pub(super) uncompressed: Option<VlaCell<'a>>,
}

/// The resolved source for one tile — which non-empty column holds its bytes.
#[derive(Debug)]
pub(super) enum TileSource<'a> {
    Compressed(VlaCell<'a>),
    Gzip(VlaCell<'a>),
    Uncompressed(VlaCell<'a>),
}

impl<'a> TileCells<'a> {
    /// Pick the first non-empty source: primary `COMPRESSED_DATA`, then the
    /// gzip and uncompressed fallbacks; error if every column's cell is empty.
    pub(super) fn resolve(self) -> Result<TileSource<'a>> {
        if let Some(c) = self.primary.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Compressed(c))
        } else if let Some(c) = self.gzip.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Gzip(c))
        } else if let Some(c) = self.uncompressed.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Uncompressed(c))
        } else {
            Err(FitsError::UnsupportedCompression {
                name: "empty tile (no compressed or uncompressed data)".to_string(),
            })
        }
    }
}
