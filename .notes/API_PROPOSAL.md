# fits-well API proposal

2026-10-23. This document puts every finding of `API_REVIEW.md` and every entry of `ISSUES.md` into one design and one plan of work. The review keeps the evidence: the `file:line` references and the comparison with other libraries. Here, each item names the review section that it answers.

This proposal is temporary. When a phase lands, its decisions go into `docs/design.md`, and the items that it closes are removed from here and from `ISSUES.md`. Phase 1, the quiet traps, has landed.

## Principles

- **One borrowed image type for reads and writes.** The reader gives an `ImageView`, the writer takes an `ImageView`, and `Image::view()` makes one from owned data. Then a caller never copies data only to satisfy a signature.
- **Parse at the boundary, one time.** Keywords, HDU names and WCS alternates are types, checked when they are made. A header lookup cannot fail because of the spelling of its argument.
- **One entry point for each operation.** A region selects the part of an image, a selector selects an HDU or a column, and a config selects the compression. No suffix such as `_view` or `_summed` multiplies the methods.
- **The caller owns the large buffers. The reader owns only small staging buffers.** The documented decision of `read_image_view` stays.
- **No quiet result.** An operation that cannot do what its name says returns an error. It does not return the container, do nothing, or return `None`.

## The design

### Reading images

The review sections that this answers: "One entry point", "Physical conversion into a caller buffer", "Typed access to `View` and `ColumnData`", "Memory planning", and "Header borrows against data reads", partly.

```rust
impl<S: Source> FitsReader<S> {
    pub fn image<'r>(&'r mut self, hdu: impl Into<HduSelector<'_>>) -> Result<ImageReader<'r, S>>;
}

pub enum Region<'a> { All, Section(&'a [Range<usize>]) }    // From<&[Range<usize>]> and From<&[Range<usize>; N]>

impl<'r, S: Source> ImageReader<'r, S> {
    pub fn header(&self) -> &'r Header;             // lives as long as the reader borrow, not the handle borrow
    pub fn metadata(&self) -> Metadata<'r>;
    pub fn with_checksum(self) -> Self;              // feeds the plain bytes of each read to a DataChecksum
    pub fn view<'a>(&'a mut self, region: impl Into<Region<'_>>, words: &'a mut Vec<u64>) -> Result<ImageView<'a>>;
    pub fn read(&mut self, region: impl Into<Region<'_>>) -> Result<Image>;
    pub fn physical_into<T: PhysicalSample>(&mut self, region: impl Into<Region<'_>>, out: &mut [T]) -> Result<()>;
    pub fn scratch_bytes(&self, region: impl Into<Region<'_>>) -> Result<usize>;
    pub fn finish(self) -> Result<ChecksumReport>;   // reads what the reads did not feed, as finish_data_checksum does now
}
```

**What goes away**
- All six `read_image*` methods.
- `Lazy`.
- `begin_data_checksum` and `finish_data_checksum`, which become private.
- `Borrowed`, which becomes `ImageView`.

**What the handle gives**
- **Parsing:** the handle parses the image geometry and the tiled-image header one time. Today, each section call runs `TiledImage::new` again.
- **Zero copy:** `Region::All` keeps the zero-copy path of `read_image_view`.
- **Single-pass conversion:** `physical_into` converts the big-endian bytes to physical values in one pass, with `BLANK → NaN`. It does not fill `words`. The arithmetic already exists as `physical_from_be`. `PhysicalSample` is the private `PhysicalOut`, made public and sealed. The element type of `out` selects it, so a caller never writes a turbofish.
- **Checksum:** `DataChecksum` already sums only the bytes that continue the sum, and at the finish it reads the rest. So `with_checksum` changes nothing for a caller that does not finish. The `_summed` variants are not necessary.

**`ImageView`** (the former `Borrowed`) has these methods:
- `new(shape, samples, scaling) -> Result`, which validates the geometry and the scaling
- `shape`, `scaling`, `samples` and `metadata`
- `physical_into`, `unsigned`, `sample_type` and `to_image()`

`Image` keeps its owned data. `Image::view()` gives an `ImageView`. `Image::physical_f64` (renamed from `physical`), `physical_f32`, `unsigned` and `sample_type` call the `ImageView` code, so each conversion has one implementation.

