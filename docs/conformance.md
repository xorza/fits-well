# FITS 4.0 Core Conformance Review

Audit of the dependency-free core (`cargo test --no-default-features`) against
the bundled normative FITS 4.0 text in
[`refs/fits_standard40.md`](refs/fits_standard40.md). The reviewed core is the
always-compiled surface: file/HDU structure, headers, checksums, images, ASCII and
binary tables, random groups, WCS, time coordinates, reader, and writer (§§3–9).
Tiled compression (§10), `mmap`, and `ndarray` are outside this review.

## Conclusion

The core is substantial and its main architecture is sound, but it is **not yet
correct and complete enough to claim full FITS 4.0 conformance**. The remaining
items below are tied to valid FITS structures or deterministic public-API inputs;
they are not speculative performance concerns or demands for strict rejection of
every malformed real-world file.

Recent fixes were meaningful: fixed-width complex-column scaling now applies
`TZERO` only to the real component, random-group array `BLANK` handling is correct,
ASCII character fields retain leading spaces, the WCS matrix/default/projection
fixes are standards-backed, and the no-default-feature test suite passes. Those
fixes improve correctness, but the previous all-green status table overstated the
remaining coverage.

Severity used below:

- **Critical** — a conforming file can yield plausible wrong values, lose stored
  information, or fail to open.
- **High** — the writer can emit non-conforming output, a standard structure is
  missing, or safe public input can panic/corrupt state.
- **Medium** — typed semantics or status reporting are incomplete while raw bytes
  remain accessible.

## Status by normative section

| Section | Area | Current status |
| --- | --- | --- |
| §3 | File/HDU structure and blocking | Complete for core discovery and sizing: first-card roles, special records, the block grid, and primary/extension/random-groups boundaries are validated independently. |
| §4 | Headers and integrity keywords | Partial: normal cards/checksums and exact unbounded integers work, but public mutation can create invalid cards, `CONTINUE` commentary is lossy, and unknown checksums are reported as failures. |
| §5 | Data representation | Mostly complete: all `BITPIX` encodings, endian paths, and unsigned conventions serialize exactly; image `BLANK` range validation remains incomplete. |
| §6 / §7.1 | Primary arrays, IMAGE, random groups | Mostly complete on disk; typed random-group raw values and safe image invariants remain incomplete. |
| §7.2 | ASCII tables | Partial: formats and scaling read correctly, but nulls collapse into ordinary zero in the raw model and over-width writes produce invalid numeric fields. |
| §7.3 | Binary tables | Mostly complete: all fixed kinds and P/Q reads and writes exist, including character, physical numeric/complex, exact unsigned, and jagged bit arrays; writer metadata validation remains incomplete. |
| §8 | WCS | Partial: image and table keyword translation and all implemented transforms are covered; unsupported transforms are explicitly rejected. Four standard projections and all Table-26 nonlinear algorithms remain unevaluated. |
| §9 | Time | Partial: references, scales, FITS units, strict year forms, leap-second-preserving UTC quasi-JD, and complete linear time-axis WCS evaluation work; secondary frame and PHASE metadata remain incomplete. |
| §10 | Tiled compression | Not assessed here; it is outside the dependency-free core. |

## Batch 1 — Critical: stop wrong values and valid-file failures

- [x] **Resolve every Table-22 binary-table WCS keyword through its normative primary/alternate spelling.** A shared resolver now supplies the distinct primary and shortened alternate axis names for pixel-list parsing, vector-cell parsing, and rank inference. It also handles `TPC`/`TP`, `TCD`/`TC`, `TPV`/`TV`, vector parameter spellings, `WCAXna`, and column-indexed `LONPna`/`LATPna` (`docs/refs/fits_standard40.md:3459-3480`). Exact keyword tests cover every supported primary and alternate axis family; equivalent image, pixel-list, and vector-cell transforms are cross-checked against wcslib/astropy TAN, CEA, and CROTA values, including non-default pole selection.

