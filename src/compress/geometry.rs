//! N-dimensional tile geometry shared by image decompress and both encoders.

/// The tiling of an N-d image: axis lengths, per-axis tile sizes, and the derived
/// strides and per-axis tile counts. Iterating `0..ntiles()` and calling `tile_into`
/// yields each tile's clipped dimensions and flat pixel indices.
#[derive(Debug)]
pub(crate) struct TileGeometry {
    dims: Vec<usize>,
    tiles: Vec<usize>,
    stride: Vec<usize>,
    ntiles_axis: Vec<usize>,
}

/// Reusable per-tile scratch, filled by [`TileGeometry::tile_into`] each
/// iteration so a tile loop allocates these buffers once instead of per tile
/// (the `row_bases` buffer is `nrows` long — the dominant per-tile cost).
#[derive(Debug, Default)]
pub(crate) struct TileScratch {
    /// Per-axis origin of the current tile (scratch for the index computation).
    origin: Vec<usize>,
    /// Edge-clipped per-axis extent of the tile (`ny` fastest); used by HCOMPRESS.
    pub(crate) tdims: Vec<usize>,
    /// Flat start of each contiguous tile row in the full image (length = the product
    /// of `tdims[1..]`). Axis 0 has stride 1, so a row is `row_len` contiguous
    /// elements — gather/scatter copy it as a slice instead of per-pixel indexing.
    pub(crate) row_bases: Vec<usize>,
    /// Elements per row (`tdims[0]`): the fastest-axis extent.
    pub(crate) row_len: usize,
    /// Per-axis local coordinate, the odometer state [`TileGeometry::tile_into`]
    /// walks (over the higher axes) to emit `row_bases` without per-pixel division.
    coord: Vec<usize>,
}

impl TileScratch {
    /// Total pixels in the current tile (`row_len × nrows`).
    pub(crate) fn nelem(&self) -> usize {
        self.row_len * self.row_bases.len()
    }
}

impl TileGeometry {
    pub(crate) fn new(dims: &[usize], tiles: &[usize]) -> TileGeometry {
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

    pub(crate) fn ntiles(&self) -> usize {
        if self.ntiles_axis.contains(&0) {
            0
        } else {
            self.ntiles_axis.iter().product()
        }
    }

    #[cfg(feature = "parallel")]
    pub(crate) fn scatter_tiles<T>(&self, tiles: &[Vec<T>], out: &mut [T])
    where
        T: Copy + Send + Sync,
    {
        use rayon::prelude::*;

        debug_assert_eq!(tiles.len(), self.ntiles());
        let row_len = self.dims[0];
        let x_tiles = self.ntiles_axis[0];
        out.par_chunks_mut(row_len)
            .enumerate()
            .for_each(|(row, out)| {
                let mut coordinates = row;
                let mut tile_base = 0;
                let mut tile_stride = x_tiles;
                let mut source_row = 0;
                let mut source_stride = 1;
                for axis in 1..self.dims.len() {
                    let coordinate = coordinates % self.dims[axis];
                    coordinates /= self.dims[axis];
                    let tile_coordinate = coordinate / self.tiles[axis];
                    let local_coordinate = coordinate % self.tiles[axis];
                    tile_base += tile_coordinate * tile_stride;
                    tile_stride *= self.ntiles_axis[axis];
                    source_row += local_coordinate * source_stride;
                    source_stride *=
                        self.tiles[axis].min(self.dims[axis] - tile_coordinate * self.tiles[axis]);
                }
                for x_tile in 0..x_tiles {
                    let destination = x_tile * self.tiles[0];
                    let width = self.tiles[0].min(row_len - destination);
                    let source = source_row * width;
                    out[destination..destination + width]
                        .copy_from_slice(&tiles[tile_base + x_tile][source..source + width]);
                }
            });
    }

    /// Fill `s` (reusing its buffers) with tile `t`'s edge-clipped extent and the
    /// flat indices of its pixels in the full image.
    pub(crate) fn tile_into(&self, t: usize, s: &mut TileScratch) {
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
        s.row_bases.reserve(nrows);
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
