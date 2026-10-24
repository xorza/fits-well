//! The reusable buffers one tile's decode writes through.

use crate::allocation;
use crate::compression::tile_codec::{CodecCapacities, CodecScratch};
use crate::compression::tile_geometry::{TileCapacities, TileScratch};
use crate::fits_error::Result;

/// Reusable buffers for one tile: its geometry, the decoded values in their plane, and
/// the codec workspaces, which lend a tile's quantized samples and its null mask.
#[derive(Debug)]
pub(super) struct TileScratchSet<W> {
    pub(super) tile: TileScratch,
    pub(super) values: Vec<W>,
    pub(super) codecs: CodecScratch,
}

/// The elements of a [`TileScratchSet`]'s buffers that decoding the largest tile of an
/// image fills, whichever of its cells a tile is read from.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct SetCapacities {
    pub(super) tile: TileCapacities,
    pub(super) values: usize,
    /// The bytes of one decoded value.
    pub(super) value_size: usize,
    pub(super) codecs: CodecCapacities,
}

impl SetCapacities {
    /// The heap bytes a set holds.
    pub(super) const fn heap_bytes(&self) -> usize {
        self.tile.heap_bytes() + self.values * self.value_size + self.codecs.heap_bytes()
    }
}

impl<W> TileScratchSet<W> {
    /// Grow the buffers to `capacities`, no room past them.
    pub(super) fn reserve(&mut self, capacities: &SetCapacities) -> Result<()> {
        self.tile.reserve(&capacities.tile)?;
        allocation::try_grow_to(&mut self.values, capacities.values)?;
        self.codecs.reserve(&capacities.codecs)
    }
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
