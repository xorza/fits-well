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

Recent fixes were meaningful: table/image scaling and null metadata is now checked
against its stored type, fixed-width bit padding is canonical, random-group array
`BLANK` handling is correct, ASCII character fields retain leading spaces, and the
WCS matrix/default/projection fixes are standards-backed. Those fixes improve
correctness, but the previous all-green status table overstated the remaining
coverage.

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
| §4 | Headers and integrity keywords | Mostly complete: normal/continued cards, fallible construction, exact unbounded integers, and checksums work; unknown checksum states are still reported as failures. |
| §5 | Data representation | Complete for core stored types: all `BITPIX` encodings, endian paths, unsigned conventions, and type/range-checked image scaling metadata serialize exactly. |
| §6 / §7.1 | Primary arrays, IMAGE, random groups | Mostly complete on disk; typed random-group raw values and safe image invariants remain incomplete. |
| §7.2 | ASCII tables | Complete for core table structures: formats, scaling, distinct nullable raw cells, field-count limits, and fallible width-checked writing are covered. |
| §7.3 | Binary tables | Complete for core table structures: all fixed kinds and P/Q reads and writes exist, including type-checked scaling/null metadata, character, physical numeric/complex, exact unsigned, jagged bit arrays, and the 999-field ceiling. |
| §8 | WCS | Complete for standard algorithms: image and table keyword translation, all 27 celestial projections, every Table-26 spectral/detector algorithm, generic `LOG`, and BINTABLE-resolved multidimensional `TAB` coordinates are covered. Convention-only `XPH` remains explicitly unsupported. |
| §9 | Time | Partial: references, scales, FITS units, strict year forms, leap-second-preserving UTC quasi-JD, and complete linear/`LOG` time-axis WCS evaluation work; secondary frame and PHASE metadata remain incomplete. |
| §10 | Tiled compression | Not assessed here; it is outside the dependency-free core. |

## Batch 1 — Critical: stop wrong values and valid-file failures

- [x] **Resolve every Table-22 binary-table WCS keyword through its normative primary/alternate spelling.** A shared resolver now supplies the distinct primary and shortened alternate axis names for pixel-list parsing, vector-cell parsing, and rank inference. It also handles `TPC`/`TP`, `TCD`/`TC`, `TPV`/`TV`, vector parameter spellings, `WCAXna`, and column-indexed `LONPna`/`LATPna` (`docs/refs/fits_standard40.md:3459-3480`). Exact keyword tests cover every supported primary and alternate axis family; equivalent image, pixel-list, and vector-cell transforms are cross-checked against wcslib/astropy TAN, CEA, and CROTA values, including non-default pole selection.

- [x] **Do not return `Ok(world)` from a transform whose nonlinear stage was not evaluated.** Unsupported-axis classification covers celestial projections, convention-only algorithms, and header-only four-character `TAB` forms (`docs/refs/fits_standard40.md:3779-3801`). Raw HDU/WCS parsing remains permissive, but `pixel_to_world` and `world_to_pixel` return `UnsupportedWcsTransform` whenever required external table data was not resolved. XPH and header-only `FREQ-TAB`/`TIME-TAB` tests verify that partial coordinates cannot be returned as complete results; `FitsReader::read_wcs` supplies the complete `TAB` path.

- [x] **Evaluate time coordinates through the complete WCS row and its effective axis frame.** `time_axis_mjd` consumes the already-parsed `Wcs` and full pixel vector, reusing its PC/CD precedence, alternate/table keyword translation, generic `LOG`, and resolved `TIME-TAB` lookup. Per-axis `CUNITia` and recognized `CTYPEia` scales override `TIMEUNIT`/`TIMESYS`; the returned `TimeCoordinate` carries both MJD and the effective scale. Exact fixtures cover coupled PC rows, `CD1_1=2`, logarithmic and tabular sampling, days over global seconds, and TAI over global UTC (`docs/refs/fits_standard40.md:4087-4103`, `:4499-4503`, `:4824-4832`).

- [x] **Serialize the unsigned 64-bit convention with the exact `2^63` decimal.** `Image::from_u64` and signed-`K` table columns with the unsigned convention now route their offsets through `FitsInteger`, emitting the normative `BZERO`/`TZEROn = 9223372036854775808` rather than a rounded `f64` decimal (`docs/refs/fits_standard40.md:1635-1643`, `docs/refs/fits_standard40.md:2604-2625`). Raw-card tests assert the exact 20-byte numeric field, and image/table tests round-trip `u64::{MIN, MAX}` through the typed unsigned view.