- [x] **Do not return `Ok(world)` from a transform whose nonlinear stage was not evaluated.** Unsupported-axis classification covers celestial projections, spectral algorithms, and generic four-character `LOG`/`TAB` forms, including `TIME-TAB` (`docs/refs/fits_standard40.md:3779-3801`). Raw HDU/WCS parsing remains permissive, but `pixel_to_world` and `world_to_pixel` now return `UnsupportedWcsTransform` whenever any required nonlinear stage is unavailable. HPX, QSC, `FREQ-LOG`, and `TIME-TAB` tests verify that partial coordinates cannot be returned as complete results.

- [x] **Evaluate time coordinates through the complete WCS row and its effective axis frame.** `time_axis_mjd` consumes the already-parsed `Wcs` and full pixel vector, reusing its PC/CD precedence and alternate/table keyword translation. Per-axis `CUNITia` and recognized `CTYPEia` scales override `TIMEUNIT`/`TIMESYS`; the returned `TimeCoordinate` carries both MJD and the effective scale. Exact fixtures cover coupled PC rows, `CD1_1=2`, days over global seconds, TAI over global UTC, and rejection of `TIME-TAB` as incomplete (`docs/refs/fits_standard40.md:4087-4103`, `:4499-4503`, `:4824-4832`).

- [x] **Serialize the unsigned 64-bit convention with the exact `2^63` decimal.** `Image::from_u64` and signed-`K` table columns with the unsigned convention now route their offsets through `FitsInteger`, emitting the normative `BZERO`/`TZEROn = 9223372036854775808` rather than a rounded `f64` decimal (`docs/refs/fits_standard40.md:1635-1643`, `docs/refs/fits_standard40.md:2604-2625`). Raw-card tests assert the exact 20-byte numeric field, and image/table tests round-trip `u64::{MIN, MAX}` through the typed unsigned view.

- [x] **Preserve binary-table `A` fields and null strings exactly.** `CharacterField` retains every stored byte while exposing the members before the first NUL and the initial-NUL null-string state (`docs/refs/fits_standard40.md:2804-2821`). Fixed `A` and VLA `PA`/`QA` decoding therefore keep trailing spaces, terminators, and undefined post-NUL bytes distinct. Binary writing accepts NUL terminators, preserves exact P/Q heap payloads, and rejects over-width fixed values before emitting the automatic primary. Hand-built and write/read fixtures cover `AB  `, `AB\0x`, initial NUL, all spaces, and both descriptor widths.

- [x] **Make HDU discovery role-aware before scanning the rest of a header.** The scanner requires first-card `SIMPLE` for HDU 0 and first-card `XTENSION` thereafter; any other post-HDU block starts special records without searching it for `END` (`docs/refs/fits_standard40.md:607-612`). The validated `HduRole` also selects primary Eq. 1, extension Eq. 2, or random-groups Eq. 4, so `GROUPS` cannot alter extension sizing and role-mandatory `PCOUNT`/`GCOUNT` cannot default silently. Tests cover special records with a later canonical `END`, empty and extension-only input, exact handcrafted next-HDU boundaries, both missing counts, and invalid random-group signatures.

- [x] **Implement the two FITS year forms and a leap-second-preserving UTC conversion.** `Datetime::parse` enforces unsigned-four/signed-five syntax across the full standard range, including the normative `-04713-11-24T12:00:00` JD origin (`docs/refs/fits_standard40.md:4005-4040`). Scale-aware JD/MJD conversion validates leap labels only in the final minute of an actual UTC insertion date and represents UTC with ERFA-compatible quasi-JD day fractions. UTC↔TAI/TT/UT1 preserves the inserted instant; fixtures cover every embedded insertion plus ERFA's 2016 `23:59:59`, `23:59:60`, and 2017 midnight values (`docs/refs/fits_standard40.md:4043-4045`).

- [x] **Preserve or reject out-of-range integer keyword values; never saturate them.** `FitsInteger` stores ordinary values without allocation and retains larger signed decimals exactly, matching FITS's unbounded integer syntax (`docs/refs/fits_standard40.md:883-897`). `Value::as_integer` and `Header::get_integer` return `IntegerOutOfRange` when an exact integer or integral real cannot fit `i64`. Tests cover both `i64` boundaries, one-past values, exact complex-integer components, real-to-integer bounds, and a `BITPIX=64` `BLANK` above `i64::MAX`.

## Batch 2 — High: finish standard data and coordinate coverage

