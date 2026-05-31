# Roadmap to Feature-Complete (FITS 4.0)

Goal: **functional** completeness for reading and writing the full FITS 4.0
standard, correctness first. Spec references point at [`docs/refs/`](refs/) and,
normatively, `docs/refs/fits_standard40.pdf`.

**Out of scope for this roadmap (deferred to a later performance pass):** SIMD
byte-swap/scaling, `rayon` parallel decode, memory-mapped (`mmap`) zero-copy
sources, and benchmarks. The crate's `parallel`/`mmap` features stay empty until
then. Everything here is about *what* the library can do, not how fast.

## Definition of "feature-complete"
Round-trip (read **and** write) every standard structure: primary arrays, `IMAGE`
/ `TABLE` / `BINTABLE` extensions (incl. binary-table heap), random groups (read),
tiled-compressed images and tables, with typed access to the WCS and time
coordinate systems and the in-standard conventions (`CONTINUE`, `CHECKSUM`/
`DATASUM`) plus the ubiquitous registered `HIERARCH`.

## Current status (done)
- **Structural spine** — 2880 block layer; ordered header model with all value
  types, `CONTINUE` folding, `HIERARCH`, and a keyword builder; HDU classification
  + data-unit sizing; lazy seeking reader. *(§3–§5)*
- **Images** — read + write, big-endian decode/encode, `BSCALE`/`BZERO` physical
  plane, `BLANK`, unsigned-int trick. *(§5, §7.1)*
- **Binary tables** — read (every `TFORM` type, `TSCALn`/`TZEROn`, `P`/`Q` heap
  VLAs) and write (fixed-width). *(§7.3)*
- **ASCII tables** — read + write (`Aw`/`Iw`/`Fw.d`/`Ew.d`/`Dw.d`). *(§7.2)*
- **Multi-HDU files** — write primary + `IMAGE`/`TABLE`/`BINTABLE` extensions.
- **Random groups** — read (params + arrays, `PSCAL`/`PZERO`). *(§6)*
- **Conventions** — `CHECKSUM`/`DATASUM` verify + write; `HIERARCH` parse/render. *(§J)*

97 tests, validated against real sample files (incl. astropy-generated compressed
fixtures). Phases 1–5 are **complete**: full tiled image compression (all five
codecs, quantized float with both dither methods, `ZBLANK`, HCOMPRESS smoothing)
and tiled table compression, read and write. WCS and time (6–7) remain.

---

## Phase 1 — Complete the write path  ✅ DONE (binary-table VLA write still TODO)  *(size: M)*
The reader is far ahead of the writer; close the gap so anything readable is
writable.

- **1a. Image extensions + multi-HDU writing.** `write_image` is primary-only.
  Add `XTENSION='IMAGE'` headers (`PCOUNT=0`/`GCOUNT=1`) and a file-level writer
  that appends HDUs in sequence (primary first, then extensions), handling the
  `EXTEND` flag. New API: a `FitsWriter`-level "append HDU" path / `write_hdu`.
  *(§7.1; ref 04)*
- **1b. Binary-table writing.** A column builder (typed columns → `TFORMn`
  synthesis), row packing, heap assembly for `P`/`Q`, and `PCOUNT`/`THEAP`
  computation. Inverse of the `table.rs` reader. New API: `ColumnSpec`/table
  builder + `write_table`. *(§7.3; ref 06)*
- **Deliverable:** round-trip tests — build → write → read → identical — for
  multi-HDU image files and binary tables (incl. a VLA column).

## Phase 2 — ASCII tables (`TABLE`)  ✅ DONE  *(size: M)*
The one standard data structure with no support yet.

- Parse `TBCOLn`/`TFORMn` Fortran formats (`Aw`, `Iw`, `Fw.d`, `Ew.d`, `Dw.d`),
  fixed byte-range column extraction, `TNULLn` (string), space-padded data fill.
- Read into typed columns (reuse/extend `ColumnData`) and write (format values to
  field widths). `HduKind::AsciiTable` is already classified.
- *(§7.2; ref 05)*
- **Deliverable:** round-trip + a real/synthetic `TABLE` fixture; explicit
  blank-integer-field-=-0 vs `TNULLn` semantics tested.

## Phase 3 — Random-groups decode (read-only)  ✅ DONE  *(size: S)*
Already classified and sized; add typed access.

- Decode `GCOUNT` groups, each = `PCOUNT` parameters (`PTYPEn`, `PSCALn`/`PZEROn`)
  + the per-group array. Expose a `read_groups`-style API. **Read only** — never
  emit random groups (deprecated). *(§6; ref 04)*
- **Deliverable:** decode the bundled `DDTSUVDATA.fits` primary; hand-checked
  group/param counts and a sample parameter value.

## Phase 4 — In-standard conventions  ✅ DONE  *(size: S–M)*
- **4a. `CHECKSUM`/`DATASUM`** — the 32-bit ones'-complement accumulator,
  `verify()` on read and `update()` on write (DATASUM before CHECKSUM, fixed-format
  16-char encoding). *(§J; ref 08)*
- **4b. `HIERARCH`** — parse the compound space-separated keyword into a normalized
  key instead of the current commentary fallback; round-trip it. *(registry; ref 08)*
- **Deliverable:** checksum verify against a CFITSIO/astropy-written file; HIERARCH
  parse + render round-trip.

## Phase 5 — Tiled compression  ✅ DONE  *(size: L)*
Highest-value remaining *read* gap — most modern archive images are compressed.
These are functional codecs (decode/encode), not the deferred speed work.