- [x] **Preserve binary-table `A` fields and null strings exactly.** `CharacterField` retains every stored byte while exposing the members before the first NUL and the initial-NUL null-string state (`docs/refs/fits_standard40.md:2804-2821`). Fixed `A` and VLA `PA`/`QA` decoding therefore keep trailing spaces, terminators, and undefined post-NUL bytes distinct. Binary writing accepts NUL terminators, preserves exact P/Q heap payloads, and rejects over-width fixed values before emitting the automatic primary. Hand-built and write/read fixtures cover `AB  `, `AB\0x`, initial NUL, all spaces, and both descriptor widths.

- [x] **Make HDU discovery role-aware before scanning the rest of a header.** The scanner requires first-card `SIMPLE` for HDU 0 and first-card `XTENSION` thereafter; any other post-HDU block starts special records without searching it for `END` (`docs/refs/fits_standard40.md:607-612`). The validated `HduRole` also selects primary Eq. 1, extension Eq. 2, or random-groups Eq. 4, so `GROUPS` cannot alter extension sizing and role-mandatory `PCOUNT`/`GCOUNT` cannot default silently. Tests cover special records with a later canonical `END`, empty and extension-only input, exact handcrafted next-HDU boundaries, both missing counts, and invalid random-group signatures.

- [x] **Implement the two FITS year forms and a leap-second-preserving UTC conversion.** `Datetime::parse` enforces unsigned-four/signed-five syntax across the full standard range, including the normative `-04713-11-24T12:00:00` JD origin (`docs/refs/fits_standard40.md:4005-4040`). Scale-aware JD/MJD conversion validates leap labels only in the final minute of an actual UTC insertion date and represents UTC with ERFA-compatible quasi-JD day fractions. UTC↔TAI/TT/UT1 preserves the inserted instant; fixtures cover every embedded insertion plus ERFA's 2016 `23:59:59`, `23:59:60`, and 2017 midnight values (`docs/refs/fits_standard40.md:4043-4045`).

- [x] **Preserve or reject out-of-range integer keyword values; never saturate them.** `FitsInteger` stores ordinary values without allocation and retains larger signed decimals exactly, matching FITS's unbounded integer syntax (`docs/refs/fits_standard40.md:883-897`). `Value::as_integer` and `Header::get_integer` return `IntegerOutOfRange` when an exact integer or integral real cannot fit `i64`. Tests cover both `i64` boundaries, one-past values, exact complex-integer components, real-to-integer bounds, and a `BITPIX=64` `BLANK` above `i64::MAX`.

## Batch 2 — High: finish standard data and coordinate coverage

- [x] **Implement the remaining standard WCS nonlinear coordinate algorithms.** All 27 Table-23 celestial projections, every Table-26 `F2*`/`W2*`/`V2*`/`A2*`/`GRI`/`GRA` transform, generic `LOG`, and BINTABLE-backed `TAB` interpolation/inversion are evaluated (`docs/refs/fits_standard40.md:3551-3598`, `:3779-3801`; `src/wcs/axis.rs`, `src/wcs/tabular/mod.rs`). Analytic and detector values are cross-checked with wcslib 8.5; tabular paths use exact one-/multidimensional fixtures. XPH remains a separately reported convention.

- [x] **Parse FITS time units instead of treating every unknown spelling as seconds.** `unit_seconds` is fallible, accepts the standard time bases with one SI prefix, and rejects non-time dimensional spellings. The discouraged `ta` and `Ba` units use the normative epoch-dependent equations at `MJDREF` in TDB/ET rather than constants. Relative-time and GTI fixtures cover `ms`, `ks`, days, invalid units, and the J2000/J1900 equation origins (`docs/refs/fits_standard40.md:1089-1136`, `:4489-4538`).

- [x] **Treat `TDIMn` as inapplicable to a zero-length VLA cell.** Reader and writer skip only the dimension-product comparison for empty descriptors, while malformed shapes and undersized nonempty cells remain errors. Hand-built mixed `P`/`Q`/`PX`/`QX` rows prove empty cells ignore their undefined heap offsets; writer round-trips cover both descriptor widths with the same empty/nonempty shape (`docs/refs/fits_standard40.md:2670-2681`).

- [x] **Complete typed physical access for variable-length arrays.** `vla_complex` applies `TSCALn` to both components and `TZEROn` only to the real component, while `vla_unsigned` returns one exact typed `UnsignedView` per row for the standard integer-offset conventions (`docs/refs/fits_standard40.md:2575-2608`, `:3108-3112`). Fixed and P/Q paths share the same conversion helpers. Hand-built `PC`/`QM` fixtures verify both descriptor widths, scaling, and empty cells; `PK`/`QK` fixtures recover `2^53 + 1` and `u64::MAX` exactly while demonstrating the rounded `f64` physical result.

