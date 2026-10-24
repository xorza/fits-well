//! Everything a tiled image's decode resolves once, before the first tile.

use crate::allocation;
#[cfg(feature = "parallel")]
use crate::compression::decode;
use crate::compression::decode::decode_capacities::DecodeCapacities;
use crate::compression::decode::decode_sample::DecodeSample;
use crate::compression::decode::ensure_tile_size;
use crate::compression::decode::float_quantization::FloatQuantization;
use crate::compression::decode::null_mask::NullMask;
use crate::compression::decode::tile_decoder::TileDecoder;
use crate::compression::decode::tile_scratch_set::{SetCapacities, TileScratchSet};
use crate::compression::decode::tile_section::TileSection;
use crate::compression::decode::tile_sources::TileSources;
use crate::compression::decode::wide_plane::{TileJob, WidePlane};
use crate::compression::tile_geometry::{PlanePart, TileGeometry, TileScratch};
use crate::fits_error::Result;
use std::ops::Range;

/// Everything a tiled image's decode needs from its container table, beside what the
/// header fixed for every decode in `TiledImage` — grouped by the concern each part
/// serves rather than held as one flat bag.
#[derive(Debug)]
pub(super) struct ImageDecodePlan<'a> {
    pub(super) geometry: &'a TileGeometry,
    pub(super) decoder: &'a TileDecoder,
    pub(super) sources: TileSources<'a>,
    pub(super) nulls: NullMask<'a>,
    /// A float image's quantization; `None` for an integer image, which is stored
    /// without one and ignores its keywords and columns.
    pub(super) quantization: Option<FloatQuantization<'a>>,
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

/// How a read's tiles decode: in waves of `wave` tiles through one buffer, which the
/// scatter then places; or, for a whole image of a band of tiles for each worker, the
/// `count` bands straight into the plane.
#[derive(Debug, Clone, Copy)]
enum Layout {
    Waves { wave: usize },
    Bands { count: usize },
}

/// The workers a decode spreads over: the pool's.
#[cfg(feature = "parallel")]
fn threads() -> usize {
    rayon::current_num_threads()
}

/// The workers a decode spreads over: the one thread of a serial build.
#[cfg(not(feature = "parallel"))]
const fn threads() -> usize {
    1
}

