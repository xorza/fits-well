# fits-well

A Rust library to **read and write FITS** (Flexible Image Transport System)
files — the standard data format of astronomy. Two non-negotiable goals shape
every decision:

1. **Blazing fast** — zero-copy where the format allows, borrowed read views
   into caller-owned reusable scratch, single-pass byte-swap / scaling,
   parallel (de)compression, lazy access.
2. **Whole-standard coverage** — the full **FITS 4.0** standard (images, ASCII
   tables, binary tables with heap/variable-length arrays, random groups for
   read, WCS, time coordinates, tiled compression).