- [x] **Add `PX`/`QX` writing.** `WriteColumn::vla_bits` accepts one MSB-first, exact-length `BitVec` per row, keeping bit counts distinct from the packed heap byte lengths required by `X` arrays (`docs/refs/fits_standard40.md:3013-3065`). `.wide()` selects `QX`; the same preflight and `TDIMn` rules as other VLAs run before the automatic primary is emitted. A write/read fixture verifies empty, one-bit, and nine-bit P/Q rows, exact descriptor counts and byte offsets, `TFORMn` maxima, and zeroed low padding bits.

- [x] **Keep ASCII nulls distinct from numeric zero in both raw and write models.** `AsciiColumnData` represents every character, integer, and float cell as `Option`, so `AsciiColumnReader::raw` retains `TNULLn` as `None` while blank numeric fields remain `Some(0)`. `AsciiWriteColumn` accepts the same representation, requires a valid marker for null cells, rejects nonfinite float values and marker collisions before output, and round-trips character nulls beside a genuine integer zero (`docs/refs/fits_standard40.md:2277-2284`, `:2372-2380`).

- [x] **Expose exact stored random-group samples.** `RandomGroups::group_by_idx` returns an allocation-free `RandomGroupView` whose separate parameter and array `ImageView`s borrow the decoded host-endian buffer before any parameter or image scaling. Tests verify group boundaries, `i64` extremes and values beyond `2^53`, exact `f32`/`f64` NaN payloads, signed zero, subnormals, and an explicit out-of-range error (`docs/refs/fits_standard40.md:1994-2002`).

- [ ] **Expose declared WCS/time frames without performing astrometry.** `WcsView` exposes only axes and unsupported flags and never parses `RADESYS`/`EQUINOX` (`src/wcs/mod.rs:842-878`), despite the frame defaults in FITS (`docs/refs/fits_standard40.md:3649-3658`). `FitsTime` likewise leaves an omitted `TREFPOS` unresolved instead of exposing the `TOPOCENTER` default (`src/time/mod.rs:515-528`, `docs/refs/fits_standard40.md:4256-4269`). Add typed declared-frame metadata, including spectral/time reference metadata where applicable. FK4/FK5/ICRS conversion and light-time correction remain out of scope.

## Batch 3 — High: make every core writer path conforming and fallible

- [x] **Validate header mutation at insertion instead of panicking or emitting invalid cards.** Public mutation uses the fallible `Header::try_set`, `try_comment`, `try_push_comment`, and `try_push_history` APIs. They preserve the old value/card on failure and reject malformed or reserved/control keywords, restricted-ASCII violations, nonfinite real/complex values, and any scalar/commentary card exceeding 80 bytes. The reader and parser share the same canonical blank `END` recognizer, so a valued `END` cannot terminate a header. Exact invalid-keyword, control-keyword, Unicode/control, NaN/Inf, and oversized-complex fixtures cover the public entry points (`docs/refs/fits_standard40.md:670-807`, `docs/refs/fits_standard40.md:925-1035`).

- [x] **Preserve all `CONTINUE` payload/commentary or return an error before writing.** Orphan quoted content and its slash comment are retained as a commentary record, while conforming chains concatenate every comment fragment in order. Long-string writing adds the standard empty final substring when that makes the comment fit; an unrepresentable final comment or any oversized scalar/commentary card returns `HeaderCardTooLong` before partial records are appended. The normative value/comment example, orphan round-trip, and exact-fit/one-byte-over boundaries verify the behavior (`docs/refs/fits_standard40.md:810-874`).

- [x] **Preflight ASCII field widths instead of substituting data.** One formatter supplies the exact text used by both validation and writing. A value wider than `TFORMn` returns `AsciiFieldTooWide` with its column, zero-based row, declared width, and minimum required width instead of emitting forbidden `*` data. Float precision is bounded against the field before formatting, so hostile `decimals` cannot trigger an oversized temporary allocation. Null-marker width remains a `TNULLn` range error. Exact-fit, one-byte-over, and `usize::MAX`-precision fixtures cover every source, and rejected first extensions leave the sink untouched (`docs/refs/fits_standard40.md:2372-2450`).

