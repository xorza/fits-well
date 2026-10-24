//! [`DecodeCapacities`]: the most a read's decode holds, buffer by buffer.

use crate::compression::decode::tile_scratch_set::SetCapacities;
use crate::compression::tile_geometry::TileCapacities;

/// What one read's decode holds while it runs, from
/// [`ImageDecodePlan::decode_capacities`]: the wave of narrowed tiles and their lengths,
/// the tile the scatter places, and the tile sets. The wave and the scatter tile are
/// exact; the sets are a bound — one for each worker that takes a run of the read's tiles,
/// as many as the workers or the runs, each reserved to the largest tile's buffers before
/// its first tile, so none grows.
///
/// [`ImageDecodePlan::decode_capacities`]: crate::compression::decode::image_decode_plan::ImageDecodePlan::decode_capacities
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct DecodeCapacities {
    pub(super) wave_bytes: usize,
    pub(super) lengths: usize,
    pub(super) scatter: TileCapacities,
    /// The most tile sets alive at once.
    pub(super) sets: usize,
    pub(super) set: SetCapacities,
}

impl DecodeCapacities {
    /// The heap bytes the decode holds at most.
    pub(crate) const fn heap_bytes(&self) -> usize {
        self.wave_bytes
            + self.lengths * size_of::<usize>()
            + self.scatter.heap_bytes()
            + self.sets * self.set.heap_bytes()
    }
}
