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
- [x] **Reject incomplete transforms.** `pixel_to_world`/`world_to_pixel` now return `UnsupportedWcsTransform` whenever any nonlinear stage is unavailable. XPH, `FREQ-TAB`, and `TIME-TAB` cannot return unqualified complete coordinates; the linear stage remains an implementation detail until a concrete production caller needs it.

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
- [x] **Evaluate time axes through the complete WCS row.** `FitsTime::time_axis_mjd` now accepts a parsed `Wcs` and the full pixel vector, so image, alternate, and translated table descriptions share PC/CD resolution and generic `LOG`. `CUNITia` and recognized `CTYPEia` scales override the global frame, and `TimeCoordinate` returns both MJD and the effective scale. Tests cover PC coupling, direct CD, logarithmic sampling, day-over-second, TAI-over-UTC, and unsupported `TIME-TAB`.

## Batch 7 — Critical: correct FITS datetime boundaries

- [x] **Enforce unsigned-four and signed-five year syntax.** `Datetime::parse` accepts only unsigned four-digit or signed five-digit years, including the full `-99999`…`+99999` range. Tests reject both wrong-width forms, verify the normative `-04713-11-24T12:00:00` JD origin, and round-trip both signed limits (`docs/refs/fits_standard40.md:4005-4040`).
- [x] **Preserve leap seconds during UTC↔JD conversion.** Datetime conversion now requires a declared scale, validates `second=60` against the actual UTC insertion date/final minute, and uses an ERFA-compatible UTC quasi-JD through UTC↔TAI/TT/UT1 conversion. All embedded insertion dates plus the 2016 `23:59:59`, `23:59:60`, and following midnight values are covered (`docs/refs/fits_standard40.md:4043-4045`).

## Batch 8 — High: complete variable-length-array semantics

- [x] **Skip per-cell `TDIMn` product validation for zero-length descriptors.** Reader and writer retain `TDIMn` syntax validation but apply its product only to nonempty descriptor cells. Hand-built `P`/`Q`/`PX`/`QX` fixtures and writer round-trips cover mixed empty/nonempty rows, ignored empty-cell heap offsets, exact shapes, and undersized nonempty cells (`docs/refs/fits_standard40.md:2670-2681`).
- [x] **Add complex and exact-unsigned VLA physical views.** `vla_complex` applies `TSCAL` to both components and `TZERO` only to the real component; `vla_unsigned` preserves exact P/Q unsigned values above `2^53`, including `u64::MAX` (`docs/refs/fits_standard40.md:2575-2608`, `docs/refs/fits_standard40.md:3108-3112`).
- [x] **Add `PX`/`QX` writing.** `WriteColumn::vla_bits` carries exact per-row bit counts, sizes heap spans in bytes, and clears unused trailing bits. Empty, one-bit, and non-byte-aligned rows are covered under both P and Q (`docs/refs/fits_standard40.md:3013-3030`).

## Batch 9 — High: preflight table output against stored types

- [x] **Validate scaling/null keywords and X padding by actual fixed/VLA type.** Binary fixed and P/Q columns reject `TSCAL`/`TZERO` on `A`/`L`/`X`, restrict and range-check `TNULL`, reject nonfinite scales, and zero unused fixed-`X` bits. ASCII `A` scaling is rejected as well. Image writers range/type-check `BLANK` and reject nonfinite `BSCALE`/`BZERO` before output. Tests cover every fixed/VLA kind, every integer sentinel boundary, both float image kinds, and non-byte-aligned rows (`docs/refs/fits_standard40.md:2256-2275`, `docs/refs/fits_standard40.md:2575-2643`, `docs/refs/fits_standard40.md:2797-2803`).
- [x] **Reject `TFIELDS > 999` before allocation or output.** ASCII, binary, and compressed-table paths share one structural ceiling. Both regular writers validate it before allocating per-column state or emitting an automatic primary; exact boundary tests prove 999 succeeds and 1000 leaves a fresh sink empty (`docs/refs/fits_standard40.md:2350-2355`, `docs/refs/fits_standard40.md:2782-2787`).
- [x] **Reject over-width ASCII fields during preflight.** Text and exactly formatted integer/float cells are width-checked before `ensure_primary`; one shared formatter validates and writes each value. Over-width values return `AsciiFieldTooWide` with the column, zero-based row, field width, and minimum required width. Null markers retain their `TNULLn` range error. Exact-fit, one-byte-over, and hostile-precision fixtures cover all sources and prove rejection leaves the sink empty (`docs/refs/fits_standard40.md:2372-2450`).