- [x] **Validate scaling, null sentinels, and packed-bit padding by stored type.** Binary fixed and P/Q columns reject `TSCAL`/`TZERO` on `A`/`L`/`X`, accept `TNULL` only for stored `B`/`I`/`J`/`K` values within their exact ranges, and reject nonfinite scale/zero metadata before automatic-primary output. ASCII `A` columns reject scaling and numeric columns reject nonfinite scales. Fixed and variable `X` writers clear unused low bits. Image and compressed-image writers reject nonfinite scaling, float `BLANK`, and integer sentinels outside the stored `BITPIX` range. Tests cover every fixed/VLA type, every integer boundary, both float image kinds, and caller-supplied nonzero bit padding (`docs/refs/fits_standard40.md:2256-2275`, `:2575-2643`, `:2797-2803`).

- [x] **Reject more than 999 table fields before allocation or output.** ASCII, binary, and compressed-table code share one `MAX_TABLE_FIELDS` structural limit. Both regular writers check it before allocating column layouts or emitting an automatic primary. Exact boundary tests serialize and parse 999-field ASCII and binary tables, while 1000 fields return `KeywordOutOfRange("TFIELDS")` with an untouched sink (`docs/refs/fits_standard40.md:2350-2355`, `:2782-2787`).

- [x] **Seal image geometry/type invariants at safe API boundaries.** `Image::new` validates the axis product, sample count, and scaling before construction, and its fields are no longer independently mutable. `RawImage::bitpix` derives the tag from private raw/decoded storage; `ImageMetadata` provides the immutable public view. Writer, compression, and caller-shaped ndarray boundaries return `DataSizeMismatch` for disagreement. Tests cover empty and zero-axis shapes, all six `BITPIX` variants, and exact off-by-one failures.

- [x] **Preflight a complete HDU and poison torn writers.** Every extension path completes encoding, padding, checksum/header rendering, and any late validation before emitting an automatic primary. `write_raw_hdu` validates and commits a complete logical HDU in one operation, including role, exact data length, and derived block fill. Writer state tracks `Empty`/`Active`/`Failed`; a partial sink error poisons the writer and subsequent writes return `WriterFailed`. Late compression and raw-HDU failures leave a fresh sink untouched.

## Batch 4 — Medium: finish semantic status and secondary time APIs

- [ ] **Represent checksum status as absent, unknown, valid, or invalid.** Blank-string `DATASUM` becomes `Some(false)` and blank `CHECKSUM` is verified as if it asserted a checksum (`src/reader/mod.rs:420-449`). FITS defines blank strings as unknown values (`docs/refs/fits_standard40.md:1698-1718`). Replace `Option<bool>` with an explicit status and test all four states independently for both keywords.

- [ ] **Support alternate/table PHASE metadata and reject undefined folding.** `Header::phase_axis` reads only unsuffixed image `CZPHS`/`CPERI` (`src/time/mod.rs:657-674`), though FITS defines alternate and binary-table forms (`docs/refs/fits_standard40.md:4717-4734`). `PhaseAxis::fold` returns `0.0` when `CPERI` is absent/zero (`src/time/mod.rs:483-490`), even though that means the period is nonconstant/undefined. Accept the same selectors as WCS and make folding return `Option`/`Result` when no constant period exists.

- [ ] **Add authoring support or qualify the HIERARCH “read + write” claim.** Existing HIERARCH cards can be parsed and re-rendered, but `Header::try_set` rejects long/spaced compound names and there is no public HIERARCH constructor (`src/header/mod.rs`). This is a convention rather than normative FITS 4.0 core, so preservation-only support is acceptable if documented accurately.

## Confirmed coverage

The audit found no missing core on-disk image `BITPIX` kind, ASCII format,
fixed-width binary kind, P/Q descriptor reader, or random-groups reader. The
following foundations are substantively correct for conforming inputs:

- 2880-byte/80-byte geometry, checked padding math, and high-level fill bytes;
- big-endian scalar decoding/encoding, including float bit preservation;
- ordered logical header storage and keyword indexing;
- checked `NAXIS`/extent arithmetic once the correct HDU role is known;
- normal image scaling/`BLANK` and stored-type-checked table scaling/null handling;
- lazy bounded source reads and raw padded-data access;
- one's-complement checksum arithmetic, Appendix-J encoding, and normal
  checksum generation/verification;
- 27 implemented WCS projection formula families already covered by external
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
  270 unit tests passed
  5 doc-tests passed, 1 ignored
env CARGO_TARGET_DIR=.tmp/target cargo clippy --all-targets --no-default-features -- -D warnings
  clean
```

This baseline is healthy and fast, but it does not cover the concrete standard
cases listed above; each checklist item includes the regression boundary needed
before it can be marked complete.
