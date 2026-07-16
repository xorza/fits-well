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
- `RawImage::decode` now consumes `self`, allowing already-owned decompressed samples
  to move out without cloning. With `ndarray`, `RawImage::to_ndarray(&self)` was
  replaced by the consuming `RawImage::into_ndarray(self)`.
- `FitsTime::gti_intervals` now returns `Result<Vec<GtiInterval>>` and rejects
  mismatched start/stop column lengths instead of silently truncating through `zip`.
- Mutable WCS source fields (`naxis`, `ctype`, `crval`, `crpix`, and
  `unsupported_axes`) are now private so they cannot invalidate derived transforms.
  Read them through the immutable metadata returned by `Wcs::view`.
- `Wcs::pixel_to_world` and `Wcs::world_to_pixel` now return `Result<Vec<f64>>` so
  projection-domain and iterative-convergence failures cannot be ignored.
- `FitsError::DataUnitTooLarge::bytes` changed from `usize` to `u64`. The
  `TypeMismatch`, `InvalidAscii`, `WcsProjectionDomain`, `WcsNoConvergence`, and
  `PlioValueOutOfRange` variants were added; exhaustive matches on `FitsError`
  must handle them.

### Added

- Added `Wcs::view` with immutable per-axis metadata and unsupported-axis status.
- Added explicit `ColumnType` declarations for variable-length table columns.

### Changed

- Structural and semantic metadata now fails with `TypeMismatch` when a card is
  present with the wrong representation. This applies to HDU layout, image, table,
  ASCII-table, random-groups, compression, WCS, and time metadata instead of
  treating the card as absent or silently applying a default.
- Fallible allocation is limited to reader staging and final decompression outputs
  whose sizes come directly from untrusted FITS metadata. Writer, encoder, and
  caller-owned buffers retain checked arithmetic but use ordinary `Vec` allocation.
- Header text, header comments, binary-table text, ASCII-table text, units, names,
  and null markers are validated as FITS restricted ASCII before writing.
- Binary-table writing validates each column state before emitting data. `wide()` is
  restricted to VLA columns, bit columns require the exact packed byte count, and
  VLA rows must match their declared element type.
- `read_table` and `read_ascii_table` validate the HDU kind before reading or
  allocating its data unit, so wrong-kind calls return their semantic error first.
- The default feature list now contains only `parallel`; default builds still enable
  `compression` because `parallel` depends on it.

### Fixed

- Complex binary-table scaling now applies `TZEROn` only to the real component,
  as required by FITS, while `TSCALn` continues to scale both components.
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
- TT-to-UTC/UT1 conversion now selects leap seconds at the correct UTC instant.
  Signed Gregorian years use floor division, ISO-8601 parsing requires the FITS year
  and `hh:mm:ss` forms, and GTI endpoints must have equal lengths.

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
