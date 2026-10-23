# fits-well API review

2026-10-23. Inputs:

- **The surface.** `.notes/API_SURFACE.md`, written by `cargo drive surface`.
- **The callers.** Every call site in the workspace. lumos is the only crate that uses fits-well.
- **Prior art.** The APIs of astropy.io.fits, Python fitsio, cfitsio, the Rust `fitsio`, `fitsrs` and `fitskit` crates, and Julia FITSIO.jl.

Each finding gives its evidence, then a proposal. **P1** items remove copies or caller wrappers that exist today, and they fix correctness traps. **P2** items make the API consistent. **P3** items are nice to have.

## Summary

1. **Writes must take borrowed data.** Every write path takes owned `Data` or `ColumnData`. So a caller that already holds `&[f32]` must copy it, and the stream writer forces a `Vec` for each chunk. lumos has a trait with `unreachable!` only to move buffers in and out of `Data` (P1).
2. **One image-read entry, not six.** There are six `read_image*` methods, three return types, four names for "the stored samples", and a 2 × 2 matrix of `_view` × `_summed`. lumos wraps all of them in its own section reader (P1).
3. **Select an HDU as a column is selected.** An `HduSelector` from `usize`, `&str` and `(&str, i64)`, the same as `ColumnSelector` (P2).
4. **A typed `Keyword`.** A keyword is at most 8 ASCII bytes, so it fits in a `Copy` value. That removes the `format!` for each indexed keyword, the run-time check on each `set`, and the case trap of `get` (P2).
5. **Fix four quiet traps.**
   - `read_table` on a compressed table gives the container.
   - `Header::comment` on a missing keyword does nothing.
   - `get("naxis")` gives `None`.
   - The README indexes `image_indices()[0]`, which panics on a file with no image.

   (All P1.)

## What is already right

Keep these during any redesign:

- **No HDU cursor.** HDU metadata comes through `&self`. cfitsio moves a cursor, and Rust `fitsio` threads `&mut FitsFile` through every HDU call.
- **Absent and wrong type are separate.** `Result<Option<T>>` with `TypeMismatch` tells absence from a wrong type. Rust `fitsio`'s `read_key -> Result<T>` merges the two.
- **Stored and physical values are separate, and the caller picks the precision.** astropy silently picks float32 for `BITPIX ≤ 16`.
- **Half-open, 0-based `Range<usize>` regions.** cfitsio counts from 1, and Rust `fitsio` takes `&[&Range]`.
- **A structured `FitsError`.** Rust `fitsio` has `Message(String)`.
- **Coverage.** No other Rust crate has ASCII tables, full VLA, PLIO and HCOMPRESS, built-in WCS and time, and compressed sections that decode only the tiles they touch.

## 1. Writing

### 1.1 Borrowed data on every write path (P1)

**Evidence**

- **Owned data in.**
  - `Image::new(shape, impl Into<Data>)` and `Image::new_scaled` take owned data.
  - `WriteColumn::{scalar, fixed, vla}` take owned `ColumnData`.
  - `image::Writer::write_chunk(&Data)` takes a reference to owned `Data`, so each chunk is still an owned `Vec`.
- **lumos, `plane_hdu.rs:200-202`.** It moves its chunk buffer into `Data` and back for each chunk:
  ```rust
  let data = S::Sample::into_data(mem::take(&mut chunk));
  stream.write_chunk(&data)?;
  chunk = S::Sample::from_data(data);
  ```
  `PlaneSample::from_data` is `let Data::F32(s) = data else { unreachable!() }`. It also copies each chunk from the planes into that `Vec` even when no transform applies (`:99`).
- **lumos, `bench/files.rs:105`.** `Image::new(shape, plane.pixels().to_vec())` copies a whole plane to write it.
- **Prior art.** cfitsio `fits_write_pix(void *array)`, Rust `fitsio` `write_image(&[T])` and `write_col(&[T])`, and astropy `StreamingHDU.write(data)` all take borrowed arrays.

**Proposal**

