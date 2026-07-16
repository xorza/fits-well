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
| §3 | File/HDU structure and blocking | Partial: block geometry is correct, but a valid trailing special record containing an `END`-shaped card can be parsed as another HDU. |
| §4 | Headers and integrity keywords | Partial: normal cards/checksums work, but public mutation can create invalid cards, `CONTINUE` commentary is lossy, large integers can change value, and unknown checksums are reported as failures. |
| §5 | Data representation | Partial: all `BITPIX` encodings and endian paths exist, but unsigned 64-bit output does not serialize the exact required offset. |
| §6 / §7.1 | Primary arrays, IMAGE, random groups | Mostly complete on disk; typed random-group raw values and safe image invariants remain incomplete. |
| §7.2 | ASCII tables | Partial: formats and scaling read correctly, but nulls collapse into ordinary zero in the raw model and over-width writes produce invalid numeric fields. |
| §7.3 | Binary tables | Partial: all fixed kinds and P/Q reads exist, but character/null semantics are lossy, empty VLA+`TDIM` is rejected, some VLA physical views and `PX`/`QX` writing are absent. |
| §8 | WCS | Incomplete: table-keyword translation has correctness bugs; unsupported transforms can return successful intermediate coordinates; four standard projections and all Table-26 nonlinear algorithms remain unevaluated. |
| §9 | Time | Partial: basic dates, references, and scales work, but signed years, leap-second conversion, prefixed units, and full time-axis WCS overrides are incorrect/incomplete. |
| §10 | Tiled compression | Not assessed here; it is outside the dependency-free core. |

## Batch 1 — Critical: stop wrong values and valid-file failures

- [ ] **Resolve every Table-22 binary-table WCS keyword through its normative primary/alternate spelling.** Pixel-list alternates are currently queried as `TCTYPnA`, `TCRVLnA`, `TCDLTnA`, and similar long-name-plus-suffix forms (`src/wcs/mod.rs:1124-1145`), while FITS requires the shortened `TCTYnA`, `TCRVnA`, `TCDEnA`, `TCRPnA`, and `TCUNnA` forms (`docs/refs/fits_standard40.md:3459-3465`). Vector-cell alternates and rank inference repeat the same mismatch (`src/wcs/mod.rs:1194-1209`, `src/wcs/mod.rs:1547-1576`). Pixel lists also ignore the standard `TPn_ka`/`TCn_ka` and `TVn_ma` aliases, and read `LONPa`/`LATPa` instead of column-indexed `LONPna`/`LATPna` (`src/wcs/mod.rs:1142-1163`, `docs/refs/fits_standard40.md:3467-3480`); vector cells omit the pole keywords. Add one Table-22 resolver shared by pixel-list, vector-cell, and inference paths. Validate primary and alternate image/table equivalents, every alias, and non-default poles against wcslib/astropy.

- [ ] **Do not return `Ok(world)` from a transform whose nonlinear stage was not evaluated.** `pixel_to_world` and `world_to_pixel` ignore `unsupported_axes` and return linear-stage values as successful world coordinates (`src/wcs/mod.rs:1236-1299`). Detection is also restricted to the ten spectral type names, although `TAB` and `LOG` apply to any four-character type, so valid `TIME-TAB` is not flagged at all (`src/wcs/mod.rs:1334-1359`, `docs/refs/fits_standard40.md:3779-3801`). Keep raw HDU/WCS parsing permissive, but make complete transform methods return an unsupported-transform error or a result that inseparably carries partial status; expose intermediate coordinates through an explicitly partial API. Verify HPX, QSC, `FREQ-LOG`, and `TIME-TAB`.

- [ ] **Evaluate time coordinates through the complete WCS row and its effective axis frame.** `time_axis_mjd` reads only scalar `CRPIX`, `CRVAL`, and `CDELT` (`src/time/mod.rs:638-655`), ignoring `PC`/`CD` coupling, direct `CDi_j` intervals, per-axis `CUNITia`, and a time scale named by `CTYPEia` (`docs/refs/fits_standard40.md:4087-4103`, `docs/refs/fits_standard40.md:4499-4503`, `docs/refs/fits_standard40.md:4824-4832`). Route through the WCS linear engine, require the full pixel vector when coupled, resolve axis-over-global unit/scale precedence, and return the effective scale with the MJD. Verify a `CD1_1=2` axis, a day axis over global seconds, and TAI over global UTC.

- [ ] **Serialize the unsigned 64-bit convention with the exact `2^63` decimal.** `Image::from_u64` stores the correct mathematical offset in `f64`, but `Scaling::add_to_header` and `format_real` render it as `9223372036854776000.0` (`src/data/mod.rs:448`, `src/data/mod.rs:721`, `src/header/card/mod.rs:504-525`) rather than the required `9223372036854775808` (`docs/refs/fits_standard40.md:1635-1643`). The same exact-value requirement applies to binary-table `TZEROn` (`src/writer/mod.rs:202-204`, `src/writer/mod.rs:612-613`, `docs/refs/fits_standard40.md:2604-2625`). Add an exact integral/decimal header representation instead of routing the convention through display-formatted `f64`. Verify raw card bytes and exact-decimal interpretation, not merely a round-trip through another `f64` reader.

