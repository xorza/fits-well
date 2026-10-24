//! Everything a tiled image's decode resolves once, before the first tile.

#[cfg(feature = "parallel")]
use crate::allocation;
#[cfg(feature = "parallel")]
use crate::compress::decode;
use crate::compress::decode::decode_sample::DecodeSample;
use crate::compress::decode::ensure_tile_size;
use crate::compress::decode::float_quantization::FloatQuantization;
use crate::compress::decode::null_mask::NullMask;
use crate::compress::decode::tile_decoder::TileDecoder;
use crate::compress::decode::tile_scratch_set::TileScratchSet;
use crate::compress::decode::tile_section::TileSection;
use crate::compress::decode::tile_sources::TileSources;
use crate::compress::decode::wide_plane::{TileJob, WidePlane};
use crate::compress::tile_geometry::{PlanePart, TileGeometry, TileScratch};
use crate::error::Result;
use std::ops::Range;

/// Everything a tiled image's decode needs from its container table, beside what the
/// header fixed for every decode in `TiledImage` — grouped by the concern each part
/// serves rather than held as one flat bag.
#[derive(Debug)]
pub(super) struct ImageDecodePlan<'a> {
    pub(super) geometry: TileGeometry,
    pub(super) decoder: &'a TileDecoder,
    pub(super) sources: TileSources<'a>,
    pub(super) nulls: NullMask<'a>,
    /// A float image's quantization; `None` for an integer image, which is stored
    /// without one and ignores its keywords and columns.
    pub(super) quantization: Option<FloatQuantization>,
}

/// Where a tile lies: its row of the compressed table, and its row of the image's tile
/// grid, which drives the dither sequence and the scatter. They coincide for a
/// whole-image decode and differ for a section, which reads only the table rows its
/// region intersects.
#[derive(Debug, Clone, Copy)]
struct TileRows {
    table: usize,
    grid: usize,
}

