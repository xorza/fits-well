# fits-well

A **FITS** reader and writer, built for two goals:

1. **Speed** — zero-copy where the format allows, borrowed views into
   caller-owned scratch, single-pass byte-swap and scaling, parallel
   (de)compression, lazy access.
2. **The whole FITS 4.0 standard** — images, ASCII tables, binary tables with
   heap and variable-length arrays, random groups for read, WCS, time
   coordinates, tiled compression.

## Verification

`cargo drive` runs every check: the driver is the `drive` submodule at the
root, the same repository in every project. Run it from the root.

```
cargo drive check          # after each change: the root's `check` lanes
cargo drive test [args]    # the tests, each time; the runner's arguments pick some: a test name
cargo drive bench [args]   # the benches; a filter, or --test to run each case once
cargo drive gate           # the full check, as CI runs it: the root's `gate` lanes, fmt --check
```

`check` takes no arguments: each lane covers the whole workspace, and the
workspace of each path dependency outside it, in its own folder. A step does
not run again when no input changed since it passed; under nextest, the tests
run only for the crates a change reaches and every crate that depends on them.
Each step prints one line; a failure prints each diagnostic once and names its
log, `target/drive/log/<step>.log`. Read the log rather than run cargo by
hand: another shape of a cargo command builds another copy of the crates.

A root's lanes are in its `Cargo.toml`, under `[workspace.metadata.drive]`,
or `[package.metadata.drive]` in a root that is one package. Its `check` list
runs fmt, lint, ship, tests and doc tests by default, and its `gate` list adds
rustdoc. `test-runner` is `nextest` (the default) or `cargo-test`, which needs
nothing installed but runs every test when a change reaches any crate. Every
lane runs `--locked`: after a change to the dependencies,
`cargo update --workspace` writes the lock.

The second lint and test sets build the codec paths without `parallel`, which
`--all-features` never does; the third lint set builds the crate with no codec
at all. The plain clippy lints the build that ships: every other target turns
`internals` on through the self dev-dependency. A change to bench code adds
`cargo drive bench --test`, which runs each case once. `.cargo/config.toml` builds
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
rules. `cargo drive bench` runs them with every feature, so a case always measures
the same build. An id is `<kind>/<group>/<case>`, the case the variant (a
source, where a group has several) then the input: an `atomic` case times one
operation on the unit the read and write paths hand it (one row of the
standard frame, which is also a codec's default tile); an `integration` case
times a public entry point end to end on the 24 MP standard frame or the
source catalog, from each source (`stream`, `slice`, `mmap`) or to a `file`. A
filter selects by id: `cargo drive bench --list`, `'^atomic/'`,
`--exact integration/read_section/stream/i16`.
