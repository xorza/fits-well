# Changelog

## Unreleased

### Breaking changes

- `WriteColumn` is now an invariant-preserving opaque type instead of a collection
  of public, independently mutable fields. `WriteColumn::vla` now requires an
  explicit `ColumnType`, including for empty columns, and `WriteColumn::bits` now
  accepts packed `Vec<u8>` data directly. `ColumnType` is exported from the crate.
- `Header::scaling` now returns `Result<Scaling>` so malformed scaling metadata is
  distinguishable from absent metadata.
- `Header::get_logical`, `get_integer`, `get_real`, and `get_text` now return
  `Result<Option<_>>`; the parallel `try_get_*` family was removed. Time metadata
  accessors, `FitsTime::time_axis_mjd`, and `FitsReader::hdu_index` now return
  `Result` so malformed metadata is preserved as an error.
- `Value::Integer` and `Value::ComplexInteger` now carry exact `FitsInteger`
  values instead of bounded `i64` components. `Value::as_integer` and
  `Value::as_real` now return `Result<Option<_>>` so bounded conversions report
  range failures explicitly. `FitsInteger` is exported from the crate.
- Binary-table `A` columns now use `ColumnData::Character(Vec<CharacterField>)`,
  and ASCII tables use nullable `AsciiColumnData` cells so `TNULLn` remains distinct
  from genuine values. `ColumnData::Text` was removed, and `ColumnType::Text` was
  renamed to `ColumnType::Character`.
- `RawImage::decode` now consumes `self`, allowing already-owned decompressed samples
  to move out without cloning. With `ndarray`, `RawImage::to_ndarray(&self)` was
  replaced by the consuming `RawImage::into_ndarray(self)`.
- `FitsTime::gti_intervals` now returns `Result<Vec<GtiInterval>>` and rejects
  mismatched start/stop column lengths instead of silently truncating through `zip`.
- `FitsTime::unit_seconds` and `relative_to_mjd` now return `Result`; malformed or
  non-time units no longer silently behave as seconds. `time_axis_mjd` now accepts
  a parsed `Wcs` plus the full pixel vector and returns `TimeCoordinate`, including
  the axis-effective time scale.
- `EpochTime` was consolidated into `TimeCoordinate`; `Header::epoch` now returns
  the shared coordinate type used by both epoch keywords and WCS time axes.
- `Datetime::to_jd` and `to_mjd` now require a `TimeScale` and return `Result`;
  `Datetime::from_jd` also requires a scale. UTC values use a leap-second-preserving
  quasi-JD, and invalid scale/date combinations are rejected.
- Mutable WCS source fields (`naxis`, `ctype`, `crval`, `crpix`, and
  `unsupported_axes`) are now private so they cannot invalidate derived transforms.
  Read them through the immutable metadata returned by `Wcs::view`.
- `Wcs::pixel_to_world` and `Wcs::world_to_pixel` now return `Result<Vec<f64>>` so
  projection-domain, iterative-convergence, and unsupported-transform failures
  cannot be ignored. They no longer return linear-stage values as complete world
  coordinates when a nonlinear algorithm is unavailable.
- `FitsError::DataUnitTooLarge::bytes` changed from `usize` to `u64`. The
  `TypeMismatch`, `InvalidAscii`, `IntegerOutOfRange`, `UnsupportedWcsTransform`,
  `WcsProjectionDomain`, `WcsNoConvergence`, `PlioValueOutOfRange`, and
  `TableMetadataMismatch`, and `GroupIndexOutOfBounds` variants were added;
  exhaustive matches on `FitsError` must handle them.

### Added

- Added `FitsInteger`, an exact FITS integer value with an allocation-free `i64`
  representation and a decimal fallback for the standard's unbounded range.
- Added `CharacterField`, which preserves every stored byte of a binary-table `A`
  cell and exposes its first-NUL member boundary and null-string state.
- Added `Wcs::view` with immutable per-axis metadata and unsupported-axis status.
- Added `TimeCoordinate`, the shared MJD and effective scale value returned for
  epoch keywords and WCS time axes.