- [ ] **Implement the remaining standard WCS nonlinear algorithms.** The current 23-projection table ends at `PCO` (`src/wcs/mod.rs:126-150`); standard `TSC`, `CSC`, `QSC`, and `HPX` remain unevaluated (`docs/refs/fits_standard40.md:3551-3598`), as do the Table-26 spectral `F2*`/`W2*`/`V2*`/`A2*`, `LOG`, `GRI`, `GRA`, and `TAB` algorithms (`:3779-3801`). XPH is a convention rather than a Table-23 FITS 4.0 projection and should be described separately. Implement in independently verified families; until each lands, the complete transform must report it unsupported as required by Batch 1.

- [x] **Parse FITS time units instead of treating every unknown spelling as seconds.** `unit_seconds` is fallible, accepts the standard time bases with one SI prefix, and rejects non-time dimensional spellings. The discouraged `ta` and `Ba` units use the normative epoch-dependent equations at `MJDREF` in TDB/ET rather than constants. Relative-time and GTI fixtures cover `ms`, `ks`, days, invalid units, and the J2000/J1900 equation origins (`docs/refs/fits_standard40.md:1089-1136`, `:4489-4538`).

- [x] **Treat `TDIMn` as inapplicable to a zero-length VLA cell.** Reader and writer skip only the dimension-product comparison for empty descriptors, while malformed shapes and undersized nonempty cells remain errors. Hand-built mixed `P`/`Q`/`PX`/`QX` rows prove empty cells ignore their undefined heap offsets; writer round-trips cover both descriptor widths with the same empty/nonempty shape (`docs/refs/fits_standard40.md:2670-2681`).

- [x] **Complete typed physical access for variable-length arrays.** `vla_complex` applies `TSCALn` to both components and `TZEROn` only to the real component, while `vla_unsigned` returns one exact typed `UnsignedView` per row for the standard integer-offset conventions (`docs/refs/fits_standard40.md:2575-2608`, `:3108-3112`). Fixed and P/Q paths share the same conversion helpers. Hand-built `PC`/`QM` fixtures verify both descriptor widths, scaling, and empty cells; `PK`/`QK` fixtures recover `2^53 + 1` and `u64::MAX` exactly while demonstrating the rounded `f64` physical result.

- [x] **Add `PX`/`QX` writing.** `WriteColumn::vla_bits` accepts one MSB-first, exact-length `BitVec` per row, keeping bit counts distinct from the packed heap byte lengths required by `X` arrays (`docs/refs/fits_standard40.md:3013-3065`). `.wide()` selects `QX`; the same preflight and `TDIMn` rules as other VLAs run before the automatic primary is emitted. A write/read fixture verifies empty, one-bit, and nine-bit P/Q rows, exact descriptor counts and byte offsets, `TFORMn` maxima, and zeroed low padding bits.

- [ ] **Keep ASCII nulls distinct from numeric zero in both raw and write models.** `AsciiColumnReader::raw` maps a `TNULLn` integer entry to the same zero as a real zero (`src/ascii/mod.rs:206-247`), and the writer can explicitly select null cells only through nonfinite `F64` values (`src/writer/mod.rs:980-1004`). FITS permits a per-field null marker for every ASCII field kind (`docs/refs/fits_standard40.md:2277-2284`, `:2372-2380`). Add an ASCII-specific nullable representation or null bitmap. Verify a marker such as `NULL` beside a genuine integer zero and a character null.

- [ ] **Expose exact stored random-group samples.** `RandomGroups` keeps `ImageData` private and exposes arrays/parameters only through `f64` physical values (`src/groups/mod.rs:16-23`, `src/groups/mod.rs:96-133`). A valid `BITPIX=64` value above `2^53` therefore has no exact typed route even though random groups use the full §5 representation (`docs/refs/fits_standard40.md:1994-2002`). Add a raw typed per-group view that separates parameters from array values. Verify `i64` extremes and exact float bit patterns.

