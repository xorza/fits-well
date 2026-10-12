# fits-well

A **FITS** reader and writer, built for two goals:

1. **Speed** — zero-copy where the format allows, borrowed views into
   caller-owned scratch, single-pass byte-swap and scaling, parallel
   (de)compression, lazy access.
2. **The whole FITS 4.0 standard** — images, ASCII tables, binary tables with
   heap and variable-length arrays, random groups for read, WCS, time
   coordinates, tiled compression.

## Verification

`x.py` at the root runs every check (`python x.py` on Windows). `gate` runs,
on the packages the working tree changed: rustfmt in place, clippy on every
target per lint set, plain clippy for the build that ships, rustdoc with
private items under `-D warnings`, the tests per test set, and the doc tests.
It stops after the first step that fails. Each cargo call prints one line, a
failure adds only its diagnostics or failed tests, and the full output is in
`target/x/last/`.

```
./x.py gate                        # the changed packages
./x.py gate -p <pkg>               # named packages; --workspace for every one
./x.py test -p <pkg> -- <filter>   # one step: fmt, lint, doc, test, nextest, doctest
./x.py test --lib                  # or --test <name>, in place of the set's targets
./x.py bench -- <filter>           # --smoke runs each case once, --no-run prints the binary
./x.py watch                       # rebuilds on change, so the cache stays warm
```

Check through `x.py`, not cargo: each call is one package with fixed flags,
so every run reuses the build `watch` keeps warm, and another flag set builds
another copy of the crate. A package's feature sets are in its `Cargo.toml`,
under `[package.metadata.x]`. `x.py` is one file, the same in darkroom and
in each submodule: a change to it is copied to the others.

The second lint and test sets build the codec paths without `parallel`, which
`--all-features` never does; the third lint set builds the crate with no codec
at all. The plain clippy lints the build that ships: every other target turns
`internals` on through the self dev-dependency. A change to bench code adds
`./x.py bench --smoke`, which runs each case once. `.cargo/config.toml` builds
x86-64 for x86-64-v3, the CPU darkroom builds for.

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
rules. `./x.py bench` runs them with every feature, so a case always measures
the same build. An id is `<kind>/<group>/<case>`, the case the variant (a
source, where a group has several) then the input: an `atomic` case times one
operation on the unit the read and write paths hand it (one row of the
standard frame, which is also a codec's default tile); an `integration` case
times a public entry point end to end on the 24 MP standard frame or the
source catalog, from each source (`stream`, `slice`, `mmap`) or to a `file`. A
filter selects by id: `./x.py bench -- --list`, `-- '^atomic/'`,
`-- --exact integration/read_section/stream/i16`.
