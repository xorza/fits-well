# FITS 4.0 Core Fix Plan

Execution plan derived from [`conformance.md`](conformance.md). Batches are ordered
by risk and dependency. Each is intentionally small enough for one focused
implementation, review, and verification handoff.

For every batch:

- add exact regression tests for all changed non-GUI code;
- update `CHANGELOG.md` when public API or observable behavior changes;
- run the repository's complete verification matrix before marking the batch done.

## Batch 1 — Critical: correct binary-table WCS keyword resolution

- [x] **Introduce one Table-22 keyword resolver.** Encode the distinct primary and shortened alternate spellings for pixel-list and vector-cell WCS instead of appending an alternate suffix to the primary long name (`src/wcs/mod.rs:1124-1145`, `src/wcs/mod.rs:1194-1209`, `src/wcs/mod.rs:1547-1576`; standard `docs/refs/fits_standard40.md:3459-3470`). Verify every axis keyword under primary and alternate `A` descriptions.
- [x] **Route matrix, parameter, pole, and rank lookup through the resolver.** Accept `TPC`/`TP`, `TCD`/`TC`, `TPV`/`TV`, column-indexed `LONPna`/`LATPna`, and their vector-cell equivalents (`src/wcs/mod.rs:1142-1163`, `src/wcs/mod.rs:1222-1233`; standard `docs/refs/fits_standard40.md:3467-3480`). Cross-check equivalent image, pixel-list, and vector-cell WCS fixtures against wcslib/astropy.

## Batch 2 — Critical: make partial WCS transforms impossible to mistake for complete results

- [x] **Classify nonlinear suffixes independently of the coordinate prefix.** Detect generic `LOG` and `TAB` axes such as `TIME-TAB`, not only the ten hard-coded spectral type names (`src/wcs/mod.rs:1334-1359`; standard `docs/refs/fits_standard40.md:3779-3801`). Verify celestial, spectral, time, and generic linear axes independently.
- [x] **Reject incomplete transforms.** `pixel_to_world`/`world_to_pixel` now return `UnsupportedWcsTransform` whenever any nonlinear stage is unavailable. HPX, QSC, `FREQ-LOG`, and `TIME-TAB` cannot return unqualified complete coordinates; the linear stage remains an implementation detail until a concrete production caller needs it.

## Batch 3 — Critical: preserve exact FITS integer header values

- [x] **Add an exact integral/decimal header representation.** `FitsInteger` keeps ordinary `i64` values allocation-free and preserves larger normalized decimals exactly. Bounded integer access now returns `IntegerOutOfRange` rather than saturating. Both `i64` boundaries, one-past values, exact complex components, and oversized `BLANK` are covered.
- [x] **Write exact unsigned 64-bit offsets.** Unsigned-64 image and signed-`K` table writers emit `BZERO`/`TZEROn = 9223372036854775808` through `FitsInteger`. Raw numeric fields are asserted byte-for-byte, and `u64::{MIN, MAX}` round-trip through typed unsigned reads.

## Batch 4 — Critical: preserve binary character fields exactly

- [x] **Replace trimmed `String` decoding with a binary-character value.** `CharacterField` retains the complete fixed or heap byte sequence, exposes members before the first NUL, and distinguishes initial-NUL null strings from empty and all-space fields. Fixed `A` plus `PA`/`QA` tests cover `AB  `, `AB\0x`, initial NUL, and all spaces.
- [x] **Make binary-character writing lossless and width-checked.** Binary `A` writing now accepts the standard NUL terminator, preserves P/Q payload bytes, and rejects over-width fixed values during preflight. Exact fixed/P/Q round trips cover both descriptor widths and leave a fresh sink empty on rejection.

## Batch 5 — Critical: make HDU scanning role-aware