- `write_chunk(View<'_>)`. `Data::as_view` already exists, so owned callers keep working.
- Add a borrowed image for writes, `ImageRef<'a> { shape: &'a [usize], samples: View<'a>, scaling: Scaling }`. `write_image` and `write_compressed_image` take `ImageRef`, and `Image::as_ref()` gives one.
- Add a borrowed column for `WriteColumn`, a `ColumnView<'a>` with the same variants as `ColumnData`. `Builder` holds borrows for one write.

### 1.2 The stream writer does not need `Seek` without checksums (P2)

**Evidence.** `stream_image` needs `W: Write + Seek`. But it seeks only to write `CHECKSUM` again after the data (`fits_writer/writer.rs:168-181`). So a pipe or a gzip encoder cannot take a stream, even with checksums off. astropy's `StreamingHDU` writes to plain files.

**Proposal.** Make checksums a type state of the writer. `FitsWriter<W>` streams to any `Write`. `with_checksums()` gives a `FitsWriter<W, Checksummed>`, whose `stream_image` needs `Seek`.

### 1.3 One compression config, and compressed tables from a `Builder` (P2)

**Evidence**

- `write_compressed_image(&Image, Compression, &CompressionOptions, Option<&Header>)` keeps the codec outside the options.
- `write_compressed_table(&Header, &Table, rows_per_tile, Compression)` has three differences from the other writes:
  - The header comes first, and it is required. Every other write takes `Option<&Header>` last.
  - It takes the read-side `Table`. So a table made with `Builder` must go through a write and a read before it can be compressed.
  - Its tile setting, `rows_per_tile`, is a separate argument.
- **Prior art.** astropy `CompImageHDU(compression_type, tile_shape, quantize_level, …)`, fitskit `CompressOptions { algorithm, .. }` and Python fitsio `write(img, compress=, qlevel=)` keep the codec and its settings together.

**Proposal**

- Add one `CompressionConfig { codec: Compression, tiles, quantization }`, with tiles as a shape for images and as rows for tables.
- `write_compressed_image(ImageRef, &CompressionConfig, Option<&Header>)`
- `write_compressed_table(&Builder, &CompressionConfig, Option<&Header>)`

The name `CompressionOptions` also breaks the workspace rule that refuses `*Options`. A setting the caller sets is a `*Config`.

### 1.4 Structural headers (P2)

**Evidence**

- An empty primary HDU needs four hand-written cards and `write_raw_hdu(&header, &[])`. See lumos `calibration_bundle/mod.rs:133-143`, and the same code in three test files.
- To give an image an `EXTNAME`, the caller builds a `Header`. Prior art takes the name directly: astropy `ImageHDU(name=)`, Python fitsio `extname=`, Rust `fitsio` `create_image("EXTNAME", …)`.

**Proposal**

- `FitsWriter::write_empty_primary(Option<&Header>)`.
- The first image write makes the primary HDU, and each later write makes an extension, as cfitsio's `create_img` does. Then a caller never writes `SIMPLE`, `BITPIX`, `NAXIS` or `EXTEND`.

### 1.5 Unsigned constructors (P3)

**Evidence.** `Image::from_u16/u32/u64/i8(shape: Vec<usize>, &[T])` take `Vec<usize>`, but `Image::new` takes `impl Into<Vec<usize>>`. The read side gives `UnsignedData`, but the write side takes four separate constructors.

**Proposal.** `Image::from_unsigned(shape: impl Into<Vec<usize>>, UnsignedView<'_>)`, the borrowed twin of `UnsignedData`.

## 2. Reading images

### 2.1 One entry point (P1)

**Evidence**

- **Six methods:** `read_image`, `read_image_view`, `read_image_section`, `read_image_section_summed`, `read_image_section_view` and `read_image_section_view_summed`.
- **Three return types:** `Lazy`, `Borrowed` and `Image`.
- **Two names for the scratch:** it is `scratch` in one signature and `words` in another.
- **Four names for the stored samples:** `Image::stored`, `Borrowed::samples`, `Lazy::decode` and `Image::into_samples`.
- **`_summed` is a flag that doubles the methods.** Under the workspace rules, a flag that selects behavior is a type.
- **lumos wraps the matrix.** `HduSections` (`decode/hdu_sections.rs:54-67`) holds `Option<DataChecksum>` and a `Vec<u64>`, and selects between the `_view` and `_view_summed` methods. Row-chunk reads of one plane are written twice: `selected_fits.rs:314-343` and `pixels/mod.rs:264-339`. The second allocates a `vec![..]` of ranges for each chunk.
- **Prior art.** cfitsio has `read_pix` and one `read_subset`.

