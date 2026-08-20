//! The reusable per-worker buffers one tile's decode writes through.

use crate::compress::decode::ensure_tile_size;
use crate::compress::decode::image_decode_plan::ImageDecodePlan;
use crate::compress::decode::wide_plane::WidePlane;
use crate::compress::gzip;
use crate::compress::hcompress;
use crate::compress::tile_geometry::TileScratch;
use crate::error::Result;

/// Reusable per-worker buffers for one tile: its geometry, the decoded values in
/// their widened plane, the codecs' auxiliary integer buffer (quantized samples for a
/// float plane, the null mask for an integer one), and the codec workspaces.
#[derive(Debug)]
pub(super) struct TileScratchSet<W> {
    pub(super) tile: TileScratch,
    pub(super) values: Vec<W>,
    pub(super) aux: Vec<i64>,
    pub(super) codecs: CodecScratch,
}

/// The per-codec workspaces a tile decode reuses across tiles.
#[derive(Debug, Default)]
pub(super) struct CodecScratch {
    pub(super) gzip: gzip::GzipScratch,
    pub(super) hcompress: hcompress::HcompressScratch,
}

// Hand-written rather than derived: `Vec<W>` is `Default` for every `W`, so the
// derive's `W: Default` bound would be a fiction.
impl<W> Default for TileScratchSet<W> {
    fn default() -> TileScratchSet<W> {
        TileScratchSet {
            tile: TileScratch::default(),
            values: Vec::new(),
            aux: Vec::new(),
            codecs: CodecScratch::default(),
        }
    }
}

impl<W: WidePlane> TileScratchSet<W> {
    /// Reconstruct one tile into this scratch: resolve its geometry, decode its
    /// samples in the wide plane, and check the count against the header's tiling.
    ///
    /// `tile_row` indexes the image's tile grid (it drives the dither sequence and
    /// the scatter); `table_row` indexes the compressed table. They coincide for a
    /// whole-image decode and diverge for a section, which reads only the rows its
    /// region intersects.
    pub(super) fn decode(
        &mut self,
        plan: &ImageDecodePlan<'_>,
        table_row: usize,
        tile_row: usize,
    ) -> Result<()> {
        plan.geometry.tile_into(tile_row, &mut self.tile);
        W::decode_tile(plan, table_row, tile_row, self)?;
        ensure_tile_size(self.tile.nelem(), self.values.len())
    }
}