- [x] **Determine structure role from the first card before scanning further.** HDU 0 now requires a first-card `SIMPLE`, later HDUs require first-card `XTENSION`, and any other post-HDU block starts special records without scanning for a later `END`. Empty, extension-only, ordinary trailing, and `END`-shaped special-record fixtures cover the decision.
- [x] **Calculate extents from the validated role.** `HduRole` selects the primary Eq. 1, extension Eq. 2, or random-groups Eq. 4 path. Extension/random-groups `PCOUNT` and `GCOUNT` are mandatory, `GROUPS` cannot alter extension geometry, and exact handcrafted next-HDU boundaries plus invalid random-group signatures are covered.

## Batch 6 — Critical: resolve time units and axes through the WCS model

- [x] **Replace `unit_seconds` fallback with a fallible FITS unit parser.** Standard time bases and single SI prefixes are parsed case-sensitively, non-time units fail, and `ta`/`Ba` use their epoch-dependent FITS equations at `MJDREF`. Relative-time and GTI tests cover `ms`/`ks`, day units, invalid units, and both deprecated year definitions.
- [x] **Evaluate time axes through the complete linear WCS row.** `FitsTime::time_axis_mjd` now accepts a parsed `Wcs` and the full pixel vector, so image, alternate, and translated table descriptions share PC/CD resolution. `CUNITia` and recognized `CTYPEia` scales override the global frame, and `TimeCoordinate` returns both MJD and the effective scale. Tests cover PC coupling, direct CD, day-over-second, TAI-over-UTC, and unsupported `TIME-TAB`.

## Batch 7 — Critical: correct FITS datetime boundaries

- [x] **Enforce unsigned-four and signed-five year syntax.** `Datetime::parse` accepts only unsigned four-digit or signed five-digit years, including the full `-99999`…`+99999` range. Tests reject both wrong-width forms, verify the normative `-04713-11-24T12:00:00` JD origin, and round-trip both signed limits (`docs/refs/fits_standard40.md:4005-4040`).
- [x] **Preserve leap seconds during UTC↔JD conversion.** Datetime conversion now requires a declared scale, validates `second=60` against the actual UTC insertion date/final minute, and uses an ERFA-compatible UTC quasi-JD through UTC↔TAI/TT/UT1 conversion. All embedded insertion dates plus the 2016 `23:59:59`, `23:59:60`, and following midnight values are covered (`docs/refs/fits_standard40.md:4043-4045`).

## Batch 8 — High: complete variable-length-array semantics

- [x] **Skip per-cell `TDIMn` product validation for zero-length descriptors.** Reader and writer retain `TDIMn` syntax validation but apply its product only to nonempty descriptor cells. Hand-built `P`/`Q`/`PX`/`QX` fixtures and writer round-trips cover mixed empty/nonempty rows, ignored empty-cell heap offsets, exact shapes, and undersized nonempty cells (`docs/refs/fits_standard40.md:2670-2681`).
- [ ] **Add complex and exact-unsigned VLA physical views.** Apply `TSCAL` to both complex components and `TZERO` only to the real component, and expose exact unsigned P/Q integer values above `2^53` (`src/table/mod.rs:625-635`, `src/table/mod.rs:731-769`, `src/table/mod.rs:1027-1059`; standard `docs/refs/fits_standard40.md:2575-2608`, `docs/refs/fits_standard40.md:3108-3112`).
- [ ] **Add `PX`/`QX` writing.** Carry a bit count per jagged row, encode descriptor counts in bits, size heap spans in bytes, and clear unused trailing bits (`src/writer/mod.rs:82-102`, `src/writer/mod.rs:166-181`; standard `docs/refs/fits_standard40.md:3013-3030`). Verify empty, one-bit, and non-byte-aligned rows under P and Q.

## Batch 9 — High: preflight table output against stored types