## Batch 10 — High: make header construction fallible and lossless

- [x] **Add fallible header mutation.** `Header::try_set`, `try_comment`, `try_push_comment`, and `try_push_history` validate standard and reserved keywords, restricted-ASCII text, finite real/complex values, and physical card length before changing the header. The scanner and parser share one canonical blank `END` recognizer. Invalid-keyword, valued-control, Unicode/control, NaN/Inf, and oversized-complex fixtures all leave the header unchanged (`docs/refs/fits_standard40.md:670-807`, `docs/refs/fits_standard40.md:925-1035`).
- [x] **Preserve complete `CONTINUE` payloads and comments.** Orphan quoted records are retained as commentary, every continued comment fragment is concatenated, and the renderer uses an empty final substring when needed for a fitting comment or returns `HeaderCardTooLong` without appending partial records. The normative multi-record example plus exact-fit/+1-byte commentary and final-comment boundaries cover read and write behavior (`docs/refs/fits_standard40.md:810-874`).

## Batch 11 — High: seal image and writer state invariants

- [x] **Make image geometry and raw type tags invariant-bearing.** `Image::new` validates the axis product, sample count, and scaling before construction; its fields are no longer independently mutable. `RawImage::bitpix` derives the type from its private backing representation, while `ImageMetadata` exposes immutable geometry/type/scaling. Writers, compression, and the caller-shaped ndarray bridge return `DataSizeMismatch` rather than asserting. Empty/zero-axis shapes, all six `BITPIX` kinds, and off-by-one lengths are covered.
- [x] **Write one preflighted HDU transaction and poison torn writers.** `write_raw_hdu` replaces split raw header/data output, validates the header-implied data size, and derives the block fill. Every extension path completes data/header preparation before an automatic primary is committed. Writer state is `Empty`/`Active`/`Failed`; an injected mid-header failure permanently rejects later writes, while late compression and raw-HDU validation failures leave a fresh sink empty.

## Batch 12 — High: add standard cube and HEALPix projections

- [x] **Implement `TSC`, `CSC`, and `QSC`.** Shared cube-face selection handles the complete cross layout, negative-face wrapping, exact edges/corners, and projection-domain failures. Forward/inverse face centers, boundaries, interiors, and CSC's documented approximate closure match wcslib 8.5 (`src/wcs/cube.rs`; standard `docs/refs/fits_standard40.md:3551-3593`).
- [x] **Implement standard `HPX`.** `H = PVi_1` and `K = PVi_2` use the normative 4/3 defaults, equatorial/polar transition, facet-gap validation, and even-`K` southern half-facet offset. Default, transition, pole, non-default 3/4, and round-trip values match wcslib 8.5; convention-only XPH remains separate (`src/wcs/healpix.rs`; standard `docs/refs/fits_standard40.md:3594-3598`).

## Batch 13 — High: add analytic nonlinear spectral coordinates

- [x] **Implement the `F2*`, `W2*`, `V2*`, and `A2*` transforms.** Shared frequency, wavelength, air-wavelength, and relativistic-velocity conversions cover every Table-25 output type in both directions. The parser resolves `RESTFRQ`/`RESTWAV`, deprecated primary `RESTFREQ`, and table `RFRQn`/`RWAVn`, enforces the transformations that require rest metadata, and normalizes FITS prefixes and numeric unit multipliers to Table-25 defaults (`src/wcs/axis.rs`; standard `docs/refs/fits_standard40.md:3720-3801`). Every Table-26 pair and all derived output types match wcslib 8.5 goldens.
- [x] **Implement generic `LOG`.** Any valid four-character coordinate type now applies the standard exponential/logarithmic pair while retaining its unit metadata; spectral `LOG` axes first normalize to their Table-25 default unit (`src/wcs/axis.rs`; standard `docs/refs/fits_standard40.md:3779-3801`). Reference points, non-positive inverse domains, time-axis integration, prefixed spectral units, and inverse round-trips are covered.

## Batch 14 — High: add detector and tabular WCS algorithms