**Proposal.** A section reader for one HDU, which owns its scratch and an optional running checksum:

```rust
let mut image = reader.image(hdu)?;                     // header checks once
let meta = image.metadata();
image.verify(&mut sum);                                 // optional, replaces `_summed`
for rows in (0..h).step_by(CHUNK) {
    let samples: View<'_> = image.read(&[0..w, rows.clone(), plane..plane + 1])?;
}
let whole: Image = image.read_owned(Region::All)?;      // replaces read_image_section
```

`Lazy` and `Borrowed` merge into the one borrowed result `View` + `Metadata`. `Image` stays as the owned value.

### 2.2 Physical conversion into a caller buffer (P1)

**Evidence**

- `Lazy::physical`, `Lazy::physical_f32`, `Image::physical` and `Image::physical_f32` always allocate a whole image.
- `Borrowed` and `View` have no physical conversion at all.
- **lumos does it itself.** `pixels/mod.rs:423-508` writes the code for `BZERO + BSCALE · raw` and `BLANK → NaN` again for each `View` variant. The reason: it can fuse the conversion with its own pass into its own buffer.
- **Prior art.** cfitsio converts the type and the scaling into the caller's array, with `nulval`.

**Proposal.** `View::physical_into(&self, &Scaling, out: &mut [f32])` and an `f64` twin. Make it the one implementation, and let the allocating methods call it. A fused per-chunk callback can come later, if lumos shows that it needs one.

### 2.3 Typed access to `View` and `ColumnData` (P1)

**Evidence.** Callers match the enum with `let … else { unreachable!() }`:

- `selected_fits.rs:284-299`. The trait `ExtensionSample` exists only for this.
- `calibration_bundle/mod.rs:268-276`, three times.

**Proposal**

- `View::as_f32(&self) -> Option<&[f32]>` and the same for each variant.
- `TryFrom<Data> for Vec<T>`, the reverse of the `From` impls.
- The same pair for `ColumnData`.

### 2.4 Memory planning (P2)

**Evidence.** lumos `plan.rs:87-106` and `:327-335` compute fits-well's internal scratch sizes again, for a memory budget. These are the word buffer `next_multiple_of(8)`, two times the source bytes for compressed tiles, and the block padding. These numbers will drift when fits-well changes.

**Proposal**

- A method that gives the scratch bytes for a section read: `ImageReader::scratch_bytes(region)`.
- `Hdu::padded_data_bytes()`.

### 2.5 Section stride (P3)

**Evidence.** Prior art supports a step in a section: cfitsio `inc` and `[*:2]`, and astropy `Section` with steps. A decimated read is useful for previews.

**Proposal.** `Region` accepts a step. Do this only if a preview caller needs it.

## 3. HDUs

### 3.1 `HduSelector` (P2)

**Evidence**

- Every reader method takes `index: usize`. To use a name, the caller first calls `hdu_index(name, version) -> Result<Option<usize>>`.
- `hdu_index` drops the `level` argument that `Hdu::matches_extension` takes.
- lumos does not use `hdu_index`. It compares `EXTNAME` by hand in four places, and it maps "no name on the primary" to `PRIMARY` itself: `extension_claims.rs:105-111`, `lumos_extension.rs:194-196` and `calibration_bundle/mod.rs:192-199`.
- **Prior art.** astropy, Python fitsio and Rust `fitsio` all take an int or a name, and astropy and Python fitsio also take `(name, ver)`.

**Proposal**

- `HduSelector`, made from `usize`, `&str` and `(&str, i64)`, with `PRIMARY` resolved to index 0 as astropy does.
- Every `read_*` method takes `impl Into<HduSelector>`.