- [ ] **Validate scaling/null keywords and X padding by actual fixed/VLA type.** Reject `TSCAL`/`TZERO` on `A`/`L`/`X`, restrict and range-check `TNULL`, reject nonfinite scales, and zero unused X bits (`src/writer/mod.rs:200-210`, `src/writer/mod.rs:609-618`, `src/writer/mod.rs:852-859`; standard `docs/refs/fits_standard40.md:2575-2643`, `docs/refs/fits_standard40.md:2797-2803`). Apply equivalent range/type validation to image `BLANK` (`src/data/mod.rs:721-732`).
- [ ] **Reject `TFIELDS > 999` before allocation or output.** Share the reader's limit across ASCII and binary writers (`src/writer/mod.rs:315-327`, `src/writer/mod.rs:388-405`, `src/writer/mod.rs:595-618`, `src/writer/mod.rs:943-965`; standard `docs/refs/fits_standard40.md:2350-2355`, `docs/refs/fits_standard40.md:2782-2787`). Verify 999 succeeds and 1000 leaves the sink empty.
- [ ] **Reject over-width ASCII fields during preflight.** Never substitute `*` for numeric data unless it is the declared null marker; validate integer, float, text, and null-marker widths before `ensure_primary` (`src/writer/mod.rs:900-924`, `src/writer/mod.rs:980-1004`; standard `docs/refs/fits_standard40.md:2372-2450`).

## Batch 10 — High: make header construction fallible and lossless

- [ ] **Add fallible header mutation.** Reserve `END`/`CONTINUE`, validate keywords, restricted-ASCII strings/comments, and finite numeric values at insertion rather than asserting or panicking during render (`src/header/mod.rs:263-284`, `src/header/card/mod.rs:504-509`). Verify every invalid public input returns an error before bytes are written.
- [ ] **Preserve complete `CONTINUE` payloads and comments.** Retain orphan quoted content, concatenate continuation comment fragments, and reject unrepresentable final comments instead of clipping (`src/header/mod.rs:74-84`, `src/header/mod.rs:339-356`, `src/header/card/mod.rs:404-456`, `src/header/card/mod.rs:533-538`). Verify the normative multi-card example and exact-fit/+1-byte boundaries.

## Batch 11 — High: seal image and writer state invariants

- [ ] **Make image geometry and raw type tags invariant-bearing.** Hide/derive `RawImage.bitpix`, construct owned images fallibly, and return `DataSizeMismatch` instead of asserting when shape and sample count disagree (`src/data/mod.rs:327-396`, `src/data/mod.rs:564-605`, `src/writer/mod.rs:288-301`). Verify empty shapes, zero axes, every `BITPIX`, and off-by-one lengths.
- [ ] **Write one preflighted HDU transaction and poison torn writers.** Replace split raw header/data writes with `write_raw_hdu`, prepare the entire logical HDU before automatic-primary output, and track `Empty`/`Active`/`Failed` state (`src/writer/mod.rs:234-285`, `src/writer/mod.rs:315-421`, `src/writer/mod.rs:478-504`). Verify validation failures leave a fresh sink empty and injected partial I/O failures reject subsequent writes.

## Batch 12 — High: add standard cube and HEALPix projections

- [ ] **Implement `TSC`, `CSC`, and `QSC`.** Add forward/inverse transforms with exact face selection, boundary handling, and projection-domain errors (`src/wcs/mod.rs:126-150`; standard `docs/refs/fits_standard40.md:3551-3593`). Cross-check face centers, edges, corners, and round-trips against wcslib.
- [ ] **Implement standard `HPX`.** Support the normative HEALPix projection parameters and polar/equatorial facet transitions (`src/wcs/mod.rs:126-150`; standard `docs/refs/fits_standard40.md:3594-3598`). Keep convention-only XPH separate and verify HPX against wcslib/astropy.

## Batch 13 — High: add analytic nonlinear spectral coordinates

- [ ] **Implement the `F2*`, `W2*`, `V2*`, and `A2*` transforms.** Centralize frequency/wavelength/air-wavelength/apparent-velocity conversions, enforce required rest frequency/wavelength metadata, and support both directions (`src/wcs/mod.rs:1334-1359`; standard `docs/refs/fits_standard40.md:3720-3801`). Cross-check every Table-26 pair against wcslib.
- [ ] **Implement generic `LOG`.** Apply the standard logarithmic sampling transform to any valid four-character coordinate type and preserve unit metadata (`src/wcs/mod.rs:1334-1359`; standard `docs/refs/fits_standard40.md:3779-3801`). Verify reference points, domains, and inverse round-trips.

