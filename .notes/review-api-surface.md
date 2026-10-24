# Review: fits-well public API

Date: 2026-10-24, at `5278502`. Scope: every item `.notes/API_SURFACE.md` lists (262
items over no features and each public feature). Lenses: consistency, symmetry,
solidity, convenience, canonical and idiomatic Rust (the API Guidelines, std).

**Decided with the user on 2026-10-24:** one header name type; `Display` for all four
`FromStr` types; all three namespace moves (`error`, `compress`, the WCS renames).

## Part 1: Proposal

### Root causes

- **R1. Parallel APIs grew per case instead of one API over a shared type.**
  - HIERARCH has its own `get_hierarch`, `get_hierarch_comment`, `set_hierarch` and
    `remove_hierarch`, and no typed getter, `require_*`, `contains` or `index_of`.
  - Commentary has `push_comment`, `push_history`, `push_blank`, `comment_records` and
    `history_records`, one per name, and none for another `CommentaryName`.
  - Column-kind refusals are five variants (`NotAFixedColumn`, `NotAVla`,
    `NotABitColumn`, `NotAComplexColumn`, `NotANumericColumn`, each `{ code: char }`)
    plus a stringly `TypeMismatch { name: String, expected: &'static str }`.
  - HDU-kind refusals are five variants (`NotAnImage`, `NotABinTable`,
    `NotRandomGroups`, `NotAnAsciiTable`, `ImageHasGroups`) beside `HduKindChange
    { from, to }`.
- **R2. Twin types drifted apart.**
  - `Schema` hides its extent behind `row_count()`/`row_len()`; `AsciiSchema` has
    `pub row_count`, `pub row_len`, `pub columns`.
  - `Column::byte_offset()` is a method; `AsciiColumn::start` is a field.
  - `Table::column` errors with `ColumnNotFound`; `Selection::column` gives `Option`.
  - The `as_*` accessors are `const` on `UnsignedData`, `UnsignedView` and
    `ColumnView`, and not on `Data`, `View` and `ColumnData`.
  - `Keyword` has `TryFrom<&str | String | &String | Arguments>` and no `FromStr`;
    `HierarchName` has `FromStr` and `TryFrom<&str>`; `CommentaryName` only
    `TryFrom<&str>`.
- **R3. Strings where the crate has the type.**
  - `WcsPixelOutOfDomain`, `WcsWorldOutOfDomain` and `DegenerateProjection` carry
    `projection: &'static str`, and `wcs::Projection` exists.
  - `InvalidAsciiValue { reason: &'static str }`, `ExternalTimeDataRequired
    { operation: &'static str }` and `InvalidPqDescriptor { field: &'static str }` are
    messages, not cases.
- **R4. Conversions spelled as constructors.** `Bitpix::from_code(i64)`,
  `DitherSeed::fixed(u16)` and `Gzip::new(u32)` are `TryFrom` in all but name.
  `Tform`, `Datetime`, `time::Scale` and `time::Unit` parse (`FromStr`) and do not
  print (`Display`), so a value read cannot be written back as text.
- **R5. Namespaces that hold what is not theirs.**
  - The crate root holds 16 error helper enums (`Indexed`, `Ranked`, `SizeUnit`,
    `TextOrigin`, `ValueKind`, `NumericTarget`, `NonFinite`, `TileCell`, and the
    defect enums) beside the four types a caller starts from.
  - `image::Compression` and `image::CompressionConfig` configure table compression
    too (`write_compressed_table`).
  - `io` holds the HDU model, checksums, random groups, sources and block sizes.
  - Two `Axis` (`time::Axis`, `wcs::Axis`) and two `Metadata` (`image::Metadata`,
    `wcs::Metadata`): a caller that names both must alias one.
- **R6. Silent or panicking edges.**
  - `Hdu::extname` maps a non-text `EXTNAME` to `None` (`.ok().flatten()`), where
    `extver` and `extlevel` error.
  - `Wcs::world_unit(axis)` indexes and panics past the last axis, where `axis_world`
    errors with `IndexOutOfBounds`.
  - `AxisWorld::cunit` is the world unit `world_unit` gives, which differs from the
    axis's `CUNITi` after unit scaling.
- **R7. Convenience.**
  - A whole-image read is `reader.image(0)?.read(Region::All)`; `read(..)` is the std
    idiom (`From<RangeFull>`).
  - `Image` has no `shape()`, `scaling()` or `samples()`: every read of them goes
    through `view()`. There is `into_samples` and no `into_parts`.
  - Three of `image::Sample`'s public methods (`view`, `data`, `of_data`) repeat
    `From<&[T]> for View`, `From<Vec<T>> for Data` and `TryFrom<Data> for Vec<T>`; the
    fourth, `of`, is the one generic slice extraction, which no conversion trait gives.