- Added explicit `ColumnType` declarations for variable-length table columns.
- Added `ColumnReader::vla_complex` and `vla_unsigned` for scaled complex P/Q
  heap arrays and exact unsigned-convention integers, including `u64` values that
  cannot be represented exactly as `f64`.
- Added `WriteColumn::vla_bits` for writing jagged `PX`/`QX` bit arrays from one
  exact-length `BitVec` per row.
- Added `RandomGroups::group_by_idx` and `RandomGroupView` for allocation-free,
  exact typed access to each group's stored parameters and array samples.

### Changed

- Structural and semantic metadata now fails with `TypeMismatch` when a card is
  present with the wrong representation. This applies to HDU layout, image, table,
  ASCII-table, random-groups, compression, WCS, and time metadata instead of
  treating the card as absent or silently applying a default.
- Fallible allocation is limited to reader staging and final decompression outputs
  whose sizes come directly from untrusted FITS metadata. Writer, encoder, and
  caller-owned buffers retain checked arithmetic but use ordinary `Vec` allocation.
- Header text, header comments, binary-table character members, ASCII-table text,
  units, names, and null markers are validated as FITS restricted ASCII before
  writing.
- Binary-table writing validates each column state before emitting data. `wide()` is
  restricted to VLA columns, bit columns require the exact packed byte count, and
  VLA rows must match their declared element type.
- `read_table` and `read_ascii_table` validate the HDU kind before reading or
  allocating its data unit, so wrong-kind calls return their semantic error first.
- The default feature list now contains only `parallel`; default builds still enable
  `compression` because `parallel` depends on it.

### Fixed

- Integer keyword values outside `i64` are preserved exactly instead of being
  reparsed through `f64`, and integral reals outside `i64` now return a range error
  instead of saturating. Unsigned-64 image and binary-table writers emit the exact
  normative `BZERO`/`TZERO = 9223372036854775808` decimal.
- Binary-table fixed and P/Q `A` cells preserve trailing spaces, NUL terminators,
  undefined bytes after the first NUL, and explicit null strings. Writing accepts
  NUL-terminated fields and rejects over-width fixed fields instead of truncating.
- Zero-length `P`/`Q` descriptor cells no longer fail their column's `TDIMn`
  product check. Shape syntax and every nonempty cell remain fully validated.
- HDU discovery now requires `SIMPLE` on the first card and `XTENSION` on each
  subsequent HDU, so special records containing later `END`-shaped cards are not
  misclassified. Boundary sizing uses the distinct primary, extension, and
  random-groups formulas and requires extension/group `PCOUNT` and `GCOUNT`.
- FITS time units now support standard SI prefixes, reject non-time units, and
  evaluate the epoch-dependent tropical and Besselian year definitions. Time axes
  now honor complete PC/CD rows, per-axis units, and per-axis time scales.
- WCS unsupported-axis classification now recognizes the standard `LOG` and `TAB`
  algorithms on any four-character coordinate type, including time and generic axes,
  instead of limiting nonlinear suffix detection to spectral coordinate names.
- Binary-table WCS now resolves the normative Table-22 primary and shortened
  alternate axis keywords, both pixel-list matrix/parameter aliases, column-indexed
  pole keywords, and alternate vector-cell rank inference.
- Complex binary-table scaling now applies `TZEROn` only to the real component,
  as required by FITS, while `TSCALn` continues to scale both components.
- Random-groups arrays now map integer samples equal to `BLANK` to `NaN` on the
  physical plane, matching ordinary primary-array behavior.
- ASCII-table character columns now preserve every byte of their fixed-width
  fields, including leading and trailing spaces; numeric parsing still trims padding.
- Tiled compression now preserves float images' original `BSCALE`/`BZERO`
  metadata across quantized and raw-float fallback tiles.
