# Changelog

## Unreleased

### Breaking changes

- Public types are organized under `image`, `table`, `header`, `wcs`, `time`, and
  `io`; only common entry points remain re-exported at the crate root. Sealed
  source wrappers moved from the root to `io`.
- Reader data operations now use HDU indices exclusively. Resolve `EXTNAME` and
  optional `EXTVER` once with `FitsReader::hdu_index`; `HduSelector`,
  `HduHandle`, and `FitsReader::hdu` were removed.
- The `ndarray` feature and its mirrored image model were removed. Core image
  vectors and shape metadata remain sufficient for downstream adapter crates.
- The time layer preserves standard aliases, realization suffixes, and arbitrary
  local `TIMESYS` names without performing inter-scale chronometry.
  `TimeScale::convert`, `convert_dut1`, the embedded leap table,
  `Datetime::from_jd`, `FitsTime::gti_intervals`, and `PhaseAxis::fold` were
  removed. UTC leap-second labels that need external history return
  `ExternalTimeDataRequired`.
- `Column::tdisp` now retains the raw optional `TDISPn` string. A malformed
  display recommendation no longer prevents table decoding.
- `FitsWriter::write_table` and `write_ascii_table` now accept validated
  `TableBuilder` / `AsciiTableBuilder` values instead of a separate row count and
  column slice. The matching `*_with_header` methods use the same builders.
- `write_compressed_image` and `write_compressed_table` now accept the typed
  `Compression` enum. Codec-specific settings live in validated `Gzip` and
  `Hcompress` values, while tiling and float quantization remain in
  `CompressionOptions`.
- `Hcompress::lossy` now accepts a floating-point FITS `SCALE` noise multiplier
  instead of an absolute integer tile scale. `Hcompress` and `Compression` no
  longer implement `Eq`.
- `read_image_view` now returns `BorrowedImage`, pairing its scratch-backed
  `ImageView` with shape and scaling metadata. `RawImage` was renamed to
  `ReadImage`, and the owned result formerly called `UnsignedView` is now
  `UnsignedData`.
- Header authoring uses the primary fallible verbs `set`, `comment`,
  `push_comment`, and `push_history`. Ordered duplicate/insert/remove operations
  were added. Authored keywords must use the standard eight-character grammar;
  non-standard `HIERARCH` input is preserved only as opaque commentary.
- `FitsError` and the open-ended `HduKind`/`TableColumnData` metadata enums are
  non-exhaustive.
- `ChecksumReport` now exposes `datasum` and `checksum` as `ChecksumStatus`
  values (`Absent`, `Unknown`, `Valid`, or `Invalid`) instead of lossy
  `Option<bool>` fields.
- `Image::new(shape, samples)` now means identity scaling and accepts typed
  vectors through `Into<ImageData>`; custom scaling moved to `Image::new_scaled`.
  Geometry, samples, and scaling are immutable after construction, with geometry and stored-type
  metadata exposed through `Image::metadata` and exact immutable samples through
  `Image::stored`. The unsigned/signed-byte constructors now return `Result`.
  `ReadImage` likewise exposes immutable `ImageMetadata`, and `ReadImage::bitpix`
  derives the stored type from its backing representation instead of exposing a
  separately mutable tag.
- `FitsWriter::write_header` and `write_data_unit` were replaced by the atomic
  `write_raw_hdu`, which validates the header-implied data size and derives the
  correct block fill. A sink error now permanently fails the writer; subsequent
  writes return `FitsError::WriterFailed` instead of appending after a possibly
  torn HDU.
- `WriteColumn` is now an invariant-preserving opaque type instead of a collection
  of public, independently mutable fields. `WriteColumn::vla` infers its type
  from nonempty rows; `vla_typed` is the explicit-schema path required for an
  empty VLA. `WriteColumn::bits` accepts packed `Vec<u8>` data directly.
  `WriteColumn::vla`, `vla_typed`, and `wide` return `Result` for invalid states
  instead of panicking.
- Parsed `BinTable`, `AsciiTable`, `RandomGroups`, and `DataUnit` storage is now
  private so safe callers cannot desynchronize validated geometry from backing
  bytes. Read their immutable `*Metadata`/`DataUnitView` values instead.
