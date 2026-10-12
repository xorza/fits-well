# fits-well

A **FITS** reader and writer, built for two goals:

1. **Speed** — zero-copy where the format allows, borrowed views into
   caller-owned scratch, single-pass byte-swap and scaling, parallel
   (de)compression, lazy access.
2. **The whole FITS 4.0 standard** — images, ASCII tables, binary tables with
   heap and variable-length arrays, random groups for read, WCS, time
   coordinates, tiled compression.

## Verification

```
cargo fmt --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --tests --all-features
cargo clippy --all-targets --no-default-features --features compression,mmap -- -D warnings
cargo test --tests --no-default-features --features compression,mmap
cargo clippy --all-targets --no-default-features -- -D warnings
cargo clippy -- -D warnings
```

Lines 4 and 5 build the codec paths without `parallel`, which
`--all-features` never does; line 6 builds the crate with no codec at all. The
last lints the build that ships: every other target turns `internals` on
through the self dev-dependency.

## Test code and benches

`internals` is the one feature for what only tests and benches build; the
crate's dev-dependency on itself turns it on in every test and bench build.
That dev-dependency takes no default features, or the lines without them
would build `parallel` after all. Code a bench or an integration test reaches
is gated `#[cfg(feature = "internals")]`; a reach-in only the crate's own unit
tests call is gated `#[cfg(test)]`. Never `any(test, feature = "internals")`.

A bench: `cargo bench --bench decode`; add `--features mmap` for the
memory-mapped arm of the `read` bench.
