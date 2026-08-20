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

/// Reusable per-tile scratch, filled by [`TileGeometry::tile_into`] each
/// iteration so a tile loop allocates these buffers once instead of per tile
/// (the `row_bases` buffer is `nrows` long — the dominant per-tile cost).
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
    /// Total pixels in the current tile (`row_len × nrows`).
    pub(super) fn nelem(&self) -> usize {
        self.row_len * self.row_bases.len()
    }

    /// Write this tile's `values` into the full image plane `out`, converting each as
    /// it lands. Axis 0 has stride 1, so every row copies as a contiguous slice.
    pub(super) fn scatter_rows_into<S: Copy, D>(
        &self,
        out: &mut [D],
        values: &[S],
        convert: &impl Fn(S) -> D,
    ) {
        let mut offset = 0;
        for &base in &self.row_bases {
            for (slot, &value) in out[base..base + self.row_len]
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
    pub(super) fn new(dims: &[usize], tiles: &[usize]) -> TileGeometry {
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
        TileGeometry {
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
        // tile is `nrows` such rows (the product of the higher-axis extents). Emit only
        // the row starts — walking the *higher* axes as an odometer, `flat` maintained
        // by stride adds with no per-pixel division — and let gather/scatter copy each
        // row as a contiguous slice. The row order matches a row-major pixel walk, so
        // decoded values still land in the right pixels.
        s.row_len = if n == 0 { 1 } else { s.tdims[0] };
        let nrows: usize = if n <= 1 {
            1
        } else {
            s.tdims[1..].iter().product()
        };
        let mut flat: usize = (0..n).map(|i| s.origin[i] * self.stride[i]).sum();
        s.row_bases.clear();
        s.row_bases.reserve_exact(nrows);
        s.coord.clear();
        s.coord.resize(n, 0);
        for _ in 0..nrows {
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