### 3.2 Classification (P2)

**Evidence**

- `is_image` is false for a primary HDU with `NAXIS = 0`, but it is true for an image extension with `NAXIS = 0` (`reader/hdu.rs:63-72`).
- So lumos writes `hdu.is_image() && hdu.header.naxis()? > 0` (`extension_claims.rs:89`). It also tests "plain image" as `kind != HduKind::Image` in two places.
- `Hdu::image()` gives `Metadata`, not an image.
- `image_indices()` allocates a `Vec`. The README's `image_indices()[0]` panics on a file with only tables.

**Proposal**

- `Hdu::has_pixels()` means `NAXIS > 0` for every image kind.
- Rename `Hdu::image()` to `image_metadata()`.
- `images()` gives an iterator, and `first_image() -> Option<usize>` is what astropy's `getdata` and cfitsio's `fits_open_image` do.
- Change the README so that it cannot panic.

### 3.3 Header borrows against data reads (P2)

**Evidence.** The headers live in the reader, but data reads take `&mut self`. So a caller that needs both must clone the header: lumos `selected_fits.rs:234` and `product/mod.rs:150`.

**Proposal.** Split the borrow: `reader.split() -> (&[Hdu], DataReader<'_>)`. Or let the image reader of 2.1 hold `&Hdu` beside the data source. Then the header and the reads live together.

## 4. Header

### 4.1 A typed `Keyword` (P2)

**Evidence**

- Keywords are `&str`, so the code checks each one at run time, and each `set` returns `Result`.
- Indexed keywords need a `format!` each time: lumos `metadata/mod.rs:398`, `:595`, `:658`, `:662`, `report_keywords.rs:59`, `:85` and `flags_extension.rs:133`.
- `get("naxis")` gives `None`, but `set("naxis")` refuses the keyword. Thus a lowercase lookup is quietly empty.

**Proposal**

- `Keyword`, a `Copy` newtype over `[u8; 8]`.
- `const fn Keyword::new(&str)` panics at compile time on an invalid constant.
- `Keyword::indexed(root, n) -> Result<Keyword>` makes `CTYPE2` without an allocation.
- `TryFrom<&str>` for ad hoc names.
- Header methods take `Keyword`, so `get` can no longer be called with a lowercase string. This is the workspace rule "types over strings, parsed once at the boundary".

### 4.2 Optional and required values (P2)

**Evidence**

- lumos has three identical `set_optional_*` functions, called about 25 times (`metadata/mod.rs:417-448`).
- It repeats `.get_integer(k)?.ok_or_else(|| missing(k))`.
- `FitsError::MissingKeyword { name: &'static str }` cannot carry a run-time name, so callers make their own error.

**Proposal**

- `set` accepts `Option<V>`, where `None` removes the card or does nothing.
- `require_integer` / `require_real` / `require_text` give `Result<T>`, with `MissingKeyword { keyword: Keyword }`.

### 4.3 Comments, commentary cards and collection traits (P2)

**Evidence**

- **No way to read a comment.** Julia `get_comment`, astropy `comments` and Rust `fitsio` `HeaderValue { value, comment }` all read it.
- **No value and comment in one call.** astropy `set(k, v, comment)` and cfitsio `update_key(…, comment)` set both.
- **`Header::comment(keyword, text)` on a missing keyword is a documented no-op** (`header_model/mod.rs:437`). Thus a typo is quietly lost.
- **Two methods with clashing names.** `comment` edits a card's inline comment, and `push_comment` appends a `COMMENT` card.
- **No `HISTORY` or `COMMENT` reader.** astropy's `h['HISTORY']` gives a list.
- **Missing collection methods.** `Header` has no `len`, no `contains`, no `index_of`, no `IntoIterator for &Header`, and `iter()` is not `ExactSizeIterator`. The workspace rule for a collection type asks for all of them.

**Proposal**

- `set_comment(Keyword, &str) -> Result<()>`, which fails on a missing keyword.
- `set_with_comment(Keyword, V, &str)`.
- `Entry` for reads.
- `history()` and `comments()` iterators.
- The collection methods and traits above.