**Typed access:**
- `View::as_f32() -> Option<&[f32]>`, and the same for each variant
- `TryFrom<Data> for Vec<T>`, the reverse of the `From` impls

### Writing

The review sections that this answers: "Borrowed data on every write path", "One compression config, and compressed tables from a `Builder`", "Structural headers", and "Unsigned constructors".

```rust
impl<W: Write> FitsWriter<W> {
    pub fn write_image(&mut self, image: ImageView<'_>, header: Option<&Header>) -> Result<()>;
    pub fn write_compressed_image(&mut self, image: ImageView<'_>, config: &CompressionConfig, header: Option<&Header>) -> Result<()>;
    pub fn write_table(&mut self, table: &TableBuilder, header: Option<&Header>) -> Result<()>;
    pub fn write_compressed_table(&mut self, table: &TableBuilder, config: &CompressionConfig, header: Option<&Header>) -> Result<()>;
    pub fn write_ascii_table(&mut self, table: &AsciiTableBuilder, header: Option<&Header>) -> Result<()>;
    pub fn write_empty_primary(&mut self, header: Option<&Header>) -> Result<()>;   // NAXIS = 0; an error when it is not the first HDU
    pub fn write_raw_hdu(&mut self, header: &Header, raw: &[u8]) -> Result<()>;
}
impl<W: Write + Seek> FitsWriter<W> {
    pub fn stream_image(&mut self, layout: Metadata<'_>, header: Option<&Header>) -> Result<Writer<'_, W>>;
}
impl Writer<'_, W> { pub fn write_chunk(&mut self, samples: View<'_>) -> Result<()>; }

pub struct CompressionConfig { /* codec, tiles, quantization */ }
impl CompressionConfig {
    pub fn new(codec: Compression) -> Self;
    pub fn tiled(self, tiles: impl Into<Vec<usize>>) -> Self;     // a table takes one axis: rows per tile
    pub fn with_quantization(self, level: f64, dither: DitherMethod) -> Result<Self>;
}
```

**Why this shape**
- **Borrowed images.** Both encoders already work from a view (`Data::encode_into`), so a borrowed write costs nothing. It removes the whole-image `to_vec()` of a caller that keeps its buffer, and the buffer juggling for each chunk in lumos's `plane_hdu.rs`.
- **Signatures.** `write_compressed_table` now matches `write_table`: it takes the builder, and the header is an `Option` in the last place. Internally it encodes the builder into the writer's scratch, as `write_table` does, then compresses that table. So the compressor does not change.
- **`stream_image` takes the reader's `Metadata`** (shape, bitpix and scaling). Thus the method has two parameters, not four, and it is symmetric with the reader.
- **`CompressionConfig` holds the codec,** so the codec and its settings are no longer separate arguments. The name follows the workspace rule that refuses `*Options`.
  - A table write fails with `InvalidCompressionParameter` when the tiles have more than one axis or quantization is set.
- **`write_empty_primary`** replaces the four hand-written structural cards. The writer already adds an empty primary by itself before a first table or compressed image (`Opening::EmptyPrimary`). This method is for a primary that carries the caller's keywords.
- **`Image::from_u16/u32/u64/i8`** take `shape: impl Into<Vec<usize>>`, as `Image::new` does.

### HDUs

The review sections that this answers: "`HduSelector`" and "Classification".

```rust
pub enum HduSelector<'a> {
    Index(usize),
    Extension { name: &'a str, version: Option<i64>, level: Option<i64> },
}   // From<usize>, From<&'a str>, From<(&'a str, i64)>

impl<S: Source> FitsReader<S> {
    pub fn hdu(&self, hdu: impl Into<HduSelector<'_>>) -> Result<&Hdu>;
    pub fn hdu_index(&self, hdu: impl Into<HduSelector<'_>>) -> Result<usize>;   // FitsError::HduNotFound
    pub fn images(&self) -> impl Iterator<Item = usize> + '_;
}
```