- Random-groups physical-value methods now return `Result` and report an
  out-of-range group consistently with `group_by_idx`.
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
- `ReadImage::decode` now consumes `self`, allowing already-owned decompressed
  samples to move out without cloning.
- `FitsTime::unit_seconds` and `relative_to_mjd` now return `Result`; malformed or
  non-time units no longer silently behave as seconds. `time_axis_mjd` now accepts
  a parsed `Wcs` plus the full pixel vector and returns `TimeCoordinate`, including
  the axis-effective time scale.
- `FitsTime::trefpos` is now a resolved `TimeReferencePosition` instead of an
  optional string. `Header::phase_axis` accepts an alternate-WCS selector,
  and `PhaseAxis::period` is optional when no constant period exists.
- `EpochTime` was consolidated into `TimeCoordinate`; `Header::epoch` now returns
  the shared coordinate type used by both epoch keywords and WCS time axes.
- `Datetime::to_jd` and `to_mjd` require a declared `TimeScale` and return
  `Result`; invalid scale/date combinations are rejected.
- `TimeScale::parse` was replaced by the standard fallible `FromStr`
  implementation. Recognized aliases retain typed scale kinds, parenthesized
  realizations are preserved, and other nonempty labels remain local scale names.
- Mutable WCS source fields (`naxis`, `ctype`, `crval`, `crpix`, and
  `unsupported_axes`) are now private so they cannot invalidate derived transforms.
  Read them through the immutable metadata returned by `Wcs::view`.
- `Projection` now includes the standard `Tsc`, `Csc`, `Qsc`, and `Hpx` variants;
  exhaustive matches must handle them.
- `Wcs::pixel_to_world` and `Wcs::world_to_pixel` now return `Result<Vec<f64>>` so
  projection-domain, iterative-convergence, and unsupported-transform failures
  cannot be ignored. They no longer return linear-stage values as complete world
  coordinates when a nonlinear algorithm is unavailable, and coordinate-count
  mismatches return errors instead of panicking.
- `WcsNoConvergence` now names its payload `algorithm` instead of `projection`,
  covering iterative non-projection transforms such as `-TAB`.
- `FitsError::DataUnitTooLarge::bytes` changed from `usize` to `u64`. The
  `TypeMismatch`, `InvalidAscii`, `ReservedKeyword`, `InvalidHeaderValue`,
  `HeaderCardTooLong`, `AsciiFieldTooWide`, `IntegerOutOfRange`,
  `UnsupportedWcsTransform`, `WcsProjectionDomain`, `WcsCoordinateDomain`, `WcsNoConvergence`,
  `PlioValueOutOfRange`, `TableMetadataMismatch`, `GroupIndexOutOfBounds`,
  `CoordinateCountMismatch`, `WcsAxisIndexOutOfBounds`, `OneBasedIndexRequired`,
  `WriterFailed`, selector/range, table-builder, and external-time-data variants
  were added.

### Added

- Added checked N-dimensional image section reads. Plain sections coalesce
  contiguous source reads; compressed sections read and decompress only
  intersecting tiles.
- Added header-only `TableSchema` discovery plus source-bound table cell,
  selected-column, row, and range reads, including compact reads of referenced
  P/Q heap cells.
- Added `FitsWriter::stream_image`, `stream_image_scaled`, and
  `stream_image_with_header` for incremental image output to seekable sinks,
  including final count validation, padding, and checksum patching. Readers now
  recover their source through `into_inner` or `into_bytes`.
- Added inferred `TableBuilder` and `AsciiTableBuilder` construction,
  `WriteColumn::scalar`, consistent ASCII-column builders, and typed compression
  configuration.
- Added public WCS axis units and resolved celestial axis/projection/pole
  metadata.
- Added `FitsInteger`, an exact FITS integer value with an allocation-free `i64`
  representation and a decimal fallback for the standard's unbounded range.
- Added `CharacterField`, which preserves every stored byte of a binary-table `A`
  cell and exposes its first-NUL member boundary and null-string state.
- Added `Wcs::view` with immutable per-axis metadata and unsupported-axis status.
- Added typed `CelestialFrame`/`CelestialReferenceFrame` and per-axis
  `SpectralFrame`/`SpectralReferenceFrame` metadata, including standard defaults
  and the image, alternate, pixel-list, and vector-cell keyword forms.
