# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## What this is

`fits` is a Rust library to **read and write FITS** (Flexible Image Transport
System) files — the standard data format of astronomy. The two non-negotiable
goals shape every decision:

1. **Blazing fast** — zero-copy where the format allows, SIMD bulk byte-swap /
   scaling, parallel-friendly decode, lazy HDU access via seeking.
2. **Whole-standard coverage** — the full **FITS 4.0** standard (images, ASCII
   tables, binary tables with heap/variable-length arrays, random groups for
   read, WCS, time coordinates, tiled compression).

The structural spine is built and tested: the 2880-byte block layer, an ordered
header model (with `CONTINUE` long-string read/write), HDU classification and
boundary sizing, a lazy seeking reader, and a header / raw-data-unit writer. The
core crate is dependency-free. Typed image read/write is done (decode/encode +
`BSCALE`/`BZERO`), and binary tables are read (fixed-width columns, `TSCAL`/`TZERO`
scaling, and `P`/`Q` heap arrays); table *writing*, WCS, and tiled compression are
scaffolded — the module map below shows what is built versus planned, and
[`docs/ROADMAP.md`](docs/ROADMAP.md) lays out the path to feature-complete. The design principles there remain the spec; follow them when
filling the scaffolds in.

## Commands

```bash
cargo build                      # debug build
cargo build --release            # optimized — benchmark against this, never debug
cargo test                       # run all tests
cargo test <name>                # run tests matching a substring
cargo test --lib module::tests   # run one module's tests
cargo bench                      # run benchmarks (once criterion benches exist)
cargo doc --open                 # render API docs
```

Before confirming any change is done, run the full gate (per global rules):

```bash
cargo test && cargo fmt --all && cargo check && cargo clippy --all-targets -- -D warnings
```

## The FITS format in one screen

Read this before touching parsing/writing code; the full reference lives in
[`docs/refs/`](docs/refs/) (curated markdown) with the normative PDF at
`docs/refs/fits_standard40.pdf`.

- A file is a sequence of **HDUs** (Header/Data Units). HDU 0 is the **primary**
  (`SIMPLE = T`); the rest are **extensions** (`XTENSION = 'IMAGE'|'TABLE'|'BINTABLE'`).
- Everything is laid out on a **2880-byte block** grid (= 36 × 80-byte records).
  Header and data units are each padded up to a block multiple (headers with
  spaces; data with NULs, except ASCII-table data padded with spaces).
- A **header** is 80-byte ASCII keyword records (`KEYWORD = value / comment`),
  ending in `END`.