- **Every reader method takes `impl Into<HduSelector>`.**
- **A name matches `EXTNAME` case-insensitively.** A primary HDU with no `EXTNAME` matches `PRIMARY`, as in astropy.
- **`Hdu::matches_extension` becomes `Hdu::matches(&HduSelector)`.** Then the reader and the HDU accept the same arguments, and `level` is no longer lost.
- **`image_indices() -> Vec` becomes `images()`, an iterator.** "The first image" is `images().next()`, and the README uses it.
- **`Hdu::image()` becomes `Hdu::image_metadata()`**, beside `Hdu::table_schema()`.
- **New:** `Hdu::padded_data_bytes()`.

### Header

The review sections that this answers: "A typed `Keyword`", "Optional and required values", "Comments, commentary cards and collection traits", "`Value` conversions", and the `MissingKeyword` part of "Errors". It also closes the `get` case trap of `ISSUES.md`.

```rust
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub struct Keyword { /* [u8; 8] and a length */ }
impl Keyword {
    pub const fn new(name: &str) -> Self;                          // panics on an invalid name; in a const, at compile time
    pub fn indexed(root: Self, index: u32) -> Result<Self>;        // CTYPE + 2 = CTYPE2, with no allocation
    pub fn as_str(&self) -> &str;
}   // TryFrom<&str> for names from data; Display

pub mod header::keywords { pub const SIMPLE: Keyword = …; /* the reserved keywords of §4.4 and of the conventions fits-well reads */ }

impl Header {
    pub fn get(&self, keyword: Keyword) -> Option<&Value>;
    pub fn entry(&self, keyword: Keyword) -> Option<Entry<'_>>;              // value and comment
    pub fn get_logical / get_integer / get_real / get_text(&self, Keyword) -> Result<Option<T>>;
    pub fn require_logical / require_integer / require_real / require_text(&self, Keyword) -> Result<T>;   // MissingKeyword
    pub fn set(&mut self, keyword: Keyword, value: impl Into<Value>) -> Result<&mut Self>;
    pub fn set_optional(&mut self, keyword: Keyword, value: Option<impl Into<Value>>) -> Result<&mut Self>;   // None removes the card
    pub fn set_comment(&mut self, keyword: Keyword, text: &str) -> Result<&mut Self>;    // KeywordNotFound when it is absent
    pub fn len, is_empty, contains(Keyword), index_of(Keyword) -> Option<usize>;
    pub fn history(&self) / comments(&self) -> impl ExactSizeIterator<Item = &str>;
    pub fn iter(&self) -> impl ExactSizeIterator<Item = Entry<'_>>;          // and IntoIterator for &Header
}
```

**Why a type and not `&str`**
- **The spelling trap goes away.** A lowercase or invalid name cannot become a `Keyword`, so the case trap of `get` cannot happen.
- **Constants are checked at compile time.** lumos already names its keywords as constants: `const PRODUCT_KEYWORD: Keyword = Keyword::new("…")` fails the build on a typo.
- **No allocation for indexed keywords.** `Keyword::indexed` replaces each `format!`.
- **Parsing a header allocates less.** Today it allocates two `String`s for each valued card: the `Card` keyword (`header_model/card/mod.rs`) and the index key (`header_model/mod.rs:92`). A `Copy` key removes both.
- **The reserved-keyword lists become constants that callers can use.** Today they are the `&str` arrays in `reserved_keywords.rs`.

**`set` stays fallible.** A NaN or infinite real and non-ASCII text are invalid in FITS, so `Result` is honest. `h.set(A, 1)?.set(B, 2)?` already chains, so the `and_then` closures in lumos become `?`.

**Errors:**
- `MissingKeyword`, `KeywordOutOfRange` and `KeywordNotFound` carry a `Keyword`, so a caller can report a keyword that it made at run time. `MissingKeyword` also names the families that are not keywords today (`"ZFORMn"`, `"iCTYPn"`): those become the first absent member, such as `ZFORM3`.
- `InvalidHeaderValue`'s keyword becomes a `Keyword`.

**`Value` gets `From<usize>`, `From<f32>` and `From<u16>`.**

**A risk:** the HIERARCH convention (ESO) has keywords longer than 8 bytes. fits-well does not support it today. Such a card parses as a commentary `HIERARCH` card, which an 8-byte `Keyword` holds. If support is added later, `Keyword` gets a long form. The API stays the same.

### Tables