- [ ] **Expose declared WCS/time frames without performing astrometry.** `WcsView` exposes only axes and unsupported flags and never parses `RADESYS`/`EQUINOX` (`src/wcs/mod.rs:842-878`), despite the frame defaults in FITS (`docs/refs/fits_standard40.md:3649-3658`). `FitsTime` likewise leaves an omitted `TREFPOS` unresolved instead of exposing the `TOPOCENTER` default (`src/time/mod.rs:515-528`, `docs/refs/fits_standard40.md:4256-4269`). Add typed declared-frame metadata, including spectral/time reference metadata where applicable. FK4/FK5/ICRS conversion and light-time correction remain out of scope.

## Batch 3 — High: make every core writer path conforming and fallible

- [ ] **Validate header mutation at insertion instead of panicking or emitting invalid cards.** `Header::set` asserts on public keyword input and permits valued `END`/`CONTINUE` (`src/header/mod.rs:263-284`); public strings/comments can carry bytes outside restricted ASCII; nonfinite reals panic during rendering (`src/header/card/mod.rs:504-509`). Make mutation fallible (or provide a fallible builder), reserve control keywords, validate restricted ASCII and finite numbers, and share the canonical `END` recognizer with the scanner/parser. Verify valued control cards, Unicode/control text, malformed keywords, and NaN/Inf leave the sink untouched.

- [ ] **Preserve all `CONTINUE` payload/commentary or return an error before writing.** Orphan quoted `CONTINUE` records lose their substring when demoted (`src/header/mod.rs:74-84`), folding replaces rather than concatenates comment fragments (`src/header/mod.rs:339-356`), and long final comments/card bodies are clipped by `write_at` (`src/header/card/mod.rs:404-456`, `src/header/card/mod.rs:533-538`). Retain orphan content, implement the normative comment continuation behavior, and reject unrepresentable bodies. Verify the standard multi-record example and exact-fit/one-byte-over cases.

- [ ] **Preflight ASCII field widths instead of substituting data.** `format_ascii_field` emits `*` for every over-width value (`src/writer/mod.rs:1014-1040`), but `*` is forbidden by the ASCII numeric grammar unless it is the explicit null marker (`docs/refs/fits_standard40.md:2396-2450`). Format and validate all fields before `ensure_primary`, returning a contextual error. Verify over-width integer, float, text, and null-marker inputs write nothing. Binary `A` widths are already rejected during column preflight.

- [ ] **Validate scaling, null sentinels, and packed-bit padding by stored type.** `scaled`/`with_null` accept every binary kind and header generation emits the keywords unconditionally (`src/writer/mod.rs:200-210`, `src/writer/mod.rs:609-618`); fixed `X` bytes retain nonzero unused low bits (`src/writer/mod.rs:852-859`). FITS forbids `TSCAL`/`TZERO` on `A`/`L`/`X`, restricts `TNULL` to integer types, and requires unused bit padding to be zero (`docs/refs/fits_standard40.md:2575-2608`, `:2631-2643`, `:2797-2803`). Apply the same validation to image `BLANK`, which is currently dropped for floats and not range-checked for integer `BITPIX` (`src/data/mod.rs:721-732`). Verify every fixed/VLA type, sentinel boundary, nonfinite scale, and non-byte-aligned bit row.

- [ ] **Reject more than 999 table fields before allocation or output.** Both table writers only check that the count fits `i64` (`src/writer/mod.rs:315-327`, `src/writer/mod.rs:388-405`) before indexed-key generation (`src/writer/mod.rs:595-618`, `src/writer/mod.rs:943-965`). FITS limits both table types to 999 fields (`docs/refs/fits_standard40.md:2350-2355`, `:2782-2787`). Share the reader limit and verify 999 succeeds while 1000 returns an error with an untouched sink.

- [ ] **Seal image geometry/type invariants at safe API boundaries.** `RawImage.bitpix` is independently public and controls raw-byte interpretation (`src/data/mod.rs:327-360`, `src/data/mod.rs:376-396`); `Image.shape` and `samples` can disagree, and `write_image` responds with `assert!` (`src/data/mod.rs:564-605`, `src/writer/mod.rs:288-301`). Make raw tags derived/private, validate owned construction fallibly, and return `DataSizeMismatch` at every writer/bridge boundary. Verify empty shapes, zero axes, every `BITPIX`, and off-by-one sample counts.