- **Data** is big-endian. `BITPIX` ∈ {8, 16, 32, 64, −32, −64} sets element type
  (8 = unsigned byte; 16/32/64 = signed two's-complement; ±32/±64 = IEEE float).
- Physical value = `BZERO + BSCALE × stored`. Unsigned ints are faked via a
  `BZERO`/`TZERO` offset of `2^(n-1)` with scale 1 — detect and expose as `uN`.
- **Binary tables** carry typed, optionally array-valued columns (`TFORMn`
  codes `LXBIJKAEDCMPQ`) plus a **heap** for variable-length arrays (`P`/`Q`).

Quick map of the reference notes:

| Topic | File |
|-------|------|
| File/HDU/block structure | `docs/refs/01-file-structure.md` |
| Header & keyword syntax | `docs/refs/02-headers-keywords.md` |
| BITPIX, scaling, endianness, unsigned trick | `docs/refs/03-data-representation.md` |
| Images / IMAGE / random groups | `docs/refs/04-images.md` |
| ASCII tables | `docs/refs/05-ascii-tables.md` |
| Binary tables, heap, VLAs | `docs/refs/06-binary-tables.md` |
| WCS / time / compression | `docs/refs/07-wcs-time-compression.md` |
| CONTINUE / CHECKSUM / HIERARCH conventions | `docs/refs/08-conventions.md` |

## Architecture

The format's structure maps cleanly onto modules. Keep layers separate so the
hot decode path stays lean and optional semantics (WCS, compression) are opt-in.

```
bytes  ──►  block layer   ──►  HDU layer   ──►  header model   ──►  typed data
            (2880 grid,        (boundary       (ordered            (images,
             padding,           scan, lazy      records +           tables,
             I/O quantum)       seeking)        keyword index)      heap, VLAs)
```

### Module layout (`src/`)

| Module | Role | Status |
|--------|------|--------|
| `block.rs` | 2880-byte grid, padding, rounding math | done |
| `bitpix.rs` | `BITPIX` element type + element sizes | done |
| `header/{value,card,mod}.rs` | ordered card model, parse/render, `CONTINUE` folding, keyword index, typed getters + builder (`set`/`comment`/`push_*`) | done |
| `hdu.rs` | HDU classification + data-unit sizing (Eq. 2, incl. random groups) | done |
| `reader.rs` | lazy seeking HDU scan; `DataUnit` fetch, `read_image`, `read_table` | done |
| `writer.rs` | header + data-unit serialization; `write_image` (primary array) | image write done; tables/extension write TODO |
| `data.rs` | typed `Image`/`ImageData`, big-endian decode+encode, `BSCALE`/`BZERO` physical plane | image read+write done; SIMD/parallel TODO |
| `table.rs` | `BINTABLE` parsing (`Tform`/`Column`); fixed-width decode (`ColumnData`), `TSCAL`/`TZERO` physical plane, `P`/`Q` heap VLAs | read done; table write TODO |
| `error.rs` | `FitsError` + `Result` | done |

`lib.rs` is the only place that defines the public surface (`pub use`). Card
rendering is free-format today, so header round-trips reproduce the *model*
exactly but not yet the original byte layout.

Design principles specific to this crate:

- **Two value planes everywhere: raw and physical.** Expose zero-copy raw access
  (typed slice over the source buffer) for the common `scale==1, zero==0,
  endianness-matches-host` case; decode into an owned buffer only when scaling or
  byte-swapping is actually required. Never force callers through float scaling
  they didn't ask for.
- **Lazy by default.** HDU boundaries are computable from headers alone
  (`|BITPIX|·GCOUNT·(PCOUNT + Π NAXISn)` rounded to a block) — never read data to
  find the next HDU. Support `Read + Seek` and memory-mapped sources.
- **Headers round-trip exactly.** Model a header as an *ordered list* of records
  with a side index for lookup — not a hash map. Duplicate `COMMENT`/`HISTORY`
  and record order are significant and must be preserved byte-for-byte.
- **SIMD/parallel the bulk ops.** Endian swap + `BSCALE/BZERO` (and per-column
  `TSCAL/TZERO`) are embarrassingly parallel; tile images and table columns for
  multi-threaded decode. Gate threading behind a feature, keep a scalar fallback.
- **Feature-flag the heavy layers.** Core read/write of images+tables stays
  dependency-light; put WCS math and tiled compression (`RICE_1`, `GZIP`,
  `HCOMPRESS`, `PLIO`) behind features so the base crate is small.
- **"Once FITS, always FITS."** The format never breaks backward compatibility.
  Keep reading legacy structures (random groups, `SIMPLE = F`) forever; just
  don't *write* deprecated forms.

## Correctness expectations

FITS is full of fiddly invariants that silent bugs hide in — test them explicitly
(this is also mandated by the global Rust rules):

- Block padding: assert every written unit is a 2880 multiple, padded with the
  correct fill byte (space for headers/ASCII-table data, NUL for other data).
- Round-trip: parse→write→parse must reproduce headers byte-for-byte and data
  bit-for-bit (including float NaN/Inf, `BLANK`, unsigned offsets).
- Cross-check decoders against known-good files (CFITSIO/astropy outputs) and
  against hand-computed values for small fixtures — never `result < N` assertions.
- Boundary cases: `NAXIS = 0` (no data), zero-length axes, `TFORM` repeat count 0,
  empty heap, `PCOUNT = 0`, maximum 999 columns/axes.

## Conventions registry

Real files lean on a few near-ubiquitous conventions — `CONTINUE` long strings
(now normative, §4.2.1.2), `CHECKSUM`/`DATASUM` integrity keywords (§4.4.2.7 +
Appendix J), and the registered `HIERARCH` long-keyword convention. These are
covered in `docs/refs/08-conventions.md`; the full registry (Green Bank,
inheritance, ESO, …) is at <https://fits.gsfc.nasa.gov/fits_registry.html>.
Support the in-standard ones first and treat purely-registered ones as optional,
feature-flagged layers. `CONTINUE` long strings are implemented (read and
write); `CHECKSUM`/`DATASUM` and `HIERARCH` are not yet — a `HIERARCH` card
currently falls back to a commentary record so the file stays readable.