The review sections that this answers: "`read_table` on a compressed table", "One way to name a column", "Names", "Allocation per call", and part of "Typed access".

- **Compressed tables are transparent, as compressed images are.**
  - `read_table`, `read_table_rows`, `read_table_columns`, `read_table_cell` and `table_schema` give the logical table.
  - A partial read decodes only the tiles that its rows touch, and only the columns that it selects. That mirrors image sections.
  - `read_compressed_table` and `FitsError::TableIsCompressed` go away. Since phase 1, these reads refuse a compressed table, and they no longer give its container.
- **`ColumnSelector<'a> { Index(usize), Name(&'a str) }`** borrows its name, so it does not allocate.
  - `Table::column(impl Into<ColumnSelector>)` and `AsciiTable::column(…)` replace each `column_by_idx` / `column_by_name` pair.
  - `Selection::column(…) -> Option<&SelectedColumn>` is new.
  - The reader takes `columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>`.
- **`read_table_columns_into(&mut Selection)`** refills the buffers of an earlier call.
- **Names:**

  | Today | Proposed |
  |---|---|
  | `Ascii` | `AsciiTable` |
  | `Builder` | `TableBuilder` |
  | `AsciiBuilder` | `AsciiTableBuilder` |
  | `explicit` | `with_columns` |
  | `ColumnData::Bytes` | `ColumnData::U8`, as in `Data` |
  | `ColumnData::element_count` | `len` and `is_empty` |
  | `RandomGroups::group_by_idx` | `group` |

  `Table` stays the binary table, which FITS users call "the table".
- **Typed access on `ColumnData`:** `as_i64() -> Option<&[i64]>` and the other variants, and `TryFrom<ColumnData> for Vec<T>`.

### Checksums

The review section that this answers: "Verdict helpers".

- `ChecksumReport::is_valid()`: both checksums are `Valid`.
- `ChecksumReport::has_invalid()`: either one is `Invalid`.

A caller's policy for `Absent` and `Unknown` stays its own.

### WCS

The review sections that this answers: "Best-effort access", "Batch transforms" and "The `alt` argument".

- **`WcsAlt`, a checked newtype from `TryFrom<char>`, for `A` to `Z`.** `Option<WcsAlt>` replaces `Option<char>` in `Wcs`, `PhaseAxis` and `read_wcs`.
- **`pixel_to_world(&self, pixel: &[f64], world: &mut [f64]) -> Result<()>`** and the same for `world_to_pixel`. This is a better fix than a batch API. A point costs no allocation, and a caller loops over its points with one buffer.
- **lumos can use `Wcs::from_header` now** for its `CTYPEn`/`CRVALn` lookup. The parse succeeds with unsupported axes, which only the transforms refuse. fits-well needs no change. lumos checks this on its fixtures, and it keeps its hand lookup only for a header that `from_header` refuses.

## Findings not to address

| Finding | Why not |
|---|---|
| A stream writer to a sink without `Seek` | No caller. Seek is necessary only for checksums, and a type state for that puts a second type parameter on every writer method. A caller that writes to a pipe can write to a `Vec` and copy it. |
| Section stride | No caller. A preview can read full-resolution chunks and decimate them. Stride complicates the plain and the tiled section paths. |
| ASCII partial reads | No caller. ASCII tables are a legacy format, and every reader of them reads the whole table. |
| Table read → write round trip (`TableBuilder::from(&Table)`) | No caller. `read_data_raw` + `write_raw_hdu` copies an HDU verbatim. |
| Borrowed table columns | A table write transposes the columns into rows in the writer's scratch, and that copy dominates. Borrowing would add a lifetime to `TableBuilder` for a small gain. |
| One `from_unsigned` constructor | The four typed constructors are clear. Writing unsigned data converts it to signed + `BZERO` in any case, so a borrowed form would save nothing. Only their shape parameter changes. |
| A `serde` feature | lumos's copy of `SampleType` keeps lumos's file format separate from fits-well's enum, so a fits-well rename cannot break saved files. It would also add a dependency. |
| A general split of the header borrow from the data borrow | The image handle covers the read path, because `ImageReader::header` outlives the handle borrow. The other clone (`product/mod.rs:150`) copies one header for each file, which is small and not on a hot path. |
| Verify while reading a bundle | lumos verifies before it trusts a byte, on purpose. |
| A helper for the row chunks of an N-D plane | The ranges are a stack array. lumos's two copies of this code are lumos's own duplication. |
| Range-checked getters | The ranges come from the lumos domain. `require_*` covers the general need. |
| `set_with_comment` | `set(…)?.set_comment(…)` does the same, now that `set_comment` fails on a missing keyword. |
| An infallible `set` | A NaN real and non-ASCII text are invalid in FITS. |
| An `EXTNAME` argument on writes | A header with one `set` already carries it. |
| `first_image()` | This is `images().next()`. |
| `Hdu::has_pixels()` | Fixing `is_image` is the better change. |