- [ ] **Preflight a complete HDU and poison torn writers.** Extension writers may emit an automatic primary before later preparation fails (`src/writer/mod.rs:315-383`, `src/writer/mod.rs:388-421`, `src/writer/mod.rs:459-467`). The public raw header/data operations do not advance structured state, and a sink failure between header and data leaves a writer that can append again (`src/writer/mod.rs:234-285`, `src/writer/mod.rs:478-504`). Prepare header/data before changing the sink, replace split raw writes with a logical raw-HDU operation, and track `Empty`/`Active`/`Failed`. Verify late validation failures and injected partial I/O failures.

## Batch 4 — Medium: finish semantic status and secondary time APIs

- [ ] **Represent checksum status as absent, unknown, valid, or invalid.** Blank-string `DATASUM` becomes `Some(false)` and blank `CHECKSUM` is verified as if it asserted a checksum (`src/reader/mod.rs:420-449`). FITS defines blank strings as unknown values (`docs/refs/fits_standard40.md:1698-1718`). Replace `Option<bool>` with an explicit status and test all four states independently for both keywords.

- [ ] **Support alternate/table PHASE metadata and reject undefined folding.** `Header::phase_axis` reads only unsuffixed image `CZPHS`/`CPERI` (`src/time/mod.rs:657-674`), though FITS defines alternate and binary-table forms (`docs/refs/fits_standard40.md:4717-4734`). `PhaseAxis::fold` returns `0.0` when `CPERI` is absent/zero (`src/time/mod.rs:483-490`), even though that means the period is nonconstant/undefined. Accept the same selectors as WCS and make folding return `Option`/`Result` when no constant period exists.

- [ ] **Add authoring support or qualify the HIERARCH “read + write” claim.** Existing HIERARCH cards can be parsed and re-rendered, but `Header::set` rejects long/spaced compound names and there is no public HIERARCH constructor (`src/header/mod.rs:263-284`). This is a convention rather than normative FITS 4.0 core, so preservation-only support is acceptable if documented accurately.

## Confirmed coverage

The audit found no missing core on-disk image `BITPIX` kind, ASCII format,
fixed-width binary kind, P/Q descriptor reader, or random-groups reader. The
following foundations are substantively correct for conforming inputs:

- 2880-byte/80-byte geometry, checked padding math, and high-level fill bytes;
- big-endian scalar decoding/encoding, including float bit preservation;
- ordered logical header storage and keyword indexing;
- checked `NAXIS`/extent arithmetic once the correct HDU role is known;
- normal image scaling/`BLANK`, table scaling/null handling outside the cases above;
- lazy bounded source reads and raw padded-data access;
- one's-complement checksum arithmetic, Appendix-J encoding, and normal
  checksum generation/verification;
- 23 implemented WCS projection formula families already covered by external
  astropy/wcslib goldens;
- basic ISO date/JD/MJD, reference precedence, scale aliases, observation bounds,
  and GTI conversion outside the unit/axis/leap cases above.

## Deliberate boundaries and non-findings

- Inter-frame astrometry (FK4/FK5/ICRS/Galactic conversion), ephemeris-based
  light-time correction, and observatory-motion correction are not FITS parsing
  responsibilities. Parsing and exposing the declared frame remains in scope.
- Reader permissiveness for malformed mandatory-keyword order, nonblank fill, or
  other invalid files is not elevated here unless it changes valid-file behavior
  or lets a safe writer create invalid output.
- Deferring truncated data errors until a lazy HDU is read is consistent with the
  reader design.
- Random groups remain read-only because their use for new files is deprecated.
- Original physical header card bytes are not retained: parsing folds/normalizes
  strings and rendering canonicalizes them. `src/lib.rs:19-23` should say the
  ordered **model** round-trips, not claim byte-for-byte physical preservation.
- HIERARCH, XPH, and other registered conventions should be reported separately
  from normative FITS 4.0 coverage.

## Verification baseline

```text
env CARGO_TARGET_DIR=.tmp/target cargo test --no-default-features
  225 unit tests passed
  5 doc-tests passed, 1 ignored
env CARGO_TARGET_DIR=.tmp/target cargo clippy --all-targets --no-default-features -- -D warnings
  clean
```

This baseline is healthy and fast, but it does not cover the concrete standard
cases listed above; each checklist item includes the regression boundary needed
before it can be marked complete.
