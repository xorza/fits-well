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

## Verification

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --tests --all-features
cargo clippy --all-targets --no-default-features --features compression,mmap,internals -- -D warnings
cargo test --tests --no-default-features --features compression,mmap,internals
```

The last two lines are the sequential leg: every feature but `parallel`, whose
`cfg(not(feature = "parallel"))` codec paths `--all-features` never builds.

## Benchmarks

`decode` and `wcs` require `--features internals`, which re-exposes the hot
decode/encode entry points; `compress` and `read` build with the default
features.

```
cargo bench --features internals --bench decode
```