impl ImageDecodePlan<'_> {
    /// The most a decode into the stored type `D` holds: of `count` tiles a section
    /// touches, or of the whole image for `None`. A worker takes a run of the wave's
    /// tiles or of the bands with a set of its own, which lives while it decodes the run;
    /// no loop of the decode starts another, so a worker holds one set at a time.
    pub(super) fn decode_capacities<D: DecodeSample>(
        &self,
        count: Option<usize>,
    ) -> Result<DecodeCapacities> {
        let set = self.set_capacities::<D::Wide>()?;
        Ok(match self.layout::<D>(count) {
            Layout::Waves { wave } => DecodeCapacities {
                wave_bytes: wave * self.geometry.max_tile_elements() * size_of::<D>(),
                lengths: wave,
                scatter: self.geometry.tile_capacities(),
                sets: threads().min(wave),
                set,
            },
            Layout::Bands { count } => DecodeCapacities {
                sets: threads().min(count),
                set,
                ..DecodeCapacities::default()
            },
        })
    }

    /// What decoding the largest tile fills of a set, whichever of the container's cells
    /// it is read from and whether or not it has a mask.
    fn set_capacities<W: WidePlane>(&self) -> Result<SetCapacities> {
        let elements = self.geometry.max_tile_elements();
        let lossless = matches!(self.quantization, Some(FloatQuantization::Lossless(_)));
        Ok(SetCapacities {
            tile: self.geometry.tile_capacities(),
            values: elements,
            value_size: size_of::<W>(),
            codecs: self
                .decoder
                .capacities(self.geometry.max_tile(), &self.sources, lossless)?
                .max(self.nulls.capacities(elements)?),
        })
    }

    /// How the tiles of a decode into `D` run: of `count` tiles, or the whole image.
    fn layout<D: DecodeSample>(&self, count: Option<usize>) -> Layout {
        let geom = self.geometry;
        if let Some(count) = count {
            return Layout::Waves {
                wave: self.wave_len::<D>(count),
            };
        }
        let bands = geom.bands();
        if bands.count < threads().min(geom.ntiles()) {
            Layout::Waves {
                wave: self.wave_len::<D>(geom.ntiles()),
            }
        } else {
            Layout::Bands { count: bands.count }
        }
    }

    /// The tiles of a wave of `count`: as many as [`decode_wave_tile_count`] holds, or
    /// one at a time in a serial build.
    ///
    /// [`decode_wave_tile_count`]: crate::compression::decode::decode_wave_tile_count
    fn wave_len<D: DecodeSample>(&self, count: usize) -> usize {
        #[cfg(feature = "parallel")]
        {
            decode::decode_wave_tile_count::<D>(self.geometry).min(count)
        }
        #[cfg(not(feature = "parallel"))]
        {
            count.min(1)
        }
    }

    /// Decode every tile and scatter it into the full image plane.
    ///
    /// The tiles at one index along the slowest axis write one band of the plane and
    /// nothing else, so with enough bands for the workers each takes whole bands and
    /// narrows its tiles straight into its own part of `out`. An image of fewer bands
    /// decodes in waves.
    pub(super) fn decode_all_into<D: DecodeSample>(&self, out: &mut [D]) -> Result<()> {
        let geom = self.geometry;
        if let Layout::Waves { .. } = self.layout::<D>(None) {
            let mut scatter = TileScratch::default();
            scatter.reserve(&geom.tile_capacities())?;
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
        let set = self.set_capacities::<D::Wide>()?;
        let bands = geom.bands();
        let band = |scratch: &mut TileScratchSet<D::Wide>, (band, part): (usize, &mut [D])| {
            scratch.reserve(&set)?;
            self.decode_band(scratch, bands.tiles_of(band), bands.part(band, part))
        };
        #[cfg(feature = "parallel")]
        {
            use rayon::prelude::*;
            out.par_chunks_mut(bands.len)
                .enumerate()
                .try_for_each_init(TileScratchSet::default, band)
        }
        #[cfg(not(feature = "parallel"))]
        {
            let mut scratch = TileScratchSet::default();
            out.chunks_mut(bands.len)
                .enumerate()
                .try_for_each(|item| band(&mut scratch, item))
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
        scatter.reserve(&self.geometry.tile_capacities())?;
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
    /// hand each tile row and its narrowed samples to `scatter`, in order: in waves whose
    /// tiles share one buffer, a slot each, bounded by
    /// [`decode_wave_tile_count`](crate::compression::decode::decode_wave_tile_count).
    fn decode_waves<D: DecodeSample>(
        &self,
        count: usize,
        rows: impl Fn(usize) -> TileRows + Sync,
        mut scatter: impl FnMut(usize, &[D]),
    ) -> Result<()> {
        let slot = self.geometry.max_tile_elements();
        let wave_len = self.wave_len::<D>(count);
        let set = self.set_capacities::<D::Wide>()?;
        let mut wave = allocation::try_zeroed(D::default(), wave_len.saturating_mul(slot))?;
        let mut lengths = allocation::try_zeroed(0, wave_len)?;
        #[cfg(not(feature = "parallel"))]
        let mut scratch = TileScratchSet::default();
        for wave_start in (0..count).step_by(wave_len.max(1)) {
            let tiles = wave_len.min(count - wave_start);
            let decode = |scratch: &mut TileScratchSet<D::Wide>,
                          (offset, (values, length)): (usize, (&mut [D], &mut usize))|
             -> Result<()> {
                scratch.reserve(&set)?;
                self.decode_tile::<D>(scratch, rows(wave_start + offset))?;
                *length = scratch.values.len();
                for (slot, &value) in values.iter_mut().zip(&scratch.values) {
                    *slot = D::narrow(value);
                }
                Ok(())
            };
            #[cfg(feature = "parallel")]
            {
                use rayon::prelude::*;
                wave[..tiles * slot]
                    .par_chunks_mut(slot)
                    .zip(&mut lengths[..tiles])
                    .enumerate()
                    .try_for_each_init(TileScratchSet::default, decode)?;
            }
            #[cfg(not(feature = "parallel"))]
            {
                wave[..tiles * slot]
                    .chunks_mut(slot)
                    .zip(&mut lengths[..tiles])
                    .enumerate()
                    .try_for_each(|item| decode(&mut scratch, item))?;
            }
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
            tile: tile_row,
            encoding: self
                .quantization
                .as_ref()
                .map(|quantization| quantization.encoding(tile_row)),
        };
        let overshoot = D::Wide::decode_tile(job, scratch)?;
        ensure_tile_size(scratch.tile.nelem(), scratch.values.len())?;
        D::fit(&mut scratch.values, overshoot)
    }
}