- [x] **Implement `GRI` and `GRA`.** The spectral axis parser applies the seven detector parameters with their standard defaults, requires explicit non-zero grating density and interference order, rejects degenerate geometry, and evaluates vacuum/air grisms in both directions. KPNO MARS values match wcslib 8.5 at the reference point and offsets on both sides (`src/wcs/axis.rs`; standard `docs/refs/fits_standard40.md:3796-3801`).
- [x] **Implement `TAB` with table-data context.** `FitsReader::read_wcs` resolves the exact `EXTNAME`/`EXTVER`/`EXTLEVEL` BINTABLE, coordinate column, `TDIM`, and optional monotonic index-vector columns. The coupled transform performs N-dimensional multilinear interpolation and inverse voxel location while header-only `Header::wcs` remains explicitly incomplete. Exact fixtures cover increasing/decreasing indices, non-monotonic coordinate arrays, half-bin boundaries, multidimensional coupling, spectral-unit normalization, typed time integration, malformed metadata, and exact extension selection (`src/wcs/tabular/mod.rs`, `src/reader/mod.rs`; standard `docs/refs/fits_standard40.md:3796-3801`).

## Batch 15 — Medium: complete exact typed raw surfaces

- [x] **Represent ASCII nulls explicitly.** `AsciiColumnData` carries `Option` cells for text, integer, and float columns, preserving `TNULLn` as `None` without collapsing genuine zero. The writer uses the same model and rejects missing markers, nonfinite stored floats, and values that collide with their marker before output. Round-trip coverage includes a `NULL` character field beside an integer null and genuine zero (`docs/refs/fits_standard40.md:2277-2284`, `docs/refs/fits_standard40.md:2372-2380`).
- [x] **Expose exact stored random-group values.** `RandomGroups::group_by_idx` returns a borrowed `RandomGroupView` with separate typed parameter and array slices, preserving `BITPIX=64` values and floating-point payload bits without allocation or `f64` conversion. Coverage verifies group boundaries, integer extremes, NaN payloads, signed zero, subnormals, and out-of-range indices (`docs/refs/fits_standard40.md:1994-2002`).

## Batch 16 — Medium: expose complete coordinate-frame metadata

- [x] **Add typed declared WCS/time frame metadata.** `WcsView` exposes resolved celestial `RADESYS`/`EQUINOX`, while each spectral axis carries typed `SPECSYS`/defaulted `SSYSOBS` and validated `RESTFRQ`/`RESTWAV`; image, alternate, pixel-list, and vector-cell forms remain distinct per axis. `FitsTime` exposes typed `TREFPOS`, including its `TOPOCENTER` default and `TRPOSn` override through `Header::time_for_column`. These types report the declared frame without performing astrometry or light-time corrections (`src/wcs/mod.rs`, `src/time/mod.rs`; standard `docs/refs/fits_standard40.md:3649-3658`, `docs/refs/fits_standard40.md:4256-4269`).
- [x] **Support alternate/table PHASE metadata and undefined periods.** `Header` resolves primary and alternate image, pixel-list, and vector-cell `CZPHS`/`CPERI` families. A missing/zero constant period is represented as `None`, and `PhaseAxis::fold` returns an explicit error rather than a false phase zero (`src/time/mod.rs`; standard `docs/refs/fits_standard40.md:4717-4734`).

## Batch 17 — Medium: correct secondary status and support claims

- [x] **Represent checksum state explicitly.** `ChecksumStatus` distinguishes absent, blank/unknown, valid, and invalid `DATASUM`/`CHECKSUM` assertions; null or malformed values are invalid, while one-or-more-blank assertions avoid unnecessary data reads (`src/reader/mod.rs`; standard `docs/refs/fits_standard40.md:1698-1718`).
- [x] **Add public HIERARCH authoring.** `Header::try_set_hierarch` inserts or updates a compound keyword through the same indexed logical model as parsed cards, preserves an existing comment, and validates restricted ASCII, delimiters, boundaries, values, and card length before mutation. Long spaced names render and parse back exactly; invalid convention syntax leaves the header unchanged (`src/header/mod.rs`).
- [x] **Correct byte-round-trip documentation.** Crate and project documentation now promise ordered logical-header round-trips while stating that physical value layout and `CONTINUE` splits are normalized rather than retained.

## Execution order

Execute batches strictly in numeric order. Batches 1–7 remove current silent-wrong
and valid-file failure paths. Batches 8–11 close core writer/data invariants before
new algorithms expand the surface. Batches 12–14 complete the remaining standard
WCS algorithms. Batches 15–17 finish typed semantics and support accuracy.