## Plan

Each phase is one change to fits-well and its lumos migration. No phase keeps an old form: the workspace takes no backward compatibility, and lumos is the only caller.

**Verification at the end of each phase:**
- In fits-well, run `cargo drive gate`.
- From the Darkroom root, run `cargo drive check` and `cargo drive test`.
- Run `cargo drive surface` in fits-well, and review its diff as the record of the API change.
- Phases 2 and 3 also compare the fits-well benches (`integration/read_section/*`, `integration/write*`, `atomic/*`) and the lumos benches before and after. A slower case blocks the phase.

### Phase 2: the image reader

1. Add `Region`, `ImageReader`, `ImageView` (renamed from `Borrowed`) and the public sealed `PhysicalSample`.
   - Move the section, view and tiled paths of `reader/mod.rs` into the handle, which parses the geometry and the `TiledImage` one time.
2. Add `physical_into` on `ImageView`, and on the handle from the big-endian bytes.
   - `Image::physical_f64`, `Image::physical_f32` and `RandomGroups::*_physical` call it.
3. Add `with_checksum` and `finish` on the handle. `begin_data_checksum` and `finish_data_checksum` become private.
4. Add `scratch_bytes(region)` and `Hdu::padded_data_bytes()`.
5. Add the typed accessors on `View`, and `TryFrom<Data> for Vec<T>`.
6. Remove the six `read_image*` methods and `Lazy`.

**Tests**
- Port the existing section, view and checksum tests to the handle.
- `physical_into` against `physical_f64` on every `BITPIX`, with and without scaling and `BLANK`, from a plain and from a compressed image.
- `scratch_bytes` equals the capacity that a read actually uses, for a plain and for a compressed section.
- A handle that reads every row band, then calls `finish`, gives the same report as `verify_checksum`.

**lumos**
- `HduSections` goes away, and so do `ExtensionSample`, `padded_data_bytes`, and the internal sizes that `plan.rs` derives itself.
- The two chunk readers (`selected_fits.rs:314-343` and `pixels/mod.rs:264-339`) use one handle each.
- `selected_fits.rs:234` stops cloning the header.
- lumos's fused conversion (`pixels/mod.rs:423-508`) moves to `physical_into` only if its bench does not get slower. Otherwise it stays, and it reads `ImageView` through the typed accessors.

### Phase 3: writing

1. `write_image(ImageView, …)`, `write_compressed_image(ImageView, &CompressionConfig, …)` and `write_chunk(View)`. Remove `Image`-taking writes and `&Data` chunks.
2. `stream_image(Metadata, Option<&Header>)`.
3. `CompressionConfig` replaces `CompressionOptions` and the separate `Compression` argument.
4. `write_compressed_table(&Builder, &CompressionConfig, Option<&Header>)`. `Builder` is renamed in phase 5.
5. `write_empty_primary(Option<&Header>)`.
6. The shape of `Image::from_u16/u32/u64/i8` becomes `impl Into<Vec<usize>>`.

**Tests**
- A borrowed write is byte-identical to the former owned write for every `BITPIX`, plain and compressed, with and without checksums.
- A compressed table from a `Builder` decompresses to the same table that `write_table` writes.
- The table-config errors: tiles with two axes, and quantization set.
- `write_empty_primary` as the first HDU, and its error as the second.

**lumos**
- `PlaneSample::into_data/from_data` and its `unreachable!` go away. `write_chunk` takes the chunk's slice.
- The bundle primary uses `write_empty_primary`.
- `bench/files.rs:105` writes the plane without `to_vec()`.