### 4.4 `Value` conversions (P3)

**Evidence**

- There is no `From<usize>` or `From<f32>`. So lumos writes `f64::from(x)` and `as i64` in about ten places.
- `report_keywords.rs:95-97` goes through `i64` with `expect`, although `From<u64>` exists.

**Proposal.** Add `From<usize>`, `From<f32>` and `From<u16>`.

### 4.5 Chaining (P3)

`set` gives `Result<&mut Self>`, so `h.set(a, 1)?.set(b, 2)?` already works. lumos writes `and_then` closures in 22 places. With the `Keyword` of 4.1, a numeric or logical `set` cannot fail, so `set` can give `&mut Self`, and only text values keep a `Result`.

## 5. Tables

### 5.1 `read_table` on a compressed table (P1, correctness)

**Evidence.** `read_image` decompresses a tiled image transparently. But `read_table` on a tiled table gives the raw `BINTABLE` container, with no error. Only `read_compressed_table` decompresses it. This is quietly wrong.

**Proposal.** `read_table` decompresses as `read_image` does, and `read_compressed_table` is removed. Raw container access stays available through `read_data_raw`.

### 5.2 One way to name a column (P2)

**Evidence**

- The reader takes `&ColumnSelector` and `&[ColumnSelector]`.
- `Table` and `Ascii` have `column_by_idx` / `column_by_name` pairs.
- `Selection` has no lookup at all.

**Proposal**

- `column(impl Into<ColumnSelector>)` everywhere.
- The reader takes `impl IntoIterator<Item = impl Into<ColumnSelector>>`.

### 5.3 Names (P2)

**Evidence**

- **Binary-table types have no prefix, but ASCII types do:** `Table` / `Ascii`, `Builder` / `AsciiBuilder`, `Column` / `AsciiColumn`. `Ascii` is the only one that drops "Table".
- **One element type has two names.** `Data::U8` is the same type as `ColumnData::Bytes`.
- **Three names for a count.** `ColumnData::element_count`, `Data::len` and `AsciiColumnData::len`.
- **`Builder::explicit(nrows, columns)` does not say what is explicit.**

**Proposal**

- `BinTable`, `BinTableBuilder`, `AsciiTable` and `AsciiTableBuilder`.
- `ColumnData::U8`.
- `len` everywhere.
- `Builder::with_columns(nrows, columns)`.

### 5.4 Read and write round trip (P3)

**Evidence.** A table that the reader gives cannot be written again without rebuilding each `WriteColumn`. `Image` serves both sides.

**Proposal.** `BinTableBuilder::from(&BinTable)`, which borrows the columns once 1.1 is done.

### 5.5 ASCII partial reads (P3)

**Evidence.** Binary tables have row, column and cell reads. ASCII tables have none. cfitsio `read_col` and astropy `data['col']` serve both kinds.

**Proposal.** The same `read_table_rows/columns/cell` for ASCII, after 5.2.

### 5.6 Allocation per call (P3)

**Evidence.** `read_table_columns` makes a new `Selection`, and `read_table_cell` makes a new `ColumnData`, on each call. The cfitsio iterator works in chunks of `nPerLoop` rows.

**Proposal.** `read_table_columns_into(&mut Selection)`, which refills the buffers of an earlier call.

## 6. Checksums

### 6.1 Verdict helpers (P3)

**Evidence.** `report.datasum != Valid || report.checksum != Valid` appears in two lumos production files and two tests.

**Proposal**

- `ChecksumReport::is_valid()`
- `ChecksumReport::any_invalid()`

### 6.2 Verify while reading (P3)

**Evidence.** lumos calls `verify_checksum` on each bundle HDU, then reads the same data again (`calibration_bundle/mod.rs:175-183`). The order is deliberate: it checks before it trusts a byte. But for a small HDU, one pass could do both.

**Proposal.** After 2.1, an owned whole-HDU read can return the data with its report, and the caller then decides whether to keep the data.

## 7. WCS