### Target design

- **One header name type.** Every keyed `Header` method takes `impl TryInto<RecordName>`
  (or a `Name` that parses `"HIERARCH ESO DET"` and a plain keyword alike), so
  HIERARCH gets every typed getter, `require_*`, `contains` and `index_of`, and the
  four `*_hierarch` methods go.
- **One commentary API.** `push_commentary(name: CommentaryName, text)` and
  `commentary(name) -> impl Iterator<Item = &str>` replace the five per-name methods.
  The inline comment keeps `get_comment`/`set_comment`.
- **Errors by case.**
  - `ColumnKindMismatch { column, expected: ColumnAccess, element: ElementKind }`
    replaces the five `NotA*Column` and `TypeMismatch`; `ColumnAccess` names what the
    read wanted (fixed, array, bits, complex, numeric).
  - `HduKindMismatch { expected: HduKind, got: HduKind }` replaces the five HDU-kind
    variants; `ImageHasGroups` stays its own case, as no kind names it.
  - `Projection` in the three projection errors; an enum for each string reason left.
- **Twins agree.**
  - `AsciiSchema` takes `Schema`'s shape: private fields, `row_count()`, `row_len()`,
    `columns`.
  - `AsciiColumn::byte_offset()` beside `width`, as `Column`.
  - `const` on every `as_*` that compiles so.
  - All three name types implement `FromStr`, `TryFrom<&str>` and `TryFrom<String>`.
- **Conversions as traits.** `TryFrom<i64> for Bitpix`; `Display` for `Tform`, `Datetime`, `Scale` and `Unit`,
  each the inverse of its `FromStr`, as `Keyword` already does, with a round-trip test
  for each.
- **Namespaces.**
  - `fits_well::error` holds `FitsError`'s helper enums; the root keeps `FitsError`,
    `Result`, the reader, writer and editor.
  - `fits_well::compress` holds `Compression`, `CompressionConfig`, `Quantization`,
    `Dither`, `DitherSeed`, `Gzip`, `Hcompress`.
  - `wcs::Axis` → `wcs::AxisDescription`, `wcs::Metadata` → `wcs::Description`; the
    time and image names keep their short forms.
- **Solid edges.** `extname` errors as `extver` does (`Result<Option<&str>>`);
  `world_unit` gives `Result` with `IndexOutOfBounds`; `AxisWorld::cunit` →
  `AxisWorld::unit`.
- **Convenience.** `From<RangeFull> for Region`; `Image::shape`, `scaling`,
  `samples` and `into_parts`; `TryFrom<View<'a>> for
  &'a [T]` in place of `Sample::of`, so `Sample` keeps `BITPIX` alone and its
  conversions are the std traits.

### Refused

- **No `get_` drop on `Header`'s typed getters.** The guideline against `get_` is for
  field getters; a keyed lookup is `HashMap::get`, and `get_real(keyword)` reads as one.
- **No `TableReader` handle like `ImageReader`.** An image handle carries a checksum
  pass and a borrow of the caller's words; a table read has neither, and four
  methods on the reader cover it. A handle would add a type and a lifetime for the
  same calls.
- **No `new` in place of `open` on the reader and the editor.** `open` reads and
  validates the file's HDU list, which a `new` does not promise.
- **No request struct for `read_table_columns_into`.** It stays the out-parameter twin
  of `read_table_columns`.
- **No rename of `gcount`/`pcount` in `RandomGroupsMetadata`.** design.md keeps a
  FITS keyword's own name for a count.
- **No `FromIterator<Record>` for `Header`, and no `Extend<Record>`.** A record
  must render, so a `collect` would panic on caller data; `insert_record` refuses
  it. The `Extend` that panicked is removed.
- **No `Result` from `Selection::column`.** It is a lookup in the columns the caller
  selected, as `Schema::column_index` is, and no error case fits an index the table
  holds and the selection does not.
- **No `TryFrom<u16>` for `DitherSeed`.** `DitherSeed::fixed(17)` names the kind of
  seed beside `DitherSeed::FROM_SAMPLES`, which `try_from(17)` would not.
- **No change to `write_raw_hdu(header, raw)` argument order.** Its header is the
  HDU's own, not a template, which the other writes take last as an `Option`.

## Part 2: Plan

Each batch leaves `cargo drive gate` green, updates `docs/design.md` with its
decisions, regenerates `.notes/API_SURFACE.md`, and lists the lumos call sites it
moves.

1. **Solid edges and convenience** (done).
2. **Twins agree** (done).
3. **Errors by case** (done).
4. **Header names**: one name type for keyed methods; the commentary API.
5. **`Display` for the `FromStr` types.**
6. **Namespaces**: `error` and `compress` modules, the WCS renames.