- Added forward and inverse `TSC`, `CSC`, `QSC`, and parameterized `HPX`
  transforms, completing the FITS 4.0 celestial projection set.
- Added forward and inverse Table-26 `F2*`, `W2*`, `V2*`, and `A2*` spectral
  transforms plus generic `LOG`. Spectral transforms resolve image and table rest
  metadata and normalize declared units to their Table-25 defaults.
- Added detector-coordinate `GRI`/`GRA` spectral transforms and
  `FitsReader::read_wcs`, which resolves one- and multidimensional `-TAB`
  coordinate arrays and optional indexing vectors from their referenced
  `BINTABLE`.
- Added `TimeCoordinate`, the shared MJD and effective scale value returned for
  epoch keywords and WCS time axes.
- Added typed time reference positions, `Header::time_for_column` for `TRPOSn`
  overrides, and pixel-list/vector-cell PHASE metadata accessors.
- Added explicit `ColumnType` declarations for empty or predeclared
  variable-length table columns.
- Added `ColumnReader::vla_complex` and `vla_unsigned` for scaled complex P/Q
  heap arrays and exact unsigned-convention integers, including `u64` values that
  cannot be represented exactly as `f64`.
- Added `WriteColumn::vla_bits` for writing jagged `PX`/`QX` bit arrays from one
  exact-length `BitVec` per row.
- Added `RandomGroups::group_by_idx` and `RandomGroupView` for allocation-free,
  exact typed access to each group's stored parameters and array samples.
- Added `FitsWriter::write_image_with_header`, `write_table_with_header`,
  `write_ascii_table_with_header`, and `write_compressed_image_with_header`.
  These typed paths preserve ordered informational cards—including WCS/time,
  `COMMENT`, and `HISTORY`—while regenerating layout, compression, and checksum
  cards from the typed payload.
- Added immutable `BinTableMetadata`, `AsciiTableMetadata`,
  `RandomGroupsMetadata`, and `DataUnitView` values for inspecting sealed parsed
  objects. `DataUnit::into_data` and `into_padded` recover meaningful or complete
  owned bytes without exposing mutable internal ranges.

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
- Public header mutation is fallible through `Header::set`, `comment`,
  `push_comment`, and `push_history`. Invalid/reserved keywords,
  non-ASCII text, nonfinite numeric values, and cards longer than 80 bytes are
  rejected without changing the header; the former panicking mutation methods
  are no longer public.
- Binary-table writing validates each column state before emitting data. `wide()` is
  restricted to VLA columns, bit columns require the exact packed byte count, and
  VLA rows must match their declared element type.
- Every writer path now finishes header/data preparation before emitting an
  automatic primary HDU. Validation and compression failures therefore leave a
  fresh sink empty, while partial sink failures poison the writer.
- Image and table writers reject nonfinite scaling metadata, out-of-range or
  inapplicable `BLANK`/`TNULLn` sentinels, and scaling keywords forbidden for the
  stored column type before writing any bytes.
- ASCII and binary table writers reject more than 999 fields before allocating
  per-column state or emitting an automatic primary HDU.
- ASCII-table writing rejects text and formatted numeric values wider than their
  declared fields instead of replacing them with non-conforming `*` characters.
- `read_table` and `read_ascii_table` validate the HDU kind before reading or
  allocating its data unit, so wrong-kind calls return their semantic error first.
- Multidimensional `-TAB` inversion now returns `WcsNoConvergence` after a
  deterministic work budget instead of allowing exponential subdivision to run
  without a practical bound.
- The default feature list now contains only `parallel`; default builds still enable
  `compression` because `parallel` depends on it.

### Fixed

- Reject `-TAB` table-axis indices and interpolation dimensionality that would
  trigger oversized allocations, overflowing vertex counts, or infeasible
  per-coordinate work.
- Integer keyword values outside `i64` are preserved exactly instead of being
  reparsed through `f64`, and integral reals outside `i64` now return a range error
  instead of saturating. Unsigned-64 image and binary-table writers emit the exact
  normative `BZERO`/`TZERO = 9223372036854775808` decimal.
