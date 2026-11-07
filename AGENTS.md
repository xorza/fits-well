# fits-well

A reader and writer for **FITS**, the standard file format of astronomy.

## Goals

- **The whole FITS 4.0 standard.** Images, ASCII and binary tables with
  variable-length arrays, random groups (read), world coordinates, time
  coordinates, tiled compression of images and tables, and checksums. A feature
  the standard defines is implemented completely, or refused with an error;
  never half-read.
- **Pure Rust.** No C library, no build script, and few dependencies; each new
  one needs a reason the standard library cannot meet.
- **Performance.** Lazy access from the headers alone, zero-copy reads where the
  format allows, one pass for the byte swap and the scaling, and parallel codecs.
  A section or row read touches only the data it needs.
- **Efficient memory.** The caller owns the large buffers and reuses them across
  reads; the reader keeps staging no larger than its largest read. A hot path
  allocates nothing per row, tile or point.
- **Safe on untrusted files.** Every size, count and offset from a file is checked
  before use, so a bad file is an error, never a panic or an oversized allocation.
- **Interoperable.** Files read and write as cfitsio, fpack and astropy read and
  write them; where they differ, `docs/design.md` says which one we follow and why.
- **A standalone crate.** It builds and tests outside darkroom, with its own
  lockfile and profiles. The API takes bytes, readers and writers, never paths.

## Where things are

- `docs/design.md` holds the decisions, each with the options refused and its
  source; `docs/research/` holds the facts they cite.
- `.notes/ISSUES.md` lists the open issues; `.notes/API_SURFACE.md` lists the
  public API (`cargo drive surface`).
- Each bench driver sits in a `bench.rs` beside the code it measures;
  `src/bench/mod.rs` states the rules every case follows.

## Verification

The gate is one chain of plain cargo commands, run from the root. Each is an alias in
`.cargo/config.toml`, and the chain stops at the first that fails:

```
cargo fmt --all && cargo gate-lint && cargo gate-lint-compression && cargo gate-lint-bare && cargo gate-ship && cargo gate-test && cargo gate-test-compression && cargo gate-doc && cargo gate-doctest && scripts/examples.sh
```

Run the aliases rather than cargo with flags of your own: another flag set builds another copy of
the crates. A filtered `cargo gate-test <name>` is the cheapest way to see an edit work. CI runs
the same lanes on three OSes. A change to bench code adds `cargo bench-once`, and `cargo bench-all`
runs the benches. A change to the public API also runs `cargo drive surface`, as CI compares its
output with `.notes/API_SURFACE.md`.

The gate builds three feature sets: all features, `compression` without `parallel`, and no codec at
all. Code must build in each, and its tests pass in the first two.

- **CI runs on [`xorza/rust-ci`](https://github.com/xorza/rust-ci)**: the same lane on every OS, and a failure that
  names its crashes and failed tests. Its README holds the decisions it shares with its other callers.
- **The gate is plain cargo aliases**, as palantir and Darkroom have. Refused: `cargo drive gate`,
  which leaves one repository different and makes CI build the driver first. The driver serves
  `cargo drive surface` alone.

## Test code

`internals` is the one feature for what only tests and benches build; the
crate's dev-dependency on itself turns it on. Code a bench or an integration
test reaches is gated `#[cfg(feature = "internals")]`; a reach-in only the
crate's own unit tests call is gated `#[cfg(test)]`. Never
`any(test, feature = "internals")`.

`tests/` holds what must use the public API alone, or a binary of its own:
the round trips through `fits_well::` paths, and the allocation counts under a
global allocator.

A test name states the behavior: the subject first, the present tense, no
leading article, and no prefix that repeats the module path. A refusal is
`…_refuses_…`, or `…_is_refused` when the subject is the input, as
`docs/design.md` says "Refused". A name covers all that its test checks, or
the test splits.

A test reads through `slice()`. It reads through `stream()` only where it pins
a streaming behavior: staging, seeks or `finish_bytes`. A property that both
sources hold is one generic `fn check<S: Source>(reader: &mut FitsReader<S>)`
that the test calls with each.

A test spells the keywords it expects by hand, as the standard's tables name them:
its independent record. Generating them with the code under test checks that code
against itself.