impl ImageDecodePlan<'_> {
    /// Decode every tile and scatter it into the full image plane.
    ///
    /// The tiles at one index along the slowest axis write one band of the plane and
    /// nothing else, so with enough bands for the workers each takes whole bands and
    /// narrows its tiles straight into its own part of `out`. An image of fewer bands
    /// decodes in waves through one reused buffer. The serial build narrows every tile
    /// straight into `out` and allocates nothing per tile.
    pub(super) fn decode_all_into<D: DecodeSample>(&self, out: &mut [D]) -> Result<()> {
        let geom = &self.geometry;
        let bands = geom.bands();
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            if bands.count < rayon::current_num_threads().min(geom.ntiles()) {
                let mut scatter = TileScratch::default();
                return self.decode_waves(
                    geom.ntiles(),
                    |tile| TileRows {
                        table: tile,
                        grid: tile,
                    },
                    |tile, values| {
                        geom.tile_into(tile, &mut scatter);
                        let mut whole = PlanePart {
                            values: &mut *out,
                            start: 0,
                        };
                        scatter.scatter_rows_into(&mut whole, values, &|value: D| value);
                    },
                );
            }
            out.par_chunks_mut(bands.len).enumerate().try_for_each_init(
                TileScratchSet::<D::Wide>::default,
                |scratch, (band, part)| {
                    self.decode_band(scratch, bands.tiles_of(band), bands.part(band, part))
                },
            )
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut scratch = TileScratchSet::<D::Wide>::default();
            for (band, part) in out.chunks_mut(bands.len).enumerate() {
                self.decode_band(&mut scratch, bands.tiles_of(band), bands.part(band, part))?;
            }
            Ok(())
        }
    }

    /// Decode the `tiles` of one band, narrowing each into `part`, the band's run of the
    /// image plane.
    fn decode_band<D: DecodeSample>(
        &self,
        scratch: &mut TileScratchSet<D::Wide>,
        tiles: Range<usize>,
        mut part: PlanePart<'_, D>,
    ) -> Result<()> {
        for tile in tiles {
            self.decode_tile::<D>(
                scratch,
                TileRows {
                    table: tile,
                    grid: tile,
                },
            )?;
            scratch
                .tile
                .scatter_rows_into(&mut part, &scratch.values, &D::narrow);
        }
        Ok(())
    }

    /// Decode only the tiles intersecting a requested region, scattering each tile's
    /// intersection into the section plane.
    pub(super) fn decode_region_into<D: DecodeSample>(
        &self,
        section: TileSection<'_>,
        out: &mut [D],
    ) -> Result<()> {
        let mut scatter = TileScratch::default();
        self.decode_waves(
            section.rows.len(),
            |table_row| TileRows {
                table: table_row,
                grid: section.rows[table_row],
            },
            |tile_row, values| {
                self.geometry.tile_into(tile_row, &mut scatter);
                scatter.scatter_region_into(
                    section.ranges,
                    section.shape,
                    values,
                    out,
                    &|value: D| value,
                );
            },
        )
    }

    /// Decode `count` tiles — the `i`th the table row and tile row `rows(i)` give — and
    /// hand each tile row and its narrowed samples to `scatter`, in order. In parallel,
    /// in waves whose tiles share one reused buffer, a slot each, bounded by
    /// [`decode_wave_tile_count`](crate::compress::decode::decode_wave_tile_count).
    fn decode_waves<D: DecodeSample>(
        &self,
        count: usize,
        rows: impl Fn(usize) -> TileRows + Sync,
        mut scatter: impl FnMut(usize, &[D]),
    ) -> Result<()> {
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            let slot = self.geometry.max_tile_elements();
            let wave_len = decode::decode_wave_tile_count::<D>(&self.geometry).min(count);
            let mut wave = allocation::try_zeroed(D::default(), wave_len.saturating_mul(slot))?;
            let mut lengths = vec![0usize; wave_len];
            for wave_start in (0..count).step_by(wave_len.max(1)) {
                let tiles = wave_len.min(count - wave_start);
                wave[..tiles * slot]
                    .par_chunks_mut(slot)
                    .zip(&mut lengths[..tiles])
                    .enumerate()
                    .try_for_each_init(
                        TileScratchSet::<D::Wide>::default,
                        |scratch, (offset, (values, length))| -> Result<()> {
                            self.decode_tile::<D>(scratch, rows(wave_start + offset))?;
                            *length = scratch.values.len();
                            for (slot, &value) in values.iter_mut().zip(&scratch.values) {
                                *slot = D::narrow(value);
                            }
                            Ok(())
                        },
                    )?;
                for offset in 0..tiles {
                    let start = offset * slot;
                    scatter(
                        rows(wave_start + offset).grid,
                        &wave[start..start + lengths[offset]],
                    );
                }
            }
            Ok(())
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut scratch = TileScratchSet::<D::Wide>::default();
            let mut narrowed = Vec::new();
            for index in 0..count {
                let at = rows(index);
                self.decode_tile::<D>(&mut scratch, at)?;
                narrowed.clear();
                narrowed.extend(scratch.values.iter().map(|&value| D::narrow(value)));
                scatter(at.grid, &narrowed);
            }
            Ok(())
        }
    }

    /// The tile at `at` decoded into `scratch` and fitted to the stored type `D`.
    fn decode_tile<D: DecodeSample>(
        &self,
        scratch: &mut TileScratchSet<D::Wide>,
        at: TileRows,
    ) -> Result<()> {
        let TileRows {
            table: table_row,
            grid: tile_row,
        } = at;
        self.geometry.tile_into(tile_row, &mut scratch.tile);
        let job = TileJob {
            decoder: self.decoder,
            nulls: &self.nulls,
            source: self.sources.source(table_row)?,
            table_row,
            encoding: self
                .quantization
                .as_ref()
                .map(|quantization| quantization.encoding(table_row, tile_row)),
        };
        let overshoot = D::Wide::decode_tile(job, scratch)?;
        ensure_tile_size(scratch.tile.nelem(), scratch.values.len())?;
        D::fit(&mut scratch.values, overshoot)
    }
}
