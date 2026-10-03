//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`](crate::compress::gzip)/[`rice`](crate::compress::rice)/
//! [`plio`](crate::compress::plio)/[`hcompress`](crate::compress::hcompress) modules;
//! this drives the tile geometry, the fallback-column resolution, and the
//! narrow-and-scatter into the output plane.
//!
//! The work splits by concern: [`TiledImage`](tiled_image::TiledImage) is what the
//! header says the image and its tiling are,
//! [`ImageDecodePlan`](image_decode_plan::ImageDecodePlan) is everything the decode
//! resolves once up front, and [`DecodeBuffer`](decode_buffer::DecodeBuffer) is the
//! caller's output plane, which selects the stored sample type the tiles narrow into.
//!
//! [`Image`]: crate::data::Image

mod decode_buffer;
mod decode_sample;
mod float_quantization;
mod image_decode_plan;
mod null_mask;
mod tile_cells;
mod tile_decoder;
mod tile_scratch_set;
mod tile_sources;
pub(crate) mod tiled_image;
mod wide_plane;

#[cfg(feature = "parallel")]
use crate::compress::tile_geometry::TileGeometry;
use crate::error::FitsError;
use crate::error::Result;

/// The memory one parallel decode wave's retained tiles may hold.
#[cfg(feature = "parallel")]
const DECODE_WAVE_BYTES: usize = 4 * 1024 * 1024;

/// How many tiles one parallel decode wave may retain at once, from the memory a
/// wave's *narrowed* per-tile vectors hold rather than the wide plane they decode in.
#[cfg(feature = "parallel")]
pub(super) fn decode_wave_tile_count<D>(geom: &TileGeometry) -> usize {
    let payload_bytes = geom.max_tile_elements().saturating_mul(size_of::<D>());
    let retained_bytes = payload_bytes.saturating_add(size_of::<Vec<D>>()).max(1);
    (DECODE_WAVE_BYTES / retained_bytes).max(1)
}

const fn ensure_tile_size(expected: usize, got: usize) -> Result<()> {
    if got != expected {
        return Err(FitsError::DataSizeMismatch { expected, got });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
