# The `-TAB` inverse over monotonic runs and pruned cells

Measured 2026-10-24 on an AMD Ryzen 7 6800U with `cargo drive bench
'atomic/wcs/to_(world|pixel)/tabular'`: one 6000-point row a sample through the
`indexed_wcs` fixture, a `WAVE-TAB` axis over a 100 000-entry monotonic index.

- **Before:** commit a37e77f. The 1-D inverse scans the intervals in order for each
  point, and each direction allocates the interpolation location's two vectors.
- **After:** the transform keeps the coordinate array's strictly monotonic runs, and a
  point binary-searches them in order. The 1-D paths allocate nothing.

| Case | Before (row) | After (row) | Before (point) | After (point) |
|---|---:|---:|---:|---:|
| `to_world/tabular` | 513 µs | 405 µs | 85.6 ns | 67.5 ns |
| `to_pixel/tabular` | 476 ms | 428 µs | 79 µs | 71.4 ns |

- The scan cost O(K) a point, 100 000 intervals here; a run search costs O(R·log(K/R))
  for R runs, one here, so the inverse is 1100 times faster and as fast as the forward.
- The result is the scan's: the first interval in array order that holds the point.
  `run_search_matches_the_first_interval_scan` compares the two over arrays that turn,
  repeat a value and repeat an index value.
- A coupled table's inverse once spent 16 work units of its 2²² budget on each cell of
  a two-axis table, so a point past 2¹⁸ = 262 144 cells was `WcsNoConvergence`. The
  cells whose level-0 corners cannot hold the point now cost nothing, and a point in
  the last of 512 × 512 cells inverts in 65 ms in the debug profile. Its search
  allocates three buffers, where it allocated 13.
