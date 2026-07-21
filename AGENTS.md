# AGENTS.md

This file provides guidance to AI coding agents when working with code in this repository.

## What this is

`fits-well` is a Rust library to **read and write FITS** (Flexible Image Transport
System) files — the standard data format of astronomy. The two non-negotiable
goals shape every decision:

1. **Blazing fast** — zero-copy where the format allows, a borrowed read view
   (`read_image_view` → `BorrowedImage`) that byte-swaps into a caller-owned reused
   scratch so a hot read loop isn't page-fault-bound, single-pass byte-swap / scaling, tile-parallel
   (de)compression, reused read/write scratch buffers, lazy HDU access via seeking.
2. **Whole-standard coverage** — the full **FITS 4.0** standard (images, ASCII
   tables, binary tables with heap/variable-length arrays, random groups for
   read, WCS, time coordinates, tiled compression).

The structural spine is built and tested: the 2880-byte block layer, an ordered
header model (with `CONTINUE` long-string read/write), HDU classification and
boundary sizing, a lazy seeking reader, and a header / raw-data-unit writer. The
default build enables `compression` + `parallel` (pulling `flate2` + `rayon`), with
opt-in `mmap` (memmap2, zero-copy memory-mapped reads) off by default;
`--no-default-features` gives the pure-Rust core
(block / header / HDU / reader / writer / WCS / time) — whose only unconditional
dependencies are `bitvec` (packed `X` bit-array columns) and `num-complex`
(`C`/`M` complex columns). Typed image read/write is done (decode/encode +
`BSCALE`/`BZERO`). Binary and ASCII tables read and write; multi-HDU files
(primary + `IMAGE`/`TABLE`/`BINTABLE` extensions) write; binary-table `P`/`Q` heap
arrays and per-column `TSCAL`/`TZERO` decode; random groups read; normative
`CONTINUE` and `CHECKSUM`/`DATASUM` (verify + write) are supported, while
non-standard `HIERARCH` records remain opaque commentary. A typed
**WCS** layer does pixel↔world for all 27 FITS 4.0 projections — zenithal
`TAN`/`SIN`/`ARC`/`STG`/`ZEA`/`ZPN`/`AIR`, zenithal-perspective `AZP`/`SZP`,
cylindrical `CAR`/`CEA`/`MER`/`SFL`/`CYP`, all-sky `AIT`/`MOL`/`PAR`, conic
`COP`/`COE`/`COD`/`COO`, pseudoconic `BON`, polyconic `PCO`, cube
`TSC`/`CSC`/`QSC`, and HEALPix `HPX` — with `PC`/`CD`/`CROTA`
and full `PVi_m` parameters, yielding coordinates in the frame the file declares
(`RADESYS`/`EQUINOX`), exposed as typed declared-frame metadata. It also evaluates
every Table-26 spectral/detector algorithm (`F2*`/`W2*`/`V2*`/`A2*`, `GRI`/`GRA`,
and `LOG`), exposes per-axis spectral frame/rest metadata, and resolves
multidimensional `-TAB` arrays through `FitsReader::read_wcs`. A typed **time**
layer handles strict FITS ISO-8601/JD/MJD, epochs, declared scale metadata,
resolved reference positions, all image/table PHASE forms, and time WCS axes.
Inter-scale chronometry remains the responsibility of a library with current
leap-second and ephemeris data. Tiled
**image and table** compression
work behind
the `compression` feature: all five image codecs (`GZIP_1`, `GZIP_2`, `RICE_1`,
`PLIO_1`, `HCOMPRESS_1` with a signed 64-bit transform and `SMOOTH=1` decode),
quantized-float read+write
(`NO_DITHER`/`SUBTRACTIVE_DITHER_1`/`SUBTRACTIVE_DITHER_2`, `ZBLANK`/NaN),
`NULL_PIXEL_MASK` restoration, and §10.3 fixed-width plus P/Q heap-array table
compression. All tile (de)compression fans out across the rayon
pool under the default-on `parallel` feature (a scalar fallback runs without it),
which the codec benches measure at ~2.5–3× on decompress and ~4–6.5× on compress.
The standard WCS algorithms are complete; convention-only `XPH` remains readable,
is flagged in `unsupported_axes`, and makes complete transforms reject it. The
module map below shows what is built versus planned. The design principles in
this file remain the spec; follow them when filling the scaffolds in.

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
cargo bench --features internals # throughput benches (decode path + codecs)
cargo doc --open                 # render API docs
```

Before confirming any change is done, run the full gate (per global rules). The
default features now include `compression` + `parallel`, so the first line already
exercises the codecs on the rayon path:

```bash
cargo test && cargo fmt --all && cargo check && cargo clippy --all-targets -- -D warnings
# also check the dependency-free core and the serial-codec (no-rayon) fallback:
cargo test --no-default-features
cargo clippy --all-targets --no-default-features -- -D warnings
cargo test --no-default-features --features compression
# the opt-in (non-default) feature builds — memory-mapped reads and the n-D bridge:
cargo clippy --all-targets --features mmap -- -D warnings
# microbench entry points (decode/encode/read_image) compile under `internals`:
cargo clippy --all-targets --features internals -- -D warnings
```

## Changelog

- **Update `CHANGELOG.md` in the same change whenever the public API or externally
  observable behavior changes.** Purely internal refactors and performance changes
  that preserve behavior do not require an entry.

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
| `keyword.rs` | stack-allocated indexed-keyword formatting (`key!` macro / `KeyBuf`): builds `NAXISn`/`PVi_m`/`CTYPEn`-style keys without the per-lookup `format!` heap alloc (one WCS parse does ~90) | done |
| `header/` | ordered card model (`value.rs`, `card/`, `mod.rs`): fallible standard-keyword authoring, parse/render, lossless logical `CONTINUE` folding, opaque non-standard commentary, keyword index, typed getters | done |
| `hdu/` | role-aware HDU classification + primary/extension/random-groups data-unit sizing (Eqs. 1/2/4) | done |
| `reader/` | HDU scan over a `Source` (`source.rs`: `StreamSource` copies, `SliceSource`/`MmapSource` borrow zero-copy); `open`/`from_bytes`/`open_mmap`; indexed operations plus `EXTNAME`/`EXTVER` lookup through `hdu_index`; `read_image` (owned, transparently decompresses a `ZIMAGE` `CompressedImage`)/`read_image_view(idx, &mut Vec<u64>)` (`BorrowedImage` metadata + `ImageView` swapped into caller-owned scratch); N-D image sections with selected compressed tiles; source-bound binary-table schema/cell/column/row/range reads; `read_wcs` (including referenced `-TAB` BINTABLE arrays)/`read_table`/`read_ascii_table`/`read_groups`/`read_compressed_table`/`verify_checksum`, raw `DataUnit`; source recovery via `into_inner`/`into_bytes` | done |
| `writer/` | multi-HDU writer: `mod.rs` coordinates HDU commit/checksum state; `image.rs` handles typed/compressed images and seekable `ImageStream`; `table.rs` and `ascii.rs` own their builders, validation, and encoding (including fixed + `P`/`Q` VLAs and jagged bit arrays) | done |
| `data/` | typed owned `Image`/`ImageData`/`ReadImage` (zero-copy raw plane) + scratch-backed `BorrowedImage` (`ImageMetadata` + `ImageView` over caller-owned aligned storage), big-endian decode+encode (`encode_into` reuses the writer's buffer), `BSCALE`/`BZERO` physical plane + `SampleType`/`UnsignedData` resolution | image read+write done; the read-loop lever is `read_image_view`→`BorrowedImage` — owned `read_image().decode()` is page-fault-bound (~65% of cycles, profiled), so the view byte-swaps into a reused scratch (no per-call alloc, ~4–5×; `BITPIX = 8` is zero-copy). The swap itself is write-allocate/RFO-bound (~8–9 GiB/s); SIMD does **not** help it (AVX2/blocked variants measured slower) — so no SIMD-swap TODO |
| `table/` | `BINTABLE` parsing (`Tform`/`Column`); per-column `ColumnReader` handles decode on demand to `ColumnData` (`BitColumn` for `X`, `num-complex` for `C`/`M`), `TSCAL`/`TZERO` physical planes including complex and exact unsigned P/Q heap VLAs | read done (write in `writer/`) |
| `ascii/` | `TABLE` (ASCII) read: `TBCOLn`/Fortran `TFORMn` → `AsciiColumn`/nullable `AsciiColumnData` (preserves `TNULLn`) | read done (write in `writer/`) |
| `groups/` | random-groups (§6) read: exact typed per-group parameter/array `RandomGroupView` plus `PSCALn`/`PZEROn` physical values | read done (no write — deprecated) |
| `checksum.rs` | `DATASUM`/`CHECKSUM` ones'-complement accumulate + Appendix-J encode; verification distinguishes absent, unknown, valid, and invalid assertions | done |
| `compress/` (feature `compression`) | tiled image+table (de)compress: `gzip`/`rice`/`plio`/`hcompress` codecs, `quantize` (float), `table` (§10.3); `decode.rs` reassembles + dequantizes tiles into the image, `encode.rs` the integer + float encoders, `mod.rs` the typed `Compression`/`CompressionOptions` API, shared `ImageCodec` dispatch, and `P`→`Q` descriptor threshold (`needs_wide`); `geometry` handles N-d tiling; `convert` shares byte/`i64`/`f64` conversions; `map_tiles` fans independent codec work across rayon | all 5 integer-image codecs read+write plus `NOCOMPRESS`; float quantization with all 3 dither methods, `ZBLANK`, and `NULL_PIXEL_MASK` restoration; signed-64-bit HCOMPRESS with `SMOOTH=1` decode + noise-scaled lossy write; fixed-width and P/Q VLA table compression read+write; tile-parallel ((de)compress, image + table) |
| `wcs/` | typed WCS orchestration and keyword parsing, with the 27 FITS 4.0 kernels in `projection/` (including cube TSC/CSC/QSC and parameterized HPX); declared celestial/spectral frame metadata, PC/CD/CROTA + `PVi_m`, every Table-26 spectral/detector algorithm, generic `LOG`, and BINTABLE-backed multidimensional `TAB`, with complete `pixel_to_world`/`world_to_pixel` | standard algorithms done (`XPH` convention and inter-frame transforms out of scope) |
| `time/` | typed time (§9): `Datetime` (strict unsigned-four/signed-five ISO-8601 and same-frame JD/MJD), private numeric J/B epoch interpretation, preserved recognized/realized/local scale declarations, typed `TREFPOS`/`TRPOSn`, all PHASE keyword forms, fallible prefixed FITS time units, `FitsTime` header view + PC/CD-coupled time WCS axes with per-axis unit/scale overrides | done |
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
- **Lazy by default.** HDU boundaries are computable from headers alone using the
  role-specific primary, extension, or random-groups formula, rounded to a block —
  never read data to find the next HDU. The reader is generic over a `Source`
  (`reader/source.rs`):
  a `StreamSource` over any `Read + Seek` copies each data unit into the reused
  scratch, while an in-memory `SliceSource` (`from_bytes`) or `MmapSource`
  (`open_mmap`, `mmap` feature) hands back a zero-copy borrow — so the decode reads
  straight from the bytes, skipping the staging copy (≈2× on `read_image`).
- **Headers round-trip logically.** Model a header as an *ordered list* of records
  with a side index for lookup — not a hash map. Duplicate `COMMENT`/`HISTORY`
  and record order are significant and must survive parse→write→parse. Physical
  value layout and `CONTINUE` splits are normalized on write, not retained.
- **Parallelize the compute-bound layer; reuse buffers on the memory-bound one.** The
  benches settled where threads pay: the tiled codecs are compute-bound (100s of MiB/s,
  ~100× below the memory wall) and tiles are independent, so `compress::map_tiles`
  fans independent codec work across rayon under the `parallel` feature. Ordered
  heap concatenation stays serial; decode destinations use safe disjoint row chunks
  when a serial scatter is material. The raw byte-swap +
  `BSCALE/BZERO` / `TSCAL/TZERO` paths are *memory-bound*, but SIMD is **not** their
  lever: a transforming store-loop runs at write-allocate/RFO speed (~8 GiB/s on a
  Zen3 core, ~½ the single-thread `memcpy` wall), and profiling found explicit-AVX2
  and cache-blocked swaps measure *slower*, not faster; threading them buys little
  either, so they stay serial. Their real cost is the per-call output allocation — a
  fresh-`Vec`-per-call decode is page-fault-bound (~65% of cycles) — so the lever is
  buffer reuse: the read loop uses `read_image_view`→`BorrowedImage` (swap into a
  caller-owned reused scratch; `BITPIX = 8` borrows zero-copy), the writer reuses its
  buffer via `encode_into`, each paying the allocation once (~4–5×). Always keep a
  scalar fallback behind the feature gate.
- **Reuse buffers across calls.** `FitsReader` owns a data `scratch`; `FitsWriter`
  owns separate reusable data and header buffers because checksum generation needs
  both representations alive together. Steady-state staging allocates nothing;
  decode/encode expose `*_into` forms that append into a caller buffer. The codecs
  build their data unit straight into the writer's data scratch.
- **Feature-flag the layers that carry a dependency — but they're on by default.**
  Tiled compression pulls in `flate2` (`compression` feature) and tile parallelism
  pulls in `rayon` (`parallel`, which implies `compression`); both are in the
  default feature set for batteries-included performance. The further opt-in
  `mmap` feature (memmap2) adds zero-copy memory-mapped read sources
  (`FitsReader::open_mmap`/`MmapSource`). WCS (§8) and time (§9) are
  dependency-free pure math and always compiled. `--no-default-features` yields the
  pure-Rust core, whose only unconditional dependencies are `bitvec` (packed `X`
  bit-array columns) and `num-complex` (`C`/`M` complex columns) — both back core
  `BINTABLE` kinds that can't be feature-gated cleanly; gate any *other* new
  dependency behind a feature the same way unless it's likewise core.
- **"Once FITS, always FITS."** The format never breaks backward compatibility.
  Keep reading legacy structures (random groups, `SIMPLE = F`) forever; just
  don't *write* deprecated forms.

## Correctness expectations

FITS is full of fiddly invariants that silent bugs hide in — test them explicitly
(this is also mandated by the global Rust rules):

- Block padding: assert every written unit is a 2880 multiple, padded with the
  correct fill byte (space for headers/ASCII-table data, NUL for other data).
- Round-trip: parse→write→parse must reproduce the ordered logical header model
  and data bit-for-bit (including float NaN/Inf, `BLANK`, unsigned offsets).
- Cross-check decoders against known-good files (CFITSIO/astropy outputs) and
  against hand-computed values for small fixtures — never `result < N` assertions.
- Boundary cases: `NAXIS = 0` (no data), zero-length axes, `TFORM` repeat count 0,
  empty heap, `PCOUNT = 0`, maximum 999 columns/axes.

## Conventions registry

Real files lean on a few near-ubiquitous conventions — `CONTINUE` long strings
(now normative, §4.2.1.2), `CHECKSUM`/`DATASUM` integrity keywords (§4.4.2.7 +
Appendix J), plus the registered `HIERARCH` long-keyword convention. These are
covered in `docs/refs/08-conventions.md`; the full registry (Green Bank,
inheritance, ESO, …) is at <https://fits.gsfc.nasa.gov/fits_registry.html>.
The standard mechanisms are implemented: `CONTINUE` long strings (read + write)
and `CHECKSUM`/`DATASUM` (verify on read; solve + write via `with_checksums`).
`HIERARCH` input is preserved as opaque commentary but is neither interpreted nor
authored. Registered conventions remain outside the normative core.
