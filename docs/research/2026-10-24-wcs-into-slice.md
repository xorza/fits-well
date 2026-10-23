# WCS transforms into the caller's slice

Measured 2026-10-24 on an AMD Ryzen 7 6800U, rustc 1.99.1, with `cargo drive bench
'atomic/wcs/'` (the `bench` profile, `--all-features`). The cases are
`src/world_coordinates/bench.rs`: one 6000-point row a sample, one call a point.

- **Before:** commit `24b45f7^`. `pixel_to_world` and `world_to_pixel` return a new
  `Vec` for each point, and the inverse linear step multiplies by the explicit inverse
  matrix.
- **After:** commit `24b45f7`. Both write into the caller's slice, and the inverse
  linear step solves in place with the matrix's LU factors.

Time per point, the median of criterion's estimate; the `tan` inverse and the `-TAB`
inverse ran twice on each tree and gave the same figures to 0.3%.

| Case | Before (ns) | After (ns) | Change |
|---|---:|---:|---:|
| `to_world/tan` | 99.0 | 79.9 | −19.3% |
| `to_pixel/tan` | 95.6 | 91.7 | −4.0% |
| `to_world/linear` | 38.6 | 13.1 | −65.9% |
| `to_pixel/linear` | 33.8 | 45.4 | +34.4% |
| `to_world/spectral` | 26.6 | 10.1 | −61.9% |
| `to_pixel/spectral` | 20.9 | 11.8 | −43.5% |
| `to_world/tabular` | 102.4 | 85.7 | −16.3% |
| `to_pixel/tabular` | 76 165 | 65 678 | −13.8% |

- Every case but one gets faster: the allocation for each point is gone, and for
  the cheap axes it was most of the cost.
- `to_pixel/linear`, four coupled linear axes, is 34.4% slower: the LU solve is four
  dependent divisions, where the product with the inverse was one multiply-add pass.
  The other inverses solve one or two axes, where the difference does not show.
- `tests/wcs_allocations.rs` counts the allocations: none for a celestial or a
  spectral point, two vectors for a `-TAB` one.