- **5a. Tiled image (de)compression** — the `ZIMAGE` BINTABLE container, tile
  reassembly into the `ZNAXISn` image, and the codecs. *(§10.1)*
  ✅ **All five codecs, both directions** — `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`,
  `HCOMPRESS_1` (incl. `SMOOTH=1` decode), validated pixel-exact against astropy.
- **5b. Floating-point quantization** — `ZSCALE`/`ZZERO`, `ZQUANTIZ`, subtractive
  dithering (`ZDITHER0`), NaN preservation. *(§10.2)*
  ✅ **Decode + encode done** — `NO_DITHER`, `SUBTRACTIVE_DITHER_1`, and
  `SUBTRACTIVE_DITHER_2` (cfitsio noise-3 estimator + `fits_init_randoms` table),
  `ZBLANK`/NaN nulls, raw-gzip fallback for constant tiles. Validated against
  astropy (reconstruction within the 0.5·`ZSCALE` bound; smoothing bit-for-bit).
- **5c. Tiled table compression.** *(§10.3)*
  ✅ **Read + write** — `read_compressed_table`/`write_compressed_table` for
  fixed-width columns (`GZIP_1`/`GZIP_2`/`RICE_1` per column, `ZCTYPn`), the row-tile
  transpose + byte-shuffle, `1QB` heap layout. Validated against **cfitsio
  ground truth** (`fpack`/`funpack`): our decode reproduces an `fpack`-compressed
  table byte-exact, and cfitsio's `funpack` reads our compressed output back to the
  original. (VLA columns are rejected.)
- **5d. Compression writing** (encode tiles).
  ✅ **All five image codecs + quantized float + tables** write and round-trip.
- *(ref 07.) Gated behind the `compression` feature; scalar reference impls — the
  SIMD/parallel speed work is deferred to a later optimization pass.*
- **Deliverable:** ✅ round-trip every codec through astropy, both read and write.
- **Remaining (minor):** HCOMPRESS lossy *write* (`SMOOTH`/`SCALE>0` encode) and
  VLA columns inside compressed tables.

## Phase 6 — Typed World Coordinate System  🟢 v2 done  *(size: L)*
Behind the `wcs` feature: `Wcs::from_header` + `pixel_to_world`/`world_to_pixel`,
plus reference frames in `wcs::frame::Frame`.

- ✅ Linear layer: `PC`+`CDELT`, `CD`, or legacy `CDELT`+`CROTA`, alternate axes
  (`…a`), `LONPOLE`/`LATPOLE`; general matrix inversion for the reverse direction.
- ✅ Projections: zenithal `TAN`/`SIN`/`ARC`/`STG`/`ZEA` and cylindrical
  `CAR`/`CEA`/`MER`/`SFL`, via the general fiducial-point pole computation (CG 2002
  §2.4); non-celestial axes pass through linearly.
- ✅ Reference frames (`RADESYS`/`EQUINOX`): ICRS, FK5 at any equinox (IAU-1976
  precession), Galactic.
- ✅ **Validated** pixel-exact against `astropy.wcs` (every projection + CROTA to
  1e-8°) and frames against `astropy` `SkyCoord`.
- **Remaining (v2+):** `PVi_m` projection parameters (`SIN` slant, `CEA` λ), all-sky
  `AIT`/`MOL`, the ~25 mas ICRS↔FK5 frame bias, FK4 (B1950) E-terms, spectral axes.

## Phase 7 — Typed time coordinates  🟢 done  *(size: M)*
Behind the `time` feature: `Datetime`, `Epoch`, `TimeScale`, `FitsTime`.

- ✅ ISO-8601 datetimes ↔ Julian Date / MJD (proleptic-Gregorian calendar math,
  forward + inverse); `J`/`B` epochs → JD.
- ✅ `TimeScale` conversions among `UTC`/`UT1`/`TAI`/`TT`/`GPS`/`TCG`/`TDB`/`TCB`:
  exact fixed offsets, UTC↔TAI via an embedded IERS leap-second table, TCG/TCB
  rates, and the standard TDB periodic approximation.
- ✅ `FitsTime::from_header`: `TIMESYS`, `MJDREF`/`MJDREFI`+`MJDREFF`/`JDREF*`/
  `DATEREF`, `TIMEUNIT`, `TREFPOS`; resolves `DATE-OBS`/`MJD-OBS` and relative
  (`TSTART`/`TSTOP`) times to absolute MJD.
- ✅ **Validated** against `astropy.time` (ERFA) to 1e-9 day: ISO/JD, epochs, all
  six scale conversions, UT1 via explicit `ΔUT1`; leap seconds vs the IERS table.
- ✅ Time as a WCS axis (`CTYPEi = 'TIME'`) → absolute MJD (`FitsTime::time_axis_mjd`).
- **Remaining (v2+):** `TREFDIR`/topocentric light-travel corrections, bundled
  `ΔUT1` table (currently caller-supplied).

---

## Suggested order & rationale
1. **Phase 1** (write parity) and **Phase 2** (ASCII tables) — finish core
   read+write of every uncompressed structure. Small/medium, unblock real use.
2. **Phase 3** (random groups) and **Phase 4** (conventions) — small, broad value,
   round out standard coverage.
3. **Phase 5** (compression) — large but the biggest real-world read gap.
4. **Phase 6/7** (WCS, time) — large semantic layers; many users only need pixel/
   table I/O, so these come last.

Each phase ends green on the standard gate
(`cargo test && cargo fmt --all && cargo check && cargo clippy --all-targets -- -D warnings`)
with hand-computed + real-file tests, per the project's correctness rules.

## After feature-complete (separate track)
The deferred **performance pass**: criterion benches, SIMD bulk byte-swap +
scaling, `rayon` parallel tiling, and the `mmap` zero-copy read source — turning
the "blazing fast" goal into a measured result.
