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
core crate is dependency-free by default (the `compression` feature pulls in
`flate2`). Typed image read/write is done (decode/encode +
`BSCALE`/`BZERO`). Binary and ASCII tables read and write; multi-HDU files
(primary + `IMAGE`/`TABLE`/`BINTABLE` extensions) write; binary-table `P`/`Q` heap
arrays and per-column `TSCAL`/`TZERO` decode; random groups read; `CONTINUE`,
`HIERARCH`, and `CHECKSUM`/`DATASUM` (verify + write) are supported. A typed
**WCS** layer does pixel↔world for 23 projections — zenithal
`TAN`/`SIN`/`ARC`/`STG`/`ZEA`/`ZPN`/`AIR`, zenithal-perspective `AZP`/`SZP`,
cylindrical `CAR`/`CEA`/`MER`/`SFL`/`CYP`, all-sky `AIT`/`MOL`/`PAR`, conic
`COP`/`COE`/`COD`/`COO`, pseudoconic `BON`, polyconic `PCO` — with `PC`/`CD`/`CROTA`
and full `PVi_m` parameters, yielding coordinates in the frame the file declares
(`RADESYS`/`EQUINOX`). A typed **time** layer
handles ISO-8601/JD/MJD, epochs, `UTC`…`TCB`/`GPS`/UT1 scale conversions, and time
WCS axes — both validated against astropy. Tiled **image and table** compression
work behind
the `compression` feature: all five image codecs (`GZIP_1`, `GZIP_2`, `RICE_1`,
`PLIO_1`, `HCOMPRESS_1` with `SMOOTH=1` decode), quantized-float read+write
(`NO_DITHER`/`SUBTRACTIVE_DITHER_1`/`SUBTRACTIVE_DITHER_2`, `ZBLANK`/NaN), and §10.3
fixed-width table compression. The remaining WCS frontier (quad-cube/HEALPix
projections, non-linear spectral axes — both of which error cleanly today) is
charted in the module map below, which shows what is built versus planned. The
design principles in this file remain the spec; follow them when filling the
scaffolds in.

**Out of scope (deliberately):** converting *between* celestial reference frames
(FK4↔FK5↔Galactic↔ICRS — precession, E-terms, frame bias) is astrometry, not part
of the FITS standard. The WCS layer parses `RADESYS`/`EQUINOX` and returns world
coordinates in the file's own declared frame; transforming them into a different
frame is the job of an astrometry library (astropy `SkyCoord`, ERFA), not this one.

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
# also exercise the optional codecs:
cargo test --features compression && cargo clippy --all-targets --features compression -- -D warnings
```

## The FITS format in one screen

Read this before touching parsing/writing code; the full reference lives in
[`docs/refs/`](docs/refs/) — curated, implementation-focused markdown indexed by
[`docs/refs/README.md`](docs/refs/README.md). The FITS 4.0 standard itself is
included verbatim as both [`docs/refs/fits_standard40.md`](docs/refs/fits_standard40.md)
(full PDF→markdown conversion with reconstructed TOC, handy for grep/linking) and
the normative [`docs/refs/fits_standard40.pdf`](docs/refs/fits_standard40.pdf).

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

The conformance audit in [`docs/conformance.md`](docs/conformance.md) maps each
reference file to the code that implements it, flags gaps (with severity and
`file:line` anchors), and rates test coverage — check it before treating a
section as spec-complete.

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

Most modules are directories — `<name>/{mod.rs, tests.rs}` — with the tests
split out per the global rule; single-file modules keep the `.rs` suffix below.

| Module | Role | Status |
|--------|------|--------|
| `block.rs` | 2880-byte grid, padding, rounding math | done |
| `bitpix.rs` | `BITPIX` element type + element sizes | done |
| `endian.rs` | big-endian scalar (de)serialization shared by image/table/compression decode + encode | done |
| `header/` | ordered card model (`value.rs`, `card/`, `mod.rs`): parse/render, `CONTINUE` folding, `HIERARCH` compound keys, keyword index, typed getters + builder | done |
| `hdu/` | HDU classification + data-unit sizing (Eq. 2, incl. random groups) | done |
| `reader/` | lazy seeking HDU scan; `read_image`/`read_table`/`read_ascii_table`/`read_groups`/`read_compressed_image`/`read_compressed_table`/`verify_checksum`, raw `DataUnit` | done |
| `writer/` | multi-HDU writer: `write_image`/`write_table` (fixed + `P` VLA columns)/`write_ascii_table`/`write_compressed_image`(`_lossy`)/`write_compressed_table`, `with_checksums` | done |
| `data/` | typed `Image`/`ImageData`, big-endian decode+encode, `BSCALE`/`BZERO` physical plane | image read+write done; SIMD/parallel TODO |
| `table/` | `BINTABLE` parsing (`Tform`/`Column`); fixed-width decode (`ColumnData`), `TSCAL`/`TZERO` physical plane, `P`/`Q` heap VLAs | read done (write in `writer/`) |
| `ascii/` | `TABLE` (ASCII) read: `TBCOLn`/Fortran `TFORMn` → `AsciiColumn`/`ColumnData` | read done (write in `writer/`) |
| `groups/` | random-groups (§6) read: params + arrays, `PSCALn`/`PZEROn` physical | read done (no write — deprecated) |
| `checksum.rs` | `DATASUM`/`CHECKSUM` ones'-complement accumulate + Appendix-J encode | done |
| `compress/` (feature `compression`) | tiled image+table (de)compress: `gzip`/`rice`/`plio`/`hcompress` codecs, `quantize` (float), `table` (§10.3), reassembly + encode | all 5 image codecs read+write; float quant all 3 dither methods + `ZBLANK`; HCOMPRESS `SMOOTH=1` decode + lossy `SCALE>0` write; fixed-width table compression read+write |
| `wcs/` | typed WCS: keyword parse, linear transform (PC/CD/CROTA + `PVi_m` + inverse), 23 projections (zenithal + perspective AZP/SZP + cylindrical + all-sky + conic + BON + PCO) via general pole computation, `pixel_to_world`/`world_to_pixel`; unimplemented codes → `UnsupportedProjection` | v2 done (quad-cube/HEALPix, spectral TODO; inter-frame transforms out of scope) |
| `time/` | typed time (§9): `Datetime` (ISO-8601↔JD/MJD), `Epoch` (J/B), `TimeScale` conversions (UTC↔TAI leap table, TT/TCG/TDB/TCB/GPS/UT1), `FitsTime` header view + time WCS axis | v2 done |
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
- **Feature-flag only the layers that carry a dependency.** Tiled compression
  (`RICE_1`, `GZIP`, `HCOMPRESS`, `PLIO`) pulls in `flate2`, so it lives behind the
  `compression` feature. WCS (§8) and time (§9) are dependency-free pure math and
  part of the standard, so they are always compiled (no feature gate); the whole
  crate stays dependency-free by default regardless.
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
All three are implemented: `CONTINUE` long strings (read + write), `HIERARCH`
compound keywords (parsed to a space-joined key, read + write), and
`CHECKSUM`/`DATASUM` (verify on read; solve + write via `with_checksums`).
Purely-registered conventions beyond these stay optional, feature-flagged layers.