- [ ] **Preserve binary-table `A` fields and null strings exactly.** Fixed and VLA character decoders call `trim_text`, removing member trailing spaces and collapsing an initial NUL and an all-space field to the same empty `String` (`src/table/mod.rs:1003`, `src/table/mod.rs:1077`, `src/table/mod.rs:1097`). The writer forbids NUL and silently truncates/pads the input (`src/writer/mod.rs:843-851`, `src/writer/mod.rs:916-923`). FITS defines all `repeat` characters as members unless NUL terminates the string and gives an initial NUL distinct null-string semantics (`docs/refs/fits_standard40.md:2804-2821`). Introduce a binary-character value that retains bytes/length and explicit null state. Verify `AB  `, `AB\0x`, initial NUL, all spaces, and their `PA`/`QA` forms.

- [ ] **Make HDU discovery role-aware before scanning the rest of a header.** After the primary HDU, any block sequence containing a canonical `END` is parsed and any header without `XTENSION` is classified as another primary (`src/reader/mod.rs:146-179`, `src/reader/mod.rs:490-524`, `src/hdu/mod.rs:35-59`). FITS special-record contents are otherwise unspecified and only forbid `XTENSION` in their first eight bytes (`docs/refs/fits_standard40.md:607-612`), so a valid trailing special block can create a false HDU or fail file opening. Require first-card `SIMPLE` for HDU 0 and first-card `XTENSION` thereafter; otherwise stop at special records. Pass the validated role into extent calculation so `GROUPS`, `PCOUNT`, and `GCOUNT` use the correct primary/extension/random-groups formula. Verify special records containing `END`, empty/extension-only inputs, required extension counts, and random-groups boundaries.

- [ ] **Implement the two FITS year forms and a leap-second-preserving UTC conversion.** `Datetime::parse` strips a sign and then requires four digits (`src/time/mod.rs:61-73`), rejecting every normative signed five-digit year such as `-04713` and accepting non-conforming signed four-digit years (`docs/refs/fits_standard40.md:4005-4040`). `Datetime::to_jd` divides the time of day by 86400 (`src/time/mod.rs:102-107`), making `23:59:60` identical to the following midnight even though FITS permits the leap-second label (`docs/refs/fits_standard40.md:4043-4045`). Enforce unsigned-four/signed-five syntax and make UTC conversion scale/date aware, using a two-part or quasi-JD representation if necessary. Cross-check the standard year examples and the 2016 leap second against ERFA/astropy.

- [ ] **Preserve or reject out-of-range integer keyword values; never saturate them.** Integer parsing falls back to `f64` after `i64` overflow (`src/header/card/mod.rs:335-344`), and `Value::as_integer` casts integral reals back to `i64`, which saturates (`src/header/value.rs:35-43`). FITS permits larger integer lexemes (`docs/refs/fits_standard40.md:883-897`); changing one silently can alter semantic metadata. Preserve the exact lexeme/value or return a range error from integer access. Verify both `i64` boundaries, one-past values, exact `2^63` real/offset use, and a `BITPIX=64` `BLANK` that must not alias `i64::MAX`.

## Batch 2 — High: finish standard data and coordinate coverage

- [ ] **Implement the remaining standard WCS nonlinear algorithms.** The current 23-projection table ends at `PCO` (`src/wcs/mod.rs:126-150`); standard `TSC`, `CSC`, `QSC`, and `HPX` remain unevaluated (`docs/refs/fits_standard40.md:3551-3598`), as do the Table-26 spectral `F2*`/`W2*`/`V2*`/`A2*`, `LOG`, `GRI`, `GRA`, and `TAB` algorithms (`:3779-3801`). XPH is a convention rather than a Table-23 FITS 4.0 projection and should be described separately. Implement in independently verified families; until each lands, the complete transform must report it unsupported as required by Batch 1.

- [ ] **Parse FITS time units instead of treating every unknown spelling as seconds.** `unit_seconds` defaults any supplied unknown unit to `1.0` (`src/time/mod.rs:531-545`), so valid `TIMEUNIT='ms'` makes 1000 mean 1000 seconds rather than one second. FITS permits SI prefixes through §4.3 (`docs/refs/fits_standard40.md:1089-1097`, `:1123-1136`) and imports those rules for time (`:4489-4504`). Return `Result`, support time-dimensional prefixes, reject non-time units, and account for the standard epoch-dependent `ta`/`Ba` definitions instead of fixed approximations. Verify relative times, GTIs, and axes in `ms`, `ks`, days, and invalid units.

- [ ] **Treat `TDIMn` as inapplicable to a zero-length VLA cell.** Reader and writer compare `product(TDIM)` with every descriptor count, including zero (`src/table/mod.rs:801-825`, `src/table/mod.rs:951-958`, `src/writer/mod.rs:724-750`, `src/writer/mod.rs:776-805`). FITS explicitly exempts a zero-size descriptor (`docs/refs/fits_standard40.md:2670-2681`). Skip only the per-cell product check for empty descriptors while retaining syntax and nonempty-cell validation. Verify mixed empty/nonempty `P`, `Q`, `PX`, and `QX` rows.

