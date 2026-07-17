# Changelog

## Unreleased

### Breaking changes

- `ChecksumReport` now exposes `datasum` and `checksum` as `ChecksumStatus`
  values (`Absent`, `Unknown`, `Valid`, or `Invalid`) instead of lossy
  `Option<bool>` fields.
- `Image` is now constructed with fallible `Image::new`; its geometry, samples,
  and scaling are immutable after construction, with geometry and stored-type
  metadata exposed through `Image::metadata` and exact immutable samples through
  `Image::stored`. The unsigned/signed-byte constructors now return `Result`.
  `RawImage` likewise exposes immutable `ImageMetadata`, and `RawImage::bitpix`
  derives the stored type from its backing representation instead of exposing a
  separately mutable tag. `ImageData::into_ndarray` now returns `Result` when a
  caller-supplied shape has the wrong element count.
- `FitsWriter::write_header` and `write_data_unit` were replaced by the atomic
  `write_raw_hdu`, which validates the header-implied data size and derives the
  correct block fill. A sink error now permanently fails the writer; subsequent
  writes return `FitsError::WriterFailed` instead of appending after a possibly
  torn HDU.
- `WriteColumn` is now an invariant-preserving opaque type instead of a collection
  of public, independently mutable fields. `WriteColumn::vla` now requires an
  explicit `ColumnType`, including for empty columns, and `WriteColumn::bits` now
  accepts packed `Vec<u8>` data directly. `WriteColumn::vla` and
  `WriteColumn::wide` now return `Result` for mismatched cell types and non-VLA
  columns instead of panicking. `ColumnType` is exported from the crate.
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
- `RawImage::decode` now consumes `self`, allowing already-owned decompressed samples
  to move out without cloning. With `ndarray`, `RawImage::to_ndarray(&self)` was
  replaced by the consuming `RawImage::into_ndarray(self)`.
- `FitsTime::gti_intervals` now returns `Result<Vec<GtiInterval>>` and rejects
  mismatched start/stop column lengths instead of silently truncating through `zip`.
- `FitsTime::unit_seconds` and `relative_to_mjd` now return `Result`; malformed or
  non-time units no longer silently behave as seconds. `time_axis_mjd` now accepts
  a parsed `Wcs` plus the full pixel vector and returns `TimeCoordinate`, including
  the axis-effective time scale.
- `FitsTime::trefpos` is now a resolved `TimeReferencePosition` instead of an
  optional string. `Header::phase_axis` accepts an alternate-WCS selector,
  `PhaseAxis::period` is optional when no constant period exists, and
  `PhaseAxis::fold` returns `Result` instead of silently returning phase zero.
- `EpochTime` was consolidated into `TimeCoordinate`; `Header::epoch` now returns
  the shared coordinate type used by both epoch keywords and WCS time axes.
- `Datetime::to_jd`, `to_mjd`, and `from_jd` now require a `TimeScale` and return
  `Result`; `from_jd` rejects non-finite values and dates outside the representable
  FITS year range. UTC values use a leap-second-preserving quasi-JD, and invalid
  scale/date combinations are rejected.
- `TimeScale::parse` was replaced by the standard fallible `FromStr`
  implementation, and `convert`/`convert_dut1` now return `Result`. Unknown labels
  are rejected, only literal `LOCAL` selects the local scale, and conversions
  between local and defined scales are errors. Non-finite or
  calendar-unrepresentable Julian Dates are rejected.
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
- `FitsError::DataUnitTooLarge::bytes` changed from `usize` to `u64`. The
  `TypeMismatch`, `InvalidAscii`, `ReservedKeyword`, `InvalidHeaderValue`,
  `HeaderCardTooLong`, `AsciiFieldTooWide`, `IntegerOutOfRange`,
  `UnsupportedWcsTransform`, `WcsProjectionDomain`, `WcsCoordinateDomain`, `WcsNoConvergence`,
  `PlioValueOutOfRange`, `TableMetadataMismatch`, `GroupIndexOutOfBounds`,
  `CoordinateCountMismatch`, `WcsAxisIndexOutOfBounds`, `OneBasedIndexRequired`,
  and `WriterFailed` variants were added; exhaustive matches on `FitsError` must
  handle them.

### Added

- Added fallible `Header::try_set_hierarch` authoring and update support for ESO
  HIERARCH compound keywords.
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
- Added explicit `ColumnType` declarations for variable-length table columns.
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
- Public header mutation is fallible through `Header::try_set`, `try_comment`,
  `try_push_comment`, and `try_push_history`. Invalid/reserved keywords,
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