- Binary-table fixed and P/Q `A` cells preserve trailing spaces, NUL terminators,
  undefined bytes after the first NUL, and explicit null strings. Writing accepts
  NUL-terminated fields and rejects over-width fixed fields instead of truncating.
- Orphan `CONTINUE` records retain their quoted payload and comment as commentary,
  and comments spread across a long-string chain are concatenated instead of
  replacing earlier fragments. Header rendering returns `HeaderCardTooLong`
  rather than clipping an over-width card or final long-string comment.
- Fixed-width binary `X` columns now clear unused low bits in every row's final
  byte, as required for non-byte-aligned bit arrays.
- Zero-length `P`/`Q` descriptor cells no longer fail their column's `TDIMn`
  product check. Shape syntax and every nonempty cell remain fully validated.
- HDU discovery now requires `SIMPLE` on the first card and `XTENSION` on each
  subsequent HDU, so special records containing later `END`-shaped cards are not
  misclassified. Boundary sizing uses the distinct primary, extension, and
  random-groups formulas and requires extension/group `PCOUNT` and `GCOUNT`.
- FITS time units now support standard SI prefixes, reject non-time units, and
  numeric multipliers, and evaluate the epoch-dependent tropical and Besselian
  year definitions. Non-time units are rejected. Time axes now honor complete
  PC/CD rows, per-axis units, and per-axis time scales.
- WCS coordinate algorithms recognize generic `LOG` and `TAB` independently of
  the coordinate type. `LOG` is evaluated for spectral, time, and generic axes;
  header-only `TAB` remains explicitly unsupported, while `FitsReader::read_wcs`
  supplies the required table-data context.
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
- Compressed-table writing now supports fixed columns, P/Q heap arrays, and
  `NOCOMPRESS`. VLA streams are compressed separately, their Gzip-compressed
  descriptor block uses the FITS 4.0 ordering, and reading also accepts existing
  CFITSIO files that reverse the two descriptor arrays. `THEAP`, `PCOUNT`,
  `CHECKSUM`, and `DATASUM` metadata is translated through the standard `Z*`
  keywords and restored without retaining stale container values.
- `RICE_1` handles the standard `BYTEPIX = 8` form without lossy floating-point
  statistics or 32-bit bit-buffer truncation. Odd final blocks use the canonical
  integer half-block statistic, with a bit-exact CFITSIO regression. Compression
  parameter scans honor canonical `ZNAMEi` records after omitted/defaulted indices
  and reject nonstandard Rice parameter values; an omitted `BYTEPIX` uses Table
  37's four-byte default rather than inferring the logical image type.
- HCOMPRESS writing enforces two-dimensional images, records `SCALE` as a real
  noise multiplier, derives each tile's absolute stream scale from its measured
  noise, and omits the obsolete `SMOOTH=0` parameter. Its signed 64-bit transform
  now reads and writes wide `sumall` values and up to 63 bit planes, matches an
  external 35-plane golden byte-for-byte, fixes the odd-dimension corner
  coefficient, and safely rejects only transforms that cannot fit the stream.
  Float Gzip/NOCOMPRESS output no longer emits Rice-only `BYTEPIX` parameters.
- Compressed-image reading applies `NULL_PIXEL_MASK` tiles and restores integer
  `BLANK` values or floating NaNs for Gzip, Rice, PLIO, and `NOCOMPRESS` mask
  codecs. Lossy HCOMPRESS writing rejects tiles containing the declared `BLANK`
  until null-mask authoring is available.
- Random-groups `NAXIS = 1` sizing uses the empty product required by Eq. 4, and
  generic grouped HDUs accept the standard nonnegative `GCOUNT` domain.
- `TIMEPIXR` and `ZDITHER0` now enforce their normative ranges.
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
- FITS ISO-8601 parsing accepts only unsigned four-digit or signed five-digit
  years, including the standard's full signed range and JD origin. Signed
  Gregorian years use floor division. Leap-second labels are accepted only in
  the final UTC minute and require external time data before JD conversion.

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
- `read_wcs` materializes only the first lookup-table row and the referenced
  coordinate/index VLA cells. Multidimensional `-TAB` inversion reuses one search
  workspace and evaluates subvoxel corners with separable interpolation.
