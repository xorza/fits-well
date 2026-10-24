//! The reusable per-worker buffers one tile's decode writes through.

use crate::compression::tile_codec::CodecScratch;
use crate::compression::tile_geometry::TileScratch;

/// Reusable per-worker buffers for one tile: its geometry, the decoded values in
/// their plane, and the codec workspaces, which lend a tile's quantized samples and its
/// null mask.
#[derive(Debug)]
pub(super) struct TileScratchSet<W> {
    pub(super) tile: TileScratch,
    pub(super) values: Vec<W>,
    pub(super) codecs: CodecScratch,
}

// Hand-written rather than derived: `Vec<W>` is `Default` for every `W`, so the
// derive's `W: Default` bound would be a fiction.
impl<W> Default for TileScratchSet<W> {
    fn default() -> Self {
        Self {
            tile: TileScratch::default(),
            values: Vec::new(),
            codecs: CodecScratch::default(),
        }
    }
}