- Compressed-table writing now rejects header/table layout mismatches and original
  heap data instead of emitting self-contradictory or lossy containers. `THEAP`,
  `CHECKSUM`, and `DATASUM` metadata is translated through the standard `Z*`
  keywords and restored without retaining stale container values.
- GZIP, Rice, PLIO, and HCOMPRESS decoders now reject truncated streams, malformed
  control data, decompression bombs, and tiles whose decoded size differs from the
  declared geometry instead of manufacturing zero-valued pixels or reading out of
  bounds.
- Zero-sized images and one-pixel HCOMPRESS tiles no longer panic or fabricate
  pixels during tiled compression round trips.
- Reader, writer, image, table, VLA, and compressed-table dimensions now use checked
  narrowing and arithmetic. Overflow returns `DataUnitOverflow`; reader staging and
  decompression output allocation failures return `DataUnitTooLarge`.
- `FitsWriter` commits primary-HDU state only after the HDU write succeeds, so a
  retry after a failure that wrote no bytes still emits a primary HDU.
- One's-complement checksum accumulation now folds carry while streaming, avoiding
  accumulator overflow on very large valid HDUs.
- `P` and `Q` descriptors are constrained to their signed 32-bit and 64-bit FITS
  ranges. Character VLA descriptors use encoded byte counts, repeat-zero VLAs decode
  as typed empty cells, and nested `P`/`Q` descriptors are rejected.
- `TDIMn` accepts legal shapes whose product is less than or equal to the column
  repeat, validates VLA shapes against each cell, and rejects malformed or
  overflowing dimensions.
- Non-finite ASCII numeric cells require a valid, width-fitting null marker instead
  of writing a blank field that reads back as zero.
- WCS parsing now infers omitted axis counts from every supported indexed keyword,
  applies projection-specific parameter defaults, accepts legacy CDELT/CROTA beside
  CD while still rejecting PC conflicts, and flags shortened unknown celestial
  algorithms. Slant SIN parameters are evaluated instead of silently using the
  radial projection, and degenerate cylindrical projection scales are rejected.
  Mollweide transforms now return finite canonical coordinates at the poles. WCS
  transforms reject out-of-domain coordinates and failed Newton iterations instead
  of clamping them or returning an unconverged estimate.
- PLIO compression rejects values outside its lossless `0..=0xFF_FFFF` mask domain
  instead of silently clamping negative samples or truncating large ones.
- UTC/JD and UTC↔TAI/TT/UT1 conversion preserve leap-second instants with UTC
  quasi-JD day fractions and validate `second=60` against the actual insertion
  minute. FITS ISO-8601 parsing accepts only unsigned four-digit or signed
  five-digit years, including the standard's full signed range and JD origin.
  Signed Gregorian years use floor division, and GTI endpoints must have equal lengths.

### Performance and memory

- Raw-image physical, `f32`, and unsigned conversions decode big-endian samples
  directly into their final output instead of allocating an intermediate
  `ImageData` plane.
- Fixed-width table physical, unsigned, and complex reads decode strided cells
  directly into their final vectors. VLA physical reads decode bounded heap spans
  without retaining a second raw representation.
- Compressed image views decode directly into caller-owned aligned scratch instead
  of constructing and copying a complete intermediate image.
- Readers retain a checksum seed rather than every HDU's padded raw header, and
  checksum verification skips data I/O when neither checksum keyword is present.
- Compressed cells and regular writer VLAs append directly into their final table
  buffers with descriptor backpatching. Compressed tables decompress directly into
  disjoint final row/column ranges, avoiding compressed-cell and decompressed-table
  staging copies.
- Header restoration removes compression metadata and rebuilds its keyword index in
  one pass. Header cards render directly into a reusable writer header buffer,
  including long-string `CONTINUE` chains.
- Writer padding no longer allocates per data unit, and reusable buffers reserve from
  validated final sizes.
- ZPN projection values and derivatives are evaluated together with extended Horner.

### Documentation

- README Rust examples are compiled as doctests, and compression module docs were
  refreshed to describe the implemented codecs and tiled-table support.
