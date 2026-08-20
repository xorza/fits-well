//! Everything a tiled image's decode resolves once, before the first tile.

#[cfg(feature = "parallel")]
use crate::compress::decode;
use crate::compress::decode::decode_sample::DecodeSample;
use crate::compress::decode::float_quantization::FloatQuantization;
use crate::compress::decode::image_layout::ImageLayout;
use crate::compress::decode::null_mask::NullMask;
use crate::compress::decode::tile_decoder::TileDecoder;
use crate::compress::decode::tile_scratch_set::TileScratchSet;
use crate::compress::decode::tile_sources::TileSources;
#[cfg(feature = "parallel")]
use crate::compress::map_tiles;
use crate::compress::tile_geometry::TileGeometry;
#[cfg(feature = "parallel")]
use crate::compress::tile_geometry::TileScratch;
use crate::error::Result;
use crate::header::Header;
use crate::table_impl::BinTable;
use std::ops::Range;

/// Everything a tiled image's decode needs that the header and the table's metadata
/// columns determine once, up front — grouped by the concern each part serves rather
/// than held as one flat bag.
#[derive(Debug)]
pub(super) struct ImageDecodePlan<'a> {
    pub(super) geometry: TileGeometry,
    pub(super) decoder: TileDecoder,
    pub(super) sources: TileSources<'a>,
    pub(super) null_mask: NullMask<'a>,
    pub(super) quantization: FloatQuantization,
}

impl<'a> ImageDecodePlan<'a> {
    pub(super) fn new(
        header: &Header,
        table: &'a BinTable,
        layout: &ImageLayout,
    ) -> Result<ImageDecodePlan<'a>> {
        let tiles = layout.tile_shape(header)?;
        Ok(ImageDecodePlan {
            geometry: TileGeometry::new(&layout.dims, &tiles),
            decoder: TileDecoder::new(header, layout)?,
            sources: TileSources::read(table)?,
            null_mask: NullMask::read(header, table, layout)?,
            quantization: FloatQuantization::read(header, table, layout.bitpix.is_float())?,
        })
    }

    /// Build the plan for `layout` and check it against a buffer the caller sized
    /// from the same layout.
    ///
    /// The buffer's plane and `ZBITPIX` cannot disagree — every caller derives both
    /// from one [`ImageLayout`] — but the dispatch selects the tile decoder from the
    /// plane and the scatter from the buffer, so the pairing is worth stating once
    /// rather than at each of them.
    pub(super) fn for_buffer(
        header: &Header,
        table: &'a BinTable,
        layout: &ImageLayout,
        buffer_is_float: bool,
    ) -> Result<ImageDecodePlan<'a>> {
        let plan = ImageDecodePlan::new(header, table, layout)?;
        debug_assert_eq!(
            plan.decoder.is_float(),
            buffer_is_float,
            "the sample buffer is sized from ZBITPIX, so its plane must match"
        );
        Ok(plan)
    }

    /// Decode every tile and scatter it into the full image plane.
    ///
    /// The two builds share the per-tile decode ([`TileScratchSet::decode`]) but not
    /// the hand-off, and deliberately so: a parallel worker cannot scatter into `out`
    /// directly, so it must hand back an owned buffer, and narrowing *before* that
    /// hand-off is what bounds the memory a wave retains —
    /// [`decode_wave_tile_count`](crate::compress::decode::decode_wave_tile_count)
    /// sizes the wave from `size_of::<D>()`, not from the wide plane. The serial build
    /// has no hand-off to pay for, so it narrows straight into `out` and allocates
    /// nothing per tile.
    pub(super) fn decode_all_into<D: DecodeSample>(&self, out: &mut [D]) -> Result<()> {
        let geom = &self.geometry;
        #[cfg(feature = "parallel")]
        {
            let wave_len = decode::decode_wave_tile_count::<D>(geom);
            let mut scatter = TileScratch::default();
            for wave_start in (0..geom.ntiles()).step_by(wave_len) {
                let count = wave_len.min(geom.ntiles() - wave_start);
                let decoded = map_tiles(
                    count,
                    TileScratchSet::<D::Wide>::default,
                    |scratch, offset| -> Result<Vec<D>> {
                        let tile = wave_start + offset;
                        scratch.decode(self, tile, tile)?;
                        Ok(scratch.values.iter().copied().map(D::narrow).collect())
                    },
                )?;
                for (offset, values) in decoded.iter().enumerate() {
                    geom.tile_into(wave_start + offset, &mut scatter);
                    // Already narrowed, in the worker.
                    scatter.scatter_rows_into(out, values, &std::convert::identity);
                }
            }
            Ok(())
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut scratch = TileScratchSet::<D::Wide>::default();
            for tile in 0..geom.ntiles() {
                scratch.decode(self, tile, tile)?;
                scratch
                    .tile
                    .scatter_rows_into(out, &scratch.values, &D::narrow);
            }
            Ok(())
        }
    }

    /// Decode only the tiles intersecting a requested region, scattering each tile's
    /// intersection into the section plane. Serial: the tiles are already a sparse
    /// subset.
    pub(super) fn decode_region_into<D: DecodeSample>(
        &self,
        ranges: &[Range<usize>],
        selected_shape: &[usize],
        tile_rows: &[usize],
        out: &mut [D],
    ) -> Result<()> {
        let mut scratch = TileScratchSet::<D::Wide>::default();
        for (table_row, &tile_row) in tile_rows.iter().enumerate() {
            scratch.decode(self, table_row, tile_row)?;
            scratch.tile.scatter_region_into(
                ranges,
                selected_shape,
                &scratch.values,
                out,
                &D::narrow,
            );
        }
        Ok(())
    }
}