### 7.1 Best-effort access (P3)

**Evidence.** lumos reads `CTYPEn` and `CRVALn` by hand (`metadata/mod.rs:655-664`) and does not use `Wcs::from_header`. A probable reason: `from_header` fails on a partial or unsupported WCS.

**Proposal.** `wcs::Axis` values from a header without a full transform: `Wcs::axes_from_header(&Header, alt)`.

### 7.2 Batch transforms (P3)

**Evidence.** `pixel_to_world(&[f64]) -> Result<Vec<f64>>` allocates for each point. astropy `all_pix2world` takes arrays.

**Proposal.** `pixels_to_world(pixels: &[f64], out: &mut [f64])` over N points, and the same for `world_to_pixel`.

### 7.3 The `alt` argument (P3)

**Evidence.** `alt: Option<char>` accepts any character, but FITS allows only `A` to `Z`.

**Proposal.** A `WcsAlt` newtype, made with `TryFrom<char>`.

## 8. Dependencies and the crate boundary

### 8.1 Other crates' types in the API (P2)

**Evidence.** `table::{BitSlice, BitVec, Msb0}` re-export `bitvec`, and `table::Complex` re-exports `num_complex`. The workspace rules refuse any re-export of another crate's items. Also, these types tie every caller to the versions that fits-well uses.

**Proposal.** Either the caller names `bitvec` and `num_complex` directly, or fits-well owns its types: a `Bits<'a>` view and its own `Complex<T>`, with a `From` in each direction behind an optional feature.

### 8.2 The `Source` trait cannot be named (P2)

**Evidence.** The surface lists `fits_well::reader::source::Source` under "public but not exported". It is in every bound of `impl<S: Source> FitsReader<S>`. So a caller cannot write `fn f<S: Source>(reader: &mut FitsReader<S>)`, and lumos is tied to `StreamReader<File>`.

**Proposal.** Export it as a sealed trait, `io::Source`.

### 8.3 `open_mmap` takes a path (P2)

**Evidence.** It is the crate's only path API. The workspace rule is that a submodule gets bytes or readers, never paths: I/O goes through `common`.

**Proposal.** `open_mmap(&File)`, so that the caller opens the file through its own I/O layer.

### 8.4 serde (P3)

**Evidence.** lumos keeps a 77-line copy of `SampleType` (`image/sample_type_record.rs`) only to serialize it.

**Proposal.** An optional `serde` feature on the plain value types: `SampleType`, `Bitpix`, `Scaling` and `ChecksumReport`. This adds a dependency, so it waits for a decision.

## 9. Errors

- **`MissingKeyword` and `KeywordOutOfRange` hold `&'static str`.** So a run-time keyword cannot be reported, and callers make their own errors. Change them to `Keyword`, with 4.1.
- **lumos maps `NotAnImage` and `Io` into its own variants.** This is a domain choice. No change is needed.

## 10. Items no caller uses

lumos uses a small part of the API. These items have no caller:

- All of `Lazy`, `read_image`, `read_image_view` and `read_image_section`.
- All of the ASCII-table, VLA, `Ragged`, bit, complex, random-groups, `wcs` and `time` API.
- Most `Header` editing and `Value` accessors.

This is not a reason to remove them, because the crate's goal is the whole FITS 4.0 standard. But it shows that the merges in 2.1 cost lumos nothing, and that the proposals for less-used areas (5.4, 5.5, 7) can wait.

## 11. Order of work

1. **Correctness traps:** 5.1, the `Header::comment` no-op, the `get` case (with 4.1 or before it), and the README panic.
2. **Borrowed writes (1.1, 1.2), then the one image reader (2.1-2.3).** Together they remove three lumos wrappers: `PlaneSample`, `HduSections` and `ExtensionSample`. They also remove the per-chunk copy and the fused re-implementation of scaling.
3. **`Keyword` (4.1-4.3) and `HduSelector` (3.1).** These change many call sites, so they go in one pass.
4. **Names and boundary:** 5.2, 5.3, 1.3 and 8.1-8.3.
5. **The P3 items, when a caller needs them.**
