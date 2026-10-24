//! N-dimensional tile geometry shared by image decompress and both encoders.

use std::ops::Range;

/// The tiling of an N-d image: axis lengths, per-axis tile sizes, and the derived
/// strides and per-axis tile counts. Iterating `0..ntiles()` and calling `tile_into`
/// yields each tile's clipped dimensions and flat pixel indices.
#[derive(Debug)]
pub(super) struct TileGeometry {
    dims: Vec<usize>,
    tiles: Vec<usize>,
    stride: Vec<usize>,
    ntiles_axis: Vec<usize>,
}

/// The runs of an image that the tiles at each index along its slowest axis write,
/// from [`TileGeometry::bands`]: `count` bands of `len` samples, the last one shorter,
/// each written by `tiles` consecutive tiles.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Bands {
    pub(super) count: usize,
    pub(super) len: usize,
    pub(super) tiles: usize,
}

impl Bands {
    /// The tiles of band `band`.
    pub(super) const fn tiles_of(self, band: usize) -> Range<usize> {
        band * self.tiles..(band + 1) * self.tiles
    }

    /// The part `values` of the plane that band `band` fills.
    pub(super) const fn part<D>(self, band: usize, values: &mut [D]) -> PlanePart<'_, D> {
        PlanePart {
            values,
            start: band * self.len,
        }
    }
}

/// The part of an image plane from flat index `start` on, which one band of tiles fills.
#[derive(Debug)]
pub(super) struct PlanePart<'a, D> {
    pub(super) values: &'a mut [D],
    pub(super) start: usize,
}

/// Reusable per-tile scratch, filled by [`TileGeometry::tile_into`] each
/// iteration so a tile loop allocates these buffers once instead of per tile
/// (the `row_bases` buffer is `row_count` long — the dominant per-tile cost).
#[derive(Debug, Default)]
pub(super) struct TileScratch {
    /// Per-axis origin of the current tile (scratch for the index computation).
    pub(super) origin: Vec<usize>,
    /// Edge-clipped per-axis extent of the tile (`ny` fastest); used by HCOMPRESS.
    pub(super) tdims: Vec<usize>,
    /// Flat start of each contiguous tile row in the full image (length = the product
    /// of `tdims[1..]`). Axis 0 has stride 1, so a row is `row_len` contiguous
    /// elements — gather/scatter copy it as a slice instead of per-pixel indexing.
    pub(super) row_bases: Vec<usize>,
    /// Elements per row (`tdims[0]`): the fastest-axis extent.
    pub(super) row_len: usize,
    /// Per-axis local coordinate, the odometer state [`TileGeometry::tile_into`]
    /// walks (over the higher axes) to emit `row_bases` without per-pixel division.
    coord: Vec<usize>,
}

impl TileScratch {
    /// Total pixels in the current tile (`row_len × row_count`).
    pub(super) const fn nelem(&self) -> usize {
        self.row_len * self.row_bases.len()
    }

    /// Write this tile's `values` into `out`, a part of the full image plane, converting
    /// each as it lands. Axis 0 has stride 1, so every row copies as a contiguous slice.
    pub(super) fn scatter_rows_into<S: Copy, D>(
        &self,
        out: &mut PlanePart<'_, D>,
        values: &[S],
        convert: &impl Fn(S) -> D,
    ) {
        let mut offset = 0;
        for &base in &self.row_bases {
            let base = base - out.start;
            for (slot, &value) in out.values[base..base + self.row_len]
                .iter_mut()
                .zip(&values[offset..offset + self.row_len])
            {
                *slot = convert(value);
            }
            offset += self.row_len;
        }
    }

