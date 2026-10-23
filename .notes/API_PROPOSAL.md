# fits-well API proposal

2026-10-23. This document puts every finding of `API_REVIEW.md` and every entry of `ISSUES.md` into one design and one plan of work. The review keeps the evidence: the `file:line` references and the comparison with other libraries. Here, each item names the review section that it answers.

This proposal is temporary. When a phase lands, its decisions go into `docs/design.md`, and the items that it closes are removed from here and from `ISSUES.md`. Phase 1, the quiet traps, phase 2, the image reader, and phase 3, writing, have landed.

## Principles

- **One borrowed image type for reads and writes.** The reader gives an `image::Borrowed`, the writer takes one, and `Image::view()` makes one from owned data. Then a caller never copies data only to satisfy a signature.
- **Parse at the boundary, one time.** Keywords, HDU names and WCS alternates are types, checked when they are made. A header lookup cannot fail because of the spelling of its argument.
- **One entry point for each operation.** A region selects the part of an image, a selector selects an HDU or a column, and a config selects the compression. No suffix such as `_view` or `_summed` multiplies the methods.
- **The caller owns the large buffers. The reader owns only small staging buffers.** The documented decision of the view read stays: `ImageReader::view` fills the caller's `words`.
- **No quiet result.** An operation that cannot do what its name says returns an error. It does not return the container, do nothing, or return `None`.

## The design

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
| Section stride | No caller. A preview can read full-resolution chunks and decimate them. Stride complicates the plain and the tiled section paths. |
| ASCII partial reads | No caller. ASCII tables are a legacy format, and every reader of them reads the whole table. |
| A `serde` feature | lumos's copy of `SampleType` keeps lumos's file format separate from fits-well's enum, so a fits-well rename cannot break saved files. It would also add a dependency. |
| A general split of the header borrow from the data borrow | The image handle covers the read path, because `ImageReader::header` outlives the handle borrow. The other clone (`product/mod.rs:150`) copies one header for each file, which is small and not on a hot path. |
| Verify while reading a bundle | lumos verifies before it trusts a byte, on purpose. |
| A helper for the row chunks of an N-D plane | The ranges are a stack array. lumos's two copies of this code are lumos's own duplication. |
| Range-checked getters | The ranges come from the lumos domain. `require_*` covers the general need. |
| `set_with_comment` | `set(…)?.set_comment(…)` does the same, now that `set_comment` fails on a missing keyword. |
| An infallible `set` | A NaN real and non-ASCII text are invalid in FITS. |
| `first_image()` | This is `images().next()`. |
| `Hdu::has_pixels()` | Fixing `is_image` is the better change. |

## Plan

Each phase is one change to fits-well and its lumos migration. No phase keeps an old form: the workspace takes no backward compatibility, and lumos is the only caller.

**Verification at the end of each phase:**
- In fits-well, run `cargo drive gate`.
- From the Darkroom root, run `cargo drive check` and `cargo drive test`.
- Run `cargo drive surface` in fits-well, and review its diff as the record of the API change.

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
2. **The scratch a region read holds, for lumos's memory plan.** lumos plans its memory from an `Hdu`, before any handle exists, and its model of a compressed section (two times the stored unit) is an estimate, not a count. So the query belongs on `Hdu`, and it is added only with a measurement of what a compressed section decode holds at its peak. Until then, lumos keeps its own model, which now reads the padded size from `Hdu::padded_data_bytes`.
3. **lumos's fused conversion.** `pixels/mod.rs` converts, divides and locates nulls in one pass per chunk. It moves to `ImageReader::physical_into` and a second pass over the chunk only if a pinned bench shows no loss.
4. `WcsAlt`, and `pixel_to_world` / `world_to_pixel` into a caller slice.
5. lumos tries `Wcs::from_header` for `celestial_reference`, as described in "WCS" above.
6. Delete this proposal and `API_REVIEW.md`. Their decisions are in `docs/design.md`.

## Order and size

| Phase | fits-well | lumos | Depends on |
|---|---|---|---|
| 4. Header and HDU selection | large: `header_model`, `reader`, `error` | large: about 280 call sites | none |
| 5. Tables and names | medium: `compress/table`, `bintable`, renames | small | 4, which brings selectors |
| 6. Small items | small | small | 4 |

Phase 4 comes next, and phases 5 and 6 build on it.
