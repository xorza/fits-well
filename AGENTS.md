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
through the self dev-dependency. A change to bench code adds
`cargo benches -- --test`, which runs each case once. `.cargo/config.toml`
builds x86-64 for x86-64-v3, the CPU darkroom builds for.

## Test code

`internals` is the one feature for what only tests and benches build; the
crate's dev-dependency on itself turns it on in every test and bench build.
That dev-dependency takes no default features, or the lines without them
would build `parallel` after all. Code a bench or an integration test reaches
is gated `#[cfg(feature = "internals")]`; a reach-in only the crate's own unit
tests call is gated `#[cfg(test)]`. Never `any(test, feature = "internals")`.

## Benches

One target runs every case through `bench::run`; each driver sits in a
`bench.rs` beside the code it measures, and `src/bench/mod.rs` states the
rules. `cargo benches` runs them with every feature, so a case always measures
the same build. An id is `<kind>/<group>/<case>`, the case the variant (a
source, where a group has several) then the input: an `atomic` case times one
operation on the unit the read and write paths hand it (one row of the
standard frame, which is also a codec's default tile); an `integration` case
times a public entry point end to end on the 24 MP standard frame or the
source catalog, from each source (`stream`, `slice`, `mmap`) or to a `file`. A
filter selects by id: `cargo benches -- --list`, `-- '^atomic/'`,
`-- --exact integration/read_section/stream/i16`.
