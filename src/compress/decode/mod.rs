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
//! The work splits by concern: [`ImageLayout`] is what the header says the image is,
//! [`ImageDecodePlan`](image_decode_plan::ImageDecodePlan) is everything the decode
//! resolves once up front, and [`DecodeBuffer`] is the caller's output plane, which
//! selects the stored sample type the tiles narrow into.

mod decode_buffer;
mod decode_sample;
mod float_quantization;
mod image_decode_plan;
mod image_layout;
mod image_region_layout;
mod null_mask;
mod tile_cells;
mod tile_decoder;
mod tile_scratch_set;
mod tile_sources;
mod wide_plane;

use std::ops::Range;

use crate::allocation;
use crate::compress::convert;
use crate::compress::decode::decode_buffer::DecodeBuffer;
use crate::compress::decode::image_layout::ImageLayout;
use crate::compress::decode::image_region_layout::ImageRegionLayout;
#[cfg(feature = "parallel")]
use crate::compress::tile_geometry::TileGeometry;
use crate::data::Image;
use crate::data::image_view::BorrowedImage;
use crate::data::validate_image_region;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::BinTable;

/// Decompress a tiled-image `BINTABLE` into the full [`Image`] it encodes.
pub(crate) fn decompress_image(header: &Header, table: &BinTable) -> Result<Image> {
    let layout = ImageLayout::from_header(header)?;
    let mut samples = convert::zeroed_samples(layout.bitpix, layout.total)?;
    if layout.total != 0 {
        DecodeBuffer::from_samples(&mut samples).decode_image(header, table, &layout)?;
    }
    Image::new_scaled(layout.dims, samples, layout.scaling)
}

pub(crate) fn decompress_image_into_words<'a>(
    header: &Header,
    table: &BinTable,
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
    let layout = ImageLayout::from_header(header)?;
    let nbytes = layout
        .total
        .checked_mul(layout.bitpix.elem_size())
        .ok_or(FitsError::DataUnitOverflow)?;
    allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
    if layout.total != 0 {
        DecodeBuffer::from_words(words, layout.bitpix, layout.total)
            .decode_image(header, table, &layout)?;
    }
    Ok(BorrowedImage {
        shape: layout.dims,
        scaling: layout.scaling,
        samples: view_words(words, layout.bitpix, nbytes),
    })
}

/// Original compressed-table row indices for tiles intersecting `ranges`.
pub(crate) fn compressed_image_tile_rows(
    header: &Header,
    ranges: &[Range<usize>],
) -> Result<Vec<usize>> {
    let layout = ImageLayout::from_header(header)?;
    validate_image_region(ranges, &layout.dims)?;
    if ranges.iter().any(Range::is_empty) || layout.dims.is_empty() {
        return Ok(Vec::new());
    }
    let tiles = layout.tile_shape(header)?;
    let counts: Vec<usize> = layout
        .dims
        .iter()
        .zip(&tiles)
        .map(|(&dim, &tile)| dim.div_ceil(tile))
        .collect();
    let starts: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| range.start / tile)
        .collect();
    let ends: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| (range.end - 1) / tile + 1)
        .collect();
    let tile_count = starts
        .iter()
        .zip(&ends)
        .try_fold(1usize, |count, (&start, &end)| {
            count.checked_mul(end - start)
        })
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut coordinates = starts.clone();
    let mut selected = Vec::with_capacity(tile_count);
    for _ in 0..tile_count {
        let mut stride = 1usize;
        let mut index = 0usize;
        for axis in 0..coordinates.len() {
            index = index
                .checked_add(
                    coordinates[axis]
                        .checked_mul(stride)
                        .ok_or(FitsError::DataUnitOverflow)?,
                )
                .ok_or(FitsError::DataUnitOverflow)?;
            stride = stride
                .checked_mul(counts[axis])
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        selected.push(index);
        for axis in 0..coordinates.len() {
            coordinates[axis] += 1;
            if coordinates[axis] < ends[axis] {
                break;
            }
            coordinates[axis] = starts[axis];
        }
    }
    Ok(selected)
}

/// Decompress only the compact table rows in `tile_rows`, scattering their
/// intersections into one scratch-backed image section.
pub(crate) fn decompress_image_section_into_words<'a>(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
    let region = ImageRegionLayout::new(header, table, tile_rows, ranges)?;
    allocation::try_resize(words, region.nbytes.div_ceil(8), 0)?;
    if region.total != 0 {
        DecodeBuffer::from_words(words, region.image.bitpix, region.total)
            .decode_section(header, table, tile_rows, ranges, &region)?;
    }
    Ok(BorrowedImage {
        shape: region.shape,
        scaling: region.image.scaling,
        samples: view_words(words, region.image.bitpix, region.nbytes),
    })
}

pub(crate) fn decompress_image_section(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
) -> Result<Image> {
    let region = ImageRegionLayout::new(header, table, tile_rows, ranges)?;
    let mut samples = convert::zeroed_samples(region.image.bitpix, region.total)?;
    if region.total != 0 {
        DecodeBuffer::from_samples(&mut samples)
            .decode_section(header, table, tile_rows, ranges, &region)?;
    }
    Image::new_scaled(region.shape, samples, region.image.scaling)
}

/// How many tiles one parallel decode wave may retain at once, from the memory a
/// wave's *narrowed* per-tile vectors hold rather than the wide plane they decode in.
#[cfg(feature = "parallel")]
pub(super) fn decode_wave_tile_count<D>(geom: &TileGeometry) -> usize {
    const DECODE_WAVE_BYTES: usize = 4 * 1024 * 1024;

    let payload_bytes = geom
        .max_tile_elements()
        .saturating_mul(std::mem::size_of::<D>());
    let retained_bytes = payload_bytes
        .saturating_add(std::mem::size_of::<Vec<D>>())
        .max(1);
    (DECODE_WAVE_BYTES / retained_bytes).max(1)
}

fn ensure_tile_size(expected: usize, got: usize) -> Result<()> {
    if got != expected {
        return Err(FitsError::DataSizeMismatch { expected, got });
    }
    Ok(())
}

#[cfg(test)]
mod tests;