- [ ] **Complete typed physical access for variable-length arrays.** `vla_physical` rejects complex heap types and `unsigned` rejects every VLA (`src/table/mod.rs:625-635`, `src/table/mod.rs:731-769`, `src/table/mod.rs:1027-1059`), although `TSCAL`/`TZERO` apply to heap elements, including complex and unsigned-convention integers (`docs/refs/fits_standard40.md:2575-2608`, `:3108-3112`). Add complex VLA scaling with zero offset on the imaginary component and exact VLA unsigned views. Verify scaled `PC`/`QM` and `PK`/`QK` values above `2^53`.

- [ ] **Add `PX`/`QX` writing.** Reading jagged bit VLAs exists, but `ColumnType` has no bit kind for VLA payloads and `WriteColumnData::Bits` is fixed-width only (`src/writer/mod.rs:82-102`, `src/writer/mod.rs:166-181`). FITS permits `X` as a P/Q heap element type (`docs/refs/fits_standard40.md:3013-3030`). Add a jagged-bit payload with a bit count per row; descriptor counts are bits, while heap extents are bytes. Verify empty, one-bit, and non-byte-aligned P/Q rows and zeroed padding bits.

- [ ] **Keep ASCII nulls distinct from numeric zero in both raw and write models.** `AsciiColumnReader::raw` maps a `TNULLn` integer entry to the same zero as a real zero (`src/ascii/mod.rs:206-247`), and the writer can explicitly select null cells only through nonfinite `F64` values (`src/writer/mod.rs:980-1004`). FITS permits a per-field null marker for every ASCII field kind (`docs/refs/fits_standard40.md:2277-2284`, `:2372-2380`). Add an ASCII-specific nullable representation or null bitmap. Verify a marker such as `NULL` beside a genuine integer zero and a character null.

- [ ] **Expose exact stored random-group samples.** `RandomGroups` keeps `ImageData` private and exposes arrays/parameters only through `f64` physical values (`src/groups/mod.rs:16-23`, `src/groups/mod.rs:96-133`). A valid `BITPIX=64` value above `2^53` therefore has no exact typed route even though random groups use the full §5 representation (`docs/refs/fits_standard40.md:1994-2002`). Add a raw typed per-group view that separates parameters from array values. Verify `i64` extremes and exact float bit patterns.

- [ ] **Expose declared WCS/time frames without performing astrometry.** `WcsView` exposes only axes and unsupported flags and never parses `RADESYS`/`EQUINOX` (`src/wcs/mod.rs:842-878`), despite the frame defaults in FITS (`docs/refs/fits_standard40.md:3649-3658`). `FitsTime` likewise leaves an omitted `TREFPOS` unresolved instead of exposing the `TOPOCENTER` default (`src/time/mod.rs:515-528`, `docs/refs/fits_standard40.md:4256-4269`). Add typed declared-frame metadata, including spectral/time reference metadata where applicable. FK4/FK5/ICRS conversion and light-time correction remain out of scope.

## Batch 3 — High: make every core writer path conforming and fallible

- [ ] **Validate header mutation at insertion instead of panicking or emitting invalid cards.** `Header::set` asserts on public keyword input and permits valued `END`/`CONTINUE` (`src/header/mod.rs:263-284`); public strings/comments can carry bytes outside restricted ASCII; nonfinite reals panic during rendering (`src/header/card/mod.rs:504-509`). Make mutation fallible (or provide a fallible builder), reserve control keywords, validate restricted ASCII and finite numbers, and share the canonical `END` recognizer with the scanner/parser. Verify valued control cards, Unicode/control text, malformed keywords, and NaN/Inf leave the sink untouched.

- [ ] **Preserve all `CONTINUE` payload/commentary or return an error before writing.** Orphan quoted `CONTINUE` records lose their substring when demoted (`src/header/mod.rs:74-84`), folding replaces rather than concatenates comment fragments (`src/header/mod.rs:339-356`), and long final comments/card bodies are clipped by `write_at` (`src/header/card/mod.rs:404-456`, `src/header/card/mod.rs:533-538`). Retain orphan content, implement the normative comment continuation behavior, and reject unrepresentable bodies. Verify the standard multi-record example and exact-fit/one-byte-over cases.

- [ ] **Preflight ASCII/binary character widths instead of substituting data.** `format_ascii_field` emits `*` for every over-width value (`src/writer/mod.rs:980-1004`), but `*` is forbidden by the ASCII numeric grammar unless it is the explicit null marker (`docs/refs/fits_standard40.md:2396-2450`); binary `A` cells are silently truncated (`src/writer/mod.rs:843-851`). Format and validate all fields before `ensure_primary`, returning a contextual error. Verify over-width integer, float, text, and null-marker inputs write nothing.

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