### Phase 4: header and HDU selection

This phase changes the most call sites, about 280 in lumos, so it is one phase. It does not change data paths.

1. `Keyword`, `Keyword::indexed`, and `header::keywords` (from `reserved_keywords.rs`).
   - `Card`, the index and every `Header` method take `Keyword`.
   - `MissingKeyword`, `KeywordOutOfRange` and `InvalidHeaderValue` carry it.
2. `require_*`, `set_optional`, `set_comment` (renamed from `comment`), `entry`, `history`, `comments`, `len`, `is_empty`, `contains` and `index_of`.
   - `iter()` becomes `ExactSizeIterator`, and `IntoIterator for &Header` is new.
3. `Value: From<usize>`, `From<f32>` and `From<u16>`.
4. `HduSelector` on every reader method, `hdu`, `hdu_index -> Result<usize>`, `images()`, `Hdu::matches` and `Hdu::image_metadata`. Add `FitsError::HduNotFound`.

**Tests**
- `Keyword::new` refuses lowercase, length 9, and a character outside the set. Test this through `TryFrom`, which uses the same check.
- `Keyword::indexed(CTYPE, 2)` gives `CTYPE2`, and `(NAXIS, 1000)` gives an error.
- Each `require_*` gives absent → `MissingKeyword` and wrong type → `TypeMismatch`.
- `set_optional(None)` removes an existing card.
- `HduSelector` from index, name, `(name, version)` and `PRIMARY`, and the not-found error.
- A header-parse bench before and after, as the evidence for the allocation claim.

**lumos**
- Its keyword constants become `Keyword`.
- The `format!` keywords become `Keyword::indexed`.
- `set_optional_*` (three functions), `missing()` and the `and_then` chains go away.
- The hand-written `EXTNAME` matches use `HduSelector` (`lumos_extension.rs:194`, `calibration_bundle/mod.rs:192`, `extension_claims.rs:81,106`).
- `ISSUES.md` loses the `get` case entry.
- The README picks the first image with `images().next()`.

### Phase 5: tables and names

1. Transparent compressed tables: whole reads, partial reads by tile and by column, and `table_schema`. Remove `read_compressed_table` and `TableIsCompressed`. The test reach-ins `read_container_table` and `read_container_rows` stay for the tests of the container's own columns.
2. Borrowed `ColumnSelector`, the `column(…)` methods, `Selection::column` and `read_table_columns_into`.
3. The renames of the "Names" table above.
4. The typed accessors on `ColumnData`, and `TryFrom<ColumnData> for Vec<T>`.

**Tests**
- A partial read of a compressed table equals the same slice of its full decode, for ranges inside one tile, across tiles, and at the last partial tile.
- `read_table_columns_into` reuses its buffers: the capacity does not grow on a second call of the same size.

**lumos**
- `calibration_bundle/mod.rs:260-276` reads its columns through `as_i64` and the other accessors, with no `let … else`.

### Phase 6: the small items

1. `ChecksumReport::is_valid` and `has_invalid`.
   - lumos: `calibration_bundle/mod.rs:178` and `fits_load_config.rs:82-93`.
2. `WcsAlt`, and `pixel_to_world` / `world_to_pixel` into a caller slice.
3. lumos tries `Wcs::from_header` for `celestial_reference`, as described in "WCS" above.
4. Write `docs/design.md` with the decisions of all phases, then delete this proposal and `API_REVIEW.md`.

## Order and size

| Phase | fits-well | lumos | Depends on |
|---|---|---|---|
| 2. The image reader | large: `reader/mod.rs`, `image_model` | medium: decode | none |
| 3. Writing | medium: `fits_writer`, `compress` | small: `plane_hdu`, bundle | 2, which brings `ImageView` |
| 4. Header and HDU selection | large: `header_model`, `reader`, `error` | large: about 280 call sites | none |
| 5. Tables and names | medium: `compress/table`, `bintable`, renames | small | 4, which brings selectors |
| 6. Small items | small | small | 4 |

Phases 2 and 4 are independent, so either can go first. Phase 2 removes the most lumos code, and it is the one with a performance gain to measure.