## Batch 14 — High: add detector and tabular WCS algorithms

- [ ] **Implement `GRI` and `GRA`.** Parse their required detector-coordinate parameters, return explicit errors for incomplete metadata, and cross-check forward/inverse values against wcslib (`src/wcs/mod.rs:1334-1359`; standard `docs/refs/fits_standard40.md:3796-3801`).
- [ ] **Implement `TAB` with table-data context.** Introduce a resolver that can obtain coordinate arrays and indexing vectors from the referenced BINTABLE instead of trying to evaluate `TAB` from a header alone (`src/wcs/mod.rs:863-1096`, `src/wcs/mod.rs:1334-1359`; standard `docs/refs/fits_standard40.md:3796-3801`). Verify monotonic and non-monotonic arrays, multidimensional indices, boundaries, and malformed references.

## Batch 15 — Medium: complete exact typed raw surfaces

- [x] **Represent ASCII nulls explicitly.** `AsciiColumnData` carries `Option` cells for text, integer, and float columns, preserving `TNULLn` as `None` without collapsing genuine zero. The writer uses the same model and rejects missing markers, nonfinite stored floats, and values that collide with their marker before output. Round-trip coverage includes a `NULL` character field beside an integer null and genuine zero (`docs/refs/fits_standard40.md:2277-2284`, `docs/refs/fits_standard40.md:2372-2380`).
- [x] **Expose exact stored random-group values.** `RandomGroups::group_by_idx` returns a borrowed `RandomGroupView` with separate typed parameter and array slices, preserving `BITPIX=64` values and floating-point payload bits without allocation or `f64` conversion. Coverage verifies group boundaries, integer extremes, NaN payloads, signed zero, subnormals, and out-of-range indices (`docs/refs/fits_standard40.md:1994-2002`).

## Batch 16 — Medium: expose complete coordinate-frame metadata

- [ ] **Add typed declared WCS/time frame metadata.** Parse and expose `RADESYS`/`EQUINOX`, spectral frame/rest metadata, and the resolved `TREFPOS=TOPOCENTER` default without performing inter-frame astrometry or light-time corrections (`src/wcs/mod.rs:842-878`, `src/time/mod.rs:515-528`; standard `docs/refs/fits_standard40.md:3649-3658`, `docs/refs/fits_standard40.md:4256-4269`).
- [ ] **Support alternate/table PHASE metadata and undefined periods.** Resolve all Table-22 `CZPHS`/`CPERI` forms and make folding fail explicitly when the period is absent or zero instead of returning phase zero (`src/time/mod.rs:483-490`, `src/time/mod.rs:657-674`; standard `docs/refs/fits_standard40.md:4717-4734`).

## Batch 17 — Medium: correct secondary status and support claims

- [ ] **Represent checksum state explicitly.** Distinguish absent, blank/unknown, valid, and invalid `DATASUM`/`CHECKSUM` values instead of mapping blank strings to failure (`src/reader/mod.rs:420-449`; standard `docs/refs/fits_standard40.md:1698-1718`).
- [ ] **Add public HIERARCH authoring.** Provide a fallible compound-key constructor/update path so “read + write” means more than preserving already-parsed HIERARCH cards (`src/header/mod.rs:263-284`). Verify long/spaced compound names and invalid convention syntax.
- [ ] **Correct byte-round-trip documentation.** Change `src/lib.rs:19-23` and matching project documentation to say the ordered logical header model round-trips; original physical card bytes are normalized and not retained.

## Execution order

Execute batches strictly in numeric order. Batches 1–7 remove current silent-wrong
and valid-file failure paths. Batches 8–11 close core writer/data invariants before
new algorithms expand the surface. Batches 12–14 complete the remaining standard
WCS algorithms. Batches 15–17 finish typed semantics and support accuracy.
