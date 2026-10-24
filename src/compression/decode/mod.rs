//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`](crate::compression::gzip)/[`rice`](crate::compression::rice)/
//! [`plio`](crate::compression::plio)/[`hcompress`](crate::compression::hcompress) modules;
//! this drives the tile geometry, the fallback-column resolution, and the
//! narrow-and-scatter into the output plane.
//!
//! The work splits by concern: [`TiledImage`](tiled_image::TiledImage) is what the
//! header says the image and its tiling are,
//! [`ImageDecodePlan`](image_decode_plan::ImageDecodePlan) is everything the decode
//! resolves once up front, and [`DecodeBuffer`](decode_buffer::DecodeBuffer) is the
//! caller's output plane, which selects the stored sample type the tiles narrow into.
//!
//! [`Image`]: crate::image_model::Image

mod decode_buffer;
pub(crate) mod decode_capacities;
mod decode_sample;
pub(super) mod float_quantization;
pub(super) mod float_sample;
mod image_decode_plan;
mod null_mask;
mod tile_decoder;
pub(crate) mod tile_index;
mod tile_scratch_set;
pub(crate) mod tile_section;
mod tile_sources;
mod tile_value;
pub(crate) mod tiled_image;
mod wide_plane;

#[cfg(feature = "parallel")]
use crate::compression::tile_geometry::TileGeometry;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;

/// The memory one parallel decode wave's retained tiles may hold.
#[cfg(feature = "parallel")]
const DECODE_WAVE_BYTES: usize = 4 * 1024 * 1024;

/// How many tiles one parallel decode wave may hold at once: its one buffer has a slot
/// of the largest tile's *narrowed* samples for each, rather than the wide plane they
/// decode in.
#[cfg(feature = "parallel")]
pub(super) fn decode_wave_tile_count<D>(geom: &TileGeometry) -> usize {
    let slot_bytes = geom
        .max_tile_elements()
        .saturating_mul(size_of::<D>())
        .max(1);
    (DECODE_WAVE_BYTES / slot_bytes).max(1)
}

const fn ensure_tile_size(expected: usize, got: usize) -> Result<()> {
    if got != expected {
        return Err(SizeUnit::Samples.mismatch(expected, got));
    }
    Ok(())
}

#[cfg(test)]
mod tests;