    /// Write only this tile's intersection with `ranges` into a section plane of
    /// `selected_shape`. Rows outside the region are skipped whole; the surviving row
    /// is clipped on axis 0 and copied as one contiguous run.
    pub(super) fn scatter_region_into<S: Copy, D: Copy>(
        &self,
        ranges: &[Range<usize>],
        selected_shape: &[usize],
        values: &[S],
        out: &mut [D],
        convert: &impl Fn(S) -> D,
    ) {
        let x_start = self.origin[0].max(ranges[0].start);
        let x_end = (self.origin[0] + self.tdims[0]).min(ranges[0].end);
        if x_start >= x_end {
            return;
        }
        let width = x_end - x_start;
        for row in 0..self.row_bases.len() {
            let mut remainder = row;
            let mut output_base = 0usize;
            let mut output_stride = selected_shape[0];
            let mut selected = true;
            for axis in 1..self.tdims.len() {
                let local = remainder % self.tdims[axis];
                remainder /= self.tdims[axis];
                let coordinate = self.origin[axis] + local;
                if !ranges[axis].contains(&coordinate) {
                    selected = false;
                    break;
                }
                output_base += (coordinate - ranges[axis].start) * output_stride;
                output_stride *= selected_shape[axis];
            }
            if !selected {
                continue;
            }
            let source = row * self.row_len + (x_start - self.origin[0]);
            let destination = output_base + (x_start - ranges[0].start);
            for (slot, &value) in out[destination..destination + width]
                .iter_mut()
                .zip(&values[source..source + width])
            {
                *slot = convert(value);
            }
        }
    }
}

impl TileGeometry {
    pub(super) fn new(dims: &[usize], tiles: &[usize]) -> Self {
        let n = dims.len();
        let ntiles_axis = dims
            .iter()
            .zip(tiles)
            .map(|(&d, &t)| d.div_ceil(t))
            .collect();
        let mut stride = vec![0usize; n];
        if !dims.contains(&0) && n != 0 {
            stride[0] = 1;
            for i in 1..n {
                stride[i] = stride[i - 1] * dims[i - 1];
            }
        }
        Self {
            dims: dims.to_vec(),
            tiles: tiles.to_vec(),
            stride,
            ntiles_axis,
        }
    }

    pub(super) fn ntiles(&self) -> usize {
        if self.ntiles_axis.contains(&0) {
            0
        } else {
            self.ntiles_axis.iter().product()
        }
    }

    /// The bands of the slowest axis: the tiles at one index along it write one
    /// contiguous run of the image, and no other tile writes into it.
    pub(super) fn bands(&self) -> Bands {
        let Some(last) = self.dims.len().checked_sub(1) else {
            return Bands {
                count: 1,
                len: 1,
                tiles: 1,
            };
        };
        Bands {
            count: self.ntiles_axis[last],
            len: self.tiles[last].min(self.dims[last]) * self.stride[last],
            tiles: self.ntiles_axis[..last].iter().product(),
        }
    }

    #[cfg(any(feature = "parallel", test))]
    pub(super) fn max_tile_elements(&self) -> usize {
        if self.dims.is_empty() {
            return 1;
        }
        self.dims
            .iter()
            .zip(&self.tiles)
            .map(|(&dim, &tile)| dim.min(tile))
            .product()
    }

    /// Fill `s` (reusing its buffers) with tile `t`'s edge-clipped extent and the
    /// flat indices of its pixels in the full image.
    pub(super) fn tile_into(&self, t: usize, s: &mut TileScratch) {
        let n = self.dims.len();
        s.origin.clear();
        s.tdims.clear();
        let mut rem = t;
        for i in 0..n {
            let ti = rem % self.ntiles_axis[i];
            rem /= self.ntiles_axis[i];
            let origin = ti * self.tiles[i];
            s.origin.push(origin);
            s.tdims.push(self.tiles[i].min(self.dims[i] - origin));
        }
        // Axis 0 has stride 1, so each tile row is `row_len` contiguous elements and the
        // tile is `row_count` such rows (the product of the higher-axis extents). Emit only
        // the row starts — walking the *higher* axes as an odometer, `flat` maintained
        // by stride adds with no per-pixel division — and let gather/scatter copy each
        // row as a contiguous slice. The row order matches a row-major pixel walk, so
        // decoded values still land in the right pixels.
        s.row_len = if n == 0 { 1 } else { s.tdims[0] };
        let row_count: usize = if n <= 1 {
            1
        } else {
            s.tdims[1..].iter().product()
        };
        let mut flat: usize = (0..n).map(|i| s.origin[i] * self.stride[i]).sum();
        s.row_bases.clear();
        s.row_bases.reserve_exact(row_count);
        s.coord.clear();
        s.coord.resize(n, 0);
        for _ in 0..row_count {
            s.row_bases.push(flat);
            for i in 1..n {
                s.coord[i] += 1;
                flat += self.stride[i];
                if s.coord[i] < s.tdims[i] {
                    break;
                }
                s.coord[i] = 0;
                flat -= s.tdims[i] * self.stride[i];
            }
        }
    }
}

