//! Everything a tiled image's decode resolves once, before the first tile.

use crate::bintable::table_view::TableView;
#[cfg(feature = "parallel")]
use crate::compress::decode;
use crate::compress::decode::decode_sample::DecodeSample;
use crate::compress::decode::float_quantization::FloatQuantization;
use crate::compress::decode::null_mask::NullMask;
use crate::compress::decode::tile_decoder::TileDecoder;
use crate::compress::decode::tile_scratch_set::TileScratchSet;
use crate::compress::decode::tile_sources::TileSources;
use crate::compress::decode::tiled_image::TileSection;
use crate::compress::decode::tiled_image::TiledImage;
#[cfg(feature = "parallel")]
use crate::compress::map_tiles;
use crate::compress::tile_geometry::TileGeometry;
#[cfg(feature = "parallel")]
use crate::compress::tile_geometry::TileScratch;
use crate::error::Result;
use crate::header_model::Header;

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
        table: TableView<'a>,
        tiled: &TiledImage<'_>,
    ) -> Result<ImageDecodePlan<'a>> {
        Ok(ImageDecodePlan {
            geometry: TileGeometry::new(&tiled.image.shape, &tiled.tiles),
            decoder: TileDecoder::new(header, tiled)?,
            sources: TileSources::read(table)?,
            null_mask: NullMask::read(header, table, tiled)?,
            quantization: FloatQuantization::read(header, table, tiled.image.bitpix.is_float())?,
        })
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
        section: TileSection<'_>,
        out: &mut [D],
    ) -> Result<()> {
        let mut scratch = TileScratchSet::<D::Wide>::default();
        for (table_row, &tile_row) in section.rows.iter().enumerate() {
            scratch.decode(self, table_row, tile_row)?;
            scratch.tile.scatter_region_into(
                section.ranges,
                section.shape,
                &scratch.values,
                out,
                &D::narrow,
            );
        }
        Ok(())
    }
}
