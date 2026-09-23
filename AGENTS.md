# fits-well

A **FITS** reader and writer. Two non-negotiable goals shape every decision:

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

The last two lines build the `cfg(not(feature = "parallel"))` codec paths,
which `--all-features` never does.

The `decode` and `wcs` benches need `--features internals`:
`cargo bench --features internals --bench decode`.