#[cfg(test)]
mod tests {

    use crate::compression::tile_geometry::*;

    /// dims [5, 4, 3] in tiles [2, 3, 2]: strides [1, 5, 20] and [3, 2, 2] tiles along the
    /// axes. The slowest axis's two tile indices make two bands of 2·20 samples, the
    /// last one 1·20, each written by the 3·2 tiles that share the index, and by no other.
    #[test]
    fn bands_are_the_runs_each_slowest_tile_index_writes() {
        let geom = TileGeometry::new(&[5, 4, 3], &[2, 3, 2]);
        let bands = geom.bands();
        assert_eq!(
            bands,
            Bands {
                count: 2,
                len: 40,
                tiles: 6
            }
        );
        let mut scratch = TileScratch::default();
        for tile in 0..geom.ntiles() {
            geom.tile_into(tile, &mut scratch);
            let band = tile / bands.tiles;
            let run = band * bands.len..(band * bands.len + bands.len).min(60);
            assert!(
                scratch
                    .row_bases
                    .iter()
                    .all(|&base| run.contains(&base) && run.contains(&(base + scratch.row_len - 1))),
                "tile {tile}: {:?} outside {run:?}",
                scratch.row_bases
            );
        }
        // A 0-dimensional image is one band of its one sample.
        assert_eq!(
            TileGeometry::new(&[], &[]).bands(),
            Bands {
                count: 1,
                len: 1,
                tiles: 1
            }
        );
    }

    #[test]
    fn tile_into_rows_match_layout() {
        // Expand the row representation (row_bases + row_len) back to the full flat-index
        // list it stands for, so the hand-computed expectations still describe pixels.
        fn flat(s: &TileScratch) -> Vec<usize> {
            let mut v = Vec::new();
            for &b in &s.row_bases {
                v.extend(b..b + s.row_len);
            }
            v
        }

        // 4×3 image (x fastest, strides [1, 4]) tiled 2×2 → 4 tiles; the top row of tiles
        // is edge-clipped to height 1. Same scratch reused across tiles (no stale rows).
        let geom = TileGeometry::new(&[4, 3], &[2, 2]);
        assert_eq!(geom.ntiles(), 4);
        assert_eq!(geom.max_tile_elements(), 4);
        let mut s = TileScratch::default();
        let expect = [
            (vec![0, 4], 2, vec![0, 1, 4, 5]), // origin (0,0), full 2×2: two rows of 2
            (vec![2, 6], 2, vec![2, 3, 6, 7]), // origin (2,0)
            (vec![8], 2, vec![8, 9]),          // origin (0,2), clipped to height 1
            (vec![10], 2, vec![10, 11]),       // origin (2,2), clipped to height 1
        ];
        for (t, (bases, row_len, pixels)) in expect.iter().enumerate() {
            geom.tile_into(t, &mut s);
            assert_eq!(&s.row_bases, bases, "tile {t} row bases");
            assert_eq!(s.row_len, *row_len, "tile {t} row len");
            assert_eq!(s.nelem(), pixels.len(), "tile {t} nelem");
            assert_eq!(&flat(&s), pixels, "tile {t} pixels");
        }

        // 4×2×2 image (strides [1, 4, 8]) tiled 2×2×2 → 2 tiles: exercises the odometer
        // carrying across the two higher axes; each tile is 4 rows of 2.
        let geom3 = TileGeometry::new(&[4, 2, 2], &[2, 2, 2]);
        assert_eq!(geom3.ntiles(), 2);
        geom3.tile_into(0, &mut s);
        assert_eq!(s.row_bases, vec![0, 4, 8, 12]);
        assert_eq!(flat(&s), vec![0, 1, 4, 5, 8, 9, 12, 13]);
        geom3.tile_into(1, &mut s);
        assert_eq!(flat(&s), vec![2, 3, 6, 7, 10, 11, 14, 15]);

        let empty = TileGeometry::new(&[usize::MAX, usize::MAX, 0], &[1, 1, 1]);
        assert_eq!(empty.ntiles(), 0);
    }
}
