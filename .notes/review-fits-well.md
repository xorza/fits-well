# Review: fits-well

Date: 2026-10-24, at `11d4f41`. Scope: the whole crate, production code and the
test suite. Paths are relative to the crate root.

- **(checked)**: the defect was confirmed against the code. The other items come from
  one reviewer's read, so each one gets a failing test before its fix.
- **(local)**: the fix is final and does not depend on the rest of its batch, so it can
  land first.

## Part 1: Proposal

### Root causes

- **R1. Facts are checked or derived at the use site, not at the parse.**
  - `Scaling` is validated only when an `Image` is built, so `read` refuses an HDU that
    `view` and `physical_into` accept.
  - A `Tform` can hold states that the code then `unreachable!`s on.
  - `ImageDecodePlan::new` parses the compression keywords again on every read.
  - The time axis parses `CTYPE`, the scale and `CUNIT` again for each pixel.
  - WCS compares `CTYPE` text after the parse, and computes projection constants and
    pole trigonometry for each point.
  - The writer passes the opening and the fill beside a header that already decides
    them.
  - `DATASUM` is parsed in two ways.

  **Fix:** parse once into types that cannot hold a bad state, then evaluate.
- **R2. Errors are messages, not cases.** `InvalidWcs { detail }`,
  `InvalidTime { detail }`, `CorruptCompressedData { detail }`,
  `InvalidCompressionParameter { detail }`, `UnsupportedCompression { name }`,
  `TypeMismatch { name: String, expected: &str }`, `InvalidValue { card: String }`,
  `HeaderCardTooLong { keyword: String }` and `IntegerOutOfRange { value: String }`
  carry text. `DataSizeMismatch` carries elements, bytes or tile rows. A caller cannot
  tell the failures apart, and 23 tests can only match `InvalidWcs { .. }`.
  **Fix:** one variant per case, carrying the `Keyword` or record name, and the kind as
  a type.
- **R3. The `Source` abstraction is too thin.** It offers only "stage into scratch" and
  a `read_append` that zero-fills and reserves the exact size. Consequences:
  - A whole image is staged, then swapped, in two passes, through a buffer that the
    reader holds and never frees.
  - Many VLA cells grow one buffer in O(n²) time.
  - Each cell costs one seek and one system call.
  - A partial read of a compressed table reads the whole container unit.

  **Fix:** a sealed `Source` that reads into the caller's buffer, reads a set of ranges
  and merges adjacent ones, and tracks its position.
- **R4. Domain logic lives in the I/O modules.**
  - The checksum judgement and the `Selection` types live in `reader/`.
  - Table compaction lives in `DataSource`, so the reader wraps decoded bytes in a fake
    source.
  - Header rendering lives in `fits_writer`, so the editor depends on the writer.
  - The P/Q encode lives in `endian` and the decode in `bintable`, which makes a cycle.

  **Fix:** each piece moves to its domain module, and the reader and the writer only
  put the I/O in order.
- **R5. Nobody owns the HDU list or the writer's state.**
  - Free fns split the borrows (`selected_hdu`, `find_extension`, `select_rows`,
    `tiles`) and take 5 or 6 params.
  - The editor builds a whole `FitsReader` to scan the file.
  - Five sites poison the writer. `ImageWriter` writes to the sink without a check of
    the poison, which is a bug.
  - The writer stages every data unit, because the data has no type of its own.

  **Fix:** an `Hdus` type in `crate::hdu`. On the writer side, one `FitsWriter::emit`
  writes and poisons, and one `finish_hdu` takes the data unit as a typed body.
- **R6. A header record's identity is a string, and one record is defined three
  times.**
  - `Card`, `Record` and `RecordView` are the same four variants.
  - `Card::keyword() -> &str` drives the predicates, the reserved lists and the
    errors.
  - A malformed HIERARCH record has no name, so it is invisible and cannot be removed.
  - Each insert rebuilds the index (`reindex`), so `append` is O(n) and a header built
    by `append` is O(n²) (checked).

  **Fix:** a `RecordName` enum, a `Header` that stores `Record`, and an index that each
  edit maintains.
- **R7. No element-type trait.** Width, `BITPIX` code, endian conversion, null range,
  and `View` wrap and unwrap are hand `match`es in `image_model`, `bintable`, `compress`
  and the writer. Two identical conversion macros and 32 `as_*` accessors exist, and
  lumos adds two traits of its own (`PlaneSample`, `ExtensionSample`). **Fix:** grow
  `words::Sample` into one sealed sample trait.
- **R8. Tiled compression has no model for its parameters and its codecs.**
  - A tile parameter is `Option<Vec<T>>`, "a column or nothing". The convention has a
    keyword, a column, or neither (open issue: `ZSCALE` keyword).
  - The float plane is tied to the f64 arithmetic (open issue: quiet NaN).
  - Codec dispatch is written six times, across five codec enums.
  - Parallel work collects owned results, then folds them serially.

  **Fix:** `TileValue<T>`, a plane of the stored type, one `TileCodec`, and parallel
  output written in place.
- **R9. WCS native coordinates cross a lossy boundary.** θ travels in degrees and
  comes back through `asin` of a value near 1 (open issue: TAN round trip). Several
  formulas cancel (naive quadratics, differences of near-equal values). Per-axis facts
  (unit, `CTYPE`, `CRPIX`, the PV range) have several sources. **Fix:** a precomputed
  rotation, stable component forms, the colatitude in radians across the boundary, and
  one per-axis plan.
- **R10. Test helpers are gated too narrowly, scattered, and bypassed.**
  - About 12 test-only `internals` modules exist, most of them `#[cfg(test)]`.
  - The bench copies `written` twice and builds WCS headers by hand.
  - 448 `set_internal` lines build headers, and a celestial header is built in 9 ways.
  - 72 hand tolerance checks exist, about 45 with no reason, and some are looser than
    the claim that they test.

  **Fix:** one fixture tree.

### Target design

**Data**

- **Header.**
  - `RecordName { Keyword(Keyword), Hierarch(HierarchName), Commentary(CommentaryName) }`.
    `CommentaryName` is an inline 8-byte `Copy` name.
  - `Header` stores `Vec<Record>`. A malformed record keeps `Box<[u8; 80]>`,
    `Option<RecordName>` and its `Defect`.
  - The index keys on `RecordName`. `append` adds one entry, and `insert`/`remove_at`
    shift the positions after the edit.
  - The record list is private.
  - `Header::render_into(&self, Signing, spare, &mut Vec<u8>)` renders the checksum
    cards from `&self` with no clone. `Signing` is `None` or `WithDataSum(u32)`.
- **Checksum.** `checksum.rs` owns:
  - `Running { sum, partial, partial_len }` with `update(&[u8])` and `finish()`;
  - `Declared::read(header)`, the one parse of `DATASUM` and `CHECKSUM`;
  - `ChecksumStatus`, `ChecksumReport`, and one pure
    `report(declared, header_sum, data_sum)`.

  The reader, the writer and the editor all use it.
- **HDU.**
  - `crate::hdu::Hdus` owns `scan(source)`, `select(selector)`, `images()` and `find`.
  - `Hdu` moves into `crate::hdu`, beside `ImageGeometry`. It gets `extname()`,
    `extver()` and `extlevel()`, which carry the defaults.
  - `HduPosition::of_index`.
  - `hdu::Prologue { kind, bitpix, axes, pcount }` writes the structural cards for all
    six writers.
- **Source.**
  - `read_into(offset, &mut [u8])`.
  - `read_ranges(&[Range<u64>], &mut Vec<u8>)`, which merges adjacent ranges.
  - A tracked position, so that a read at the current position does not seek.
  - Growth through amortized `try_reserve`, with no zero-fill (`take(len).read_to_end`).
  - `words::bytes_mut(&mut [u64]) -> &mut [u8]`, so that a read lands in the caller's
    `words`.
- **Binary tables.**
  - `Tform { repeat, element: ElementKind, storage: Storage::{Fixed, Vla(DescriptorWidth)} }`.
  - `ColumnData::Bit(BitVec<u8, Msb0>)` and `ColumnView::Bit(&BitSlice)`.
  - A `Copy` `TableExtent` in both `Schema` and `TableView`.
  - A defect in an optional column keyword stays on its column.
  - `bintable::selection` holds the `Selection` types and the compaction, generic over
    a fetch.
  - `Ragged<V: RowValues>`, with one generic `row`, `rows` and `new`.
- **ASCII tables.** `AsciiSchema::parse(header)` and `Ascii::new(schema, data)`, as
  `Schema` and `Table` work. Both share a table-extent reader in `column.rs`.
- **Images.**
  - One sealed `Sample` trait for u8…f64: `BITPIX`, `from_be`/`to_be`, `View` wrap and
    unwrap, null range.
  - `Scaling` is resolved once at the parse: a float `BLANK` is ignored, as cfitsio
    ignores it, and an integer `BLANK` is range-checked.
  - `words::samples` is safe, with one `assert!` per call.
- **Compression.**
  - `TileValue<T> { Absent, Fixed(T), PerTile(Vec<T>) }`, read with cfitsio's
    precedence: keyword, then column, then absent.
  - `TiledImage` holds every resolved header parameter. It is built once for each image
    handle, and the plan holds only the table half.
  - The float plane is the stored type (`Wide = Self`). Dequantization computes in f64
    and stores once, as cfitsio does.
  - One `TileCodec` carries its parameters. Each codec has one encode/decode pair over
    its natural input: integers for RICE, PLIO and HCOMPRESS, bytes for GZIP and
    NOCOMPRESS. Each pair takes the tile shape and a scratch.
- **Writer.**
  - `FitsWriter::emit(&[&[u8]])` is the only write and the only place that poisons the
    writer.
  - `next_position()` contains the `ensure_writable` check.
  - `finish_hdu(header, template, Body)` classifies the header once and derives the
    opening and the fill from it. `Body` is `Bytes(&[u8])` (raw, table or compressed
    bytes) or `Samples(View)`.
  - A `Samples` body is encoded in bounded chunks. With checksums, a first pass sums the
    encoded chunks, then the header is written, then a second pass encodes and writes.
    No data unit is staged whole.
  - `AsciiWriteColumn::{text, integer, float(AsciiFloatFormat)}`, where the format is
    `F{w,d}` or `E{w,d}`.
- **Time.**
  - A `TimeAxis { scale, unit }` is resolved once and evaluated for each pixel.
  - `Datetime` has private fields and a checked constructor.
- **WCS.**
  - The rotation is precomputed with `sind`/`cosd`.
  - The native colatitude crosses into the projection in radians, or as (sin θ, cos θ).
  - One kernel per projection family, with its constants, in its own file.
  - One `AxisPlan` per axis: transform, reference, world unit, and typed `CTYPE` kind.
  - `LinearTransform` owns `CRPIX`.
  - TAB has a 1-D variant (monotonic runs, binary search, no `Vec`) and an M-D variant
    (cell extrema built at construction).
- **Errors.** One variant per case. Header value errors carry `RecordName`, column
  errors carry the column and its `ElementKind`, and stream errors carry `Bitpix`.
  These are three variants, not one shared `TypeMismatch`.

**Module layout**

- `bintable/selection.rs`: new, from `reader/mod.rs:56-126` and
  `reader/data_source.rs:116-231`.
- `checksum.rs`: absorbs `ChecksumStatus` and `ChecksumReport` (`reader/mod.rs:707-742`),
  and the running sums of `reader/data_checksum.rs` and `fits_writer/image_writer.rs`.
- `hdu/`: absorbs `reader/hdu.rs`, `reader/hdu_selector.rs`, the header scan and
  `Prologue`.
- `header_model/`: absorbs `render_header` and the signing from `fits_writer/mod.rs`.
  This cuts the edge `fits_editor` → `fits_writer` for rendering.
- `bintable/descriptor`: absorbs `validate_pq_descriptor` and `write_pq_descriptor`
  from `endian.rs`. This cuts the edge `endian.rs:7` → `bintable`.
- `world_coordinates/`: new files `angle.rs` (`sind`, `cosd`, `norm180`, `norm360`,
  tolerances), `header_read.rs` (`read_real`, `read_text`, …), `wcs.rs` (the `Wcs`
  struct) and `projection/<family>.rs`. This cuts the root ↔ child edges.
- `compress/decode/`: the cycle cuts are listed in batch 2.
- `src/internals/`: the test fixture tree (batch 1).

**Decisions to fold into `docs/design.md`** as each batch lands. Each batch has a
final item that names its decisions.

**Decided with the user on 2026-10-24.** The practice of cfitsio, astropy and wcslib is
in `docs/research/2026-10-24-reference-library-practice.md`.

- **Scaling uses `mul_add`.** No reference library gives a stable bit pattern, and one
  rounding gives the correctly rounded f64 (batch 7).
- **A bad optional column keyword fails only its column.** cfitsio and astropy ignore
  the keyword and read wrong values (batch 6).
- **`write_image` with checksums encodes in two passes.** No data unit is staged
  (batch 5).
- **An ASCII float column declares its format explicitly**: F, E or D with (w, d), and
  no default (batch 5).
- **`FitsEditor::append` refuses trailing content.** cfitsio writes over it (batch 5).

### Refused

- **No move of the HDU-layout getters** (`bitpix`, `naxis`, `axes`, …) from `Header`
  to `hdu`. The churn reaches every caller, and the gain is one dependency edge.
- **No last-hit cache for the TAB search.** `Wcs` is `&self` and shared across
  threads. A binary search over monotonic runs needs no state.
- **No perf investigation of the `e1dcdea` regression.** It moves one cycle per
  iteration of an O(K) scan, and the TAB rewrite removes the scan.
- **No `include_bytes!` fixtures under `internals`.** The package excludes
  `test_resources/`. No bench and no integration test reads a fixture (checked), so the
  loader is `#[cfg(test)]`.
- **No staged data unit in `write_image`, not even with checksums.** The samples are
  borrowed and in memory, so a second encode pass (a byte swap) is cheaper than a
  second copy of the image.
- **No interim `FitsEditor::hdu` over `selected_hdu`.** It waits for `Hdus` (batch 8),
  so that no shim lands.

### Dropped from the review

- **The split of long `push_comment`/`push_history` text, as cfitsio does it.** The
  refusal is explicit and changes nothing. A split would change the meaning of the
  caller's text with no notice.
- **An exact unsigned read when `BLANK`/`TNULL` is declared.** `view()` gives the
  stored integers and `Scaling` exactly, so an exact read exists.
- **A move of `io::RandomGroups*` to another public module.** It changes public paths,
  and no defect follows from the current place.
- **The removal of tests that overlap across layers.** A unit test and a round-trip
  test of one path catch different breaks. The cost of the overlap is small, and a
  removal risks coverage.
- **`Datetime` `Display` and an MJD → datetime conversion.** These are new features,
  not defects, and no caller asks for them.
- **One rounding for the f32 physical value.** Exact rounding needs more than f64
  arithmetic, so the fix is to correct the doc claim (batch 7).
- **One format of each real between `set` and write.** Header edits are cold code.
- **The full keyword method set for HIERARCH.** Only `remove_hierarch` and the comment
  read are kept, because those are the gaps that block a caller.
- **The drift between `examples/emit_compressed.rs` and `compress::internals`.** The
  example uses only the public API, which is its purpose.

## Part 2: Plan

The batches are sorted by impact, after batch 1. Each batch leaves `cargo drive gate`
green. Batch 1 comes first because every later batch adds its regression tests through
the batch 1 API. When an item fixes an entry of `.notes/ISSUES.md`, delete that entry
too.

Order and needs:

1. Test fixture tree.
2. Tiled-compression parameters and the float plane.
3. Header records.
4. Checksum ownership.
5. Writer and editor. Needs 3 and 4.
6. Binary and ASCII table model.
7. Image model and the sample trait. Needs 6.
8. Reader I/O and the HDU list. Needs 4, 6 and 7 (`words::bytes_mut`).
9. Time coordinates.
10. WCS precision.
11. WCS structure. Needs 10.
12. Compression codecs and parallel output. Needs 2.
13. Test suite migration and quality. Needs 1.

### Batch 1: Test fixture tree

Files: `src/lib.rs`, new `src/internals/{fixture.rs, memory.rs, cards.rs, samples.rs,
rng.rs, tolerance.rs, wcs_header.rs, container.rs, scene.rs}`, `src/fits_writer/mod.rs`,
`src/header_model/mod.rs`, `src/reader/mod.rs`, `src/bench/{mod.rs, scene.rs}`.

- [ ] **`src/lib.rs:219`: create the tree.** Defect: test helpers live in about 12
  test-only `internals` modules, mostly `#[cfg(test)]`, so benches and `tests/` cannot
  reach them. Target:
  - The inline `pub mod internals { … }` in `lib.rs` keeps its `pub use` of the TAB
    entry points. Its new child modules resolve to `src/internals/*.rs`.
  - The tree compiles under `internals`, with one
    `#![cfg_attr(not(test), expect(dead_code, reason = "a bench or the allocation test uses part of the fixtures"))]`.
  - Helpers that read private fields stay beside their type: `read_container_table`,
    `data_source::internals::scratch`, `CountingSource` and its `counted(bytes)`,
    `set_scaling`, `set_column_kind`, `Table::from_data`, `uncompress_table`,
    `ProjectionParameters::raw`, `linear::matrix`.
- [ ] **`internals/fixture.rs` (`#[cfg(test)]`).** Defect: fixtures are found in two
  ways. `reader/mod.rs:888` uses a working-directory path, and `hcompress.rs:1604` uses
  `include_bytes!`. `UITfuv2582gc.fits` is read from disk 12 times. Three tests read
  one file twice (`checksum.rs:139-140`, `reader/tests.rs:239/247`, `:296/311`,
  `:684/686`). Target:
  - `enum Fixture`, with one variant for each file in `test_resources/`.
  - `const fn bytes(self) -> &'static [u8]`, through
    `include_bytes!(concat!(env!("CARGO_MANIFEST_DIR"), …))`.
  - `stream()`, `slice()`, `header()` (parsed once through `OnceLock`) and `wcs()`.
  - It replaces `reader::internals::{fixture_path, fixture_bytes, open_fixture}`, both
    `fixture_header` copies (`world_coordinates/distortion/tests.rs:15`,
    `world_coordinates/tests.rs:558`), `open_wcs` (`world_coordinates/tests.rs:21`) and
    `TAN_FIXTURE` (`table_wcs/tests.rs:156`).
- [ ] **`internals/memory.rs`.** Defect: `fits_writer::internals::{written, round_trip, rejected_before_output}`
  (`fits_writer/mod.rs:505`) is `#[cfg(test)]`, and it always builds a fresh writer.
  So the 17 `.with_checksums()` sites write the setup inline, and the bench copies it
  twice (`bench/scene.rs:151`, `:189`). 56 `FitsWriter::new(Cursor…)` and 52
  `into_inner().into_inner()` sites remain. Target:
  - `MemoryWriter`, `memory_writer()`, `written(f)`, `written_by(w, f)`, `round_trip(f)`,
    `round_trip_by(w, f)`, `rejected_before_output(f)`, `read_whole`, and
    `reader_over(header, data)`.
  - Delete `fits_writer::internals`.
- [ ] **`internals/cards.rs`.** Defects:
  - Rendered bytes are split into 80-column lines in 5 places.
  - A card is looked up in raw bytes 4 times (`fits_writer/tests.rs:187, 1347, 1387,
    1477`).
  - Five builders repeat the structural block: `bintable/mod.rs:100`,
    `ascii/mod.rs:444`, `groups/tests.rs:6`, `decode/tests.rs:36` and
    `tabular/mod.rs:924`.
  - `Header::parse(&records(..))` is written 8 times, beside the unused
    `from_card_lines`.

  Target: `card_bytes`, `records` and `from_card_lines` (from `header_model::internals`),
  `record_lines`, `find_card`, `fits_file` (from `reader/tests.rs:369`), `structural`,
  `header_of(&[(&str, Value)])`, `assert_cards(h, &[(&str, Value)])` and
  `const VALID: ChecksumReport`.
- [ ] **`internals/samples.rs`, `rng.rs` and `scene.rs`.** Defects:
  - Six per-`Bitpix` formulas are written twice (`reader/tests.rs:1092-1144`,
    `:1277-1308`).
  - The `Bitpix` list is written twice (`bitpix/tests.rs:5`, `bench/scene.rs:27`).
  - The NaN-aware bit comparison is written 8 times.
  - SplitMix is written 3 times, and xorshift twice.
  - The plane `(7919·i+13) mod 2001 − 1000` is written twice.

  Target: `BITPIXES`, `ramp(bitpix, count)`, `gather`, `integer_range(bitpix)`,
  `NONFINITE`, `assert_same_bits`, `nan_blind_bits`, `every_column_kind(vla)`, one
  `SplitMix`, and `compress::internals::odd_plane`. `bench/scene.rs` moves to
  `internals/scene.rs`, and the bench drivers import it from there.
- [ ] **`internals/tolerance.rs`.** Defect: 72 hand `(a-b).abs() <` checks, about 45 of
  them with no reason. Target:
  - `Tolerance { bound: Exact | Abs | Rel | Ulps, why: &'static str }`.
  - `#[track_caller] assert_close(got, want, tol, ctx)`.
  - Named constants, each with a `///` reason.
  - `max_abs_diff` replaces `distance` (`distortion/tests.rs:44`) and `world_distance`
    (`world_coordinates/tests.rs:669`).
- [ ] **`internals/wcs_header.rs`.** Defects:
  - A celestial header is built in 9 ways (`world_coordinates/mod.rs:968`,
    `world_coordinates/tests.rs:274`, `table_wcs/tests.rs:26-153`,
    `projection/tests.rs:261, 304, 416`, `celestial_frame/tests.rs:10`,
    `linear_transform/tests.rs:84`, `axis_transform/tests.rs:73`).
  - `bench.rs::Fixture::linear` and `tests/wcs_allocations.rs:67-100` build headers by
    hand.

  Target:
  - `WcsHeader::{celestial, pair, axis}`, with chainable `.pc`, `.cd`, `.crota`, `.pv`,
    `.ps`, `.poles`, `.sip`, `.keyword`, `.family` and `.alternate`, then `.header()` or
    `.wcs()`.
  - `assert_round_trip(wcs, points, Tolerance, ctx)` and `assert_same_world`.
  - Keep the bit-exact `assert_same_transform` (`world_coordinates/tests.rs:454`).
    Rename the 1e-12 copy in `table_wcs/tests.rs:189`.
- [ ] **`internals/container.rs` (`#[cfg(all(test, feature = "compression"))]`).**
  Defect: the one-tile containers are built by hand, with a hand-counted `NAXIS1`
  (`decode/tests.rs:469-491`: `28, // 1P(8) + 1D + 1D + 1J`) and 46 `to_be_bytes` pushes
  over 11 sites. Target: `OneTile::new(cmptype, zbitpix, shape).heap(..).fixed(..).build()`,
  which computes `TFORMn`, `NAXIS1`, `PCOUNT` and the descriptors.

### Batch 2: Tiled-compression parameters and the float plane

Files: `src/compress/decode/{float_quantization.rs, null_mask.rs, decode_sample.rs,
wide_plane.rs, tile_decoder.rs, tile_sources.rs, tiled_image.rs, decode_buffer.rs,
image_decode_plan.rs, tile_scratch_set.rs}`, `src/compress/{convert.rs, hcompress.rs,
quantize.rs}`, `src/endian.rs`, `src/reader/image_reader.rs`, `docs/design.md`.

- [ ] **(checked) `float_quantization.rs:63-67, 119-127`: `ZSCALE`/`ZZERO` keywords are
  ignored** (ISSUES.md).
  - With one keyword and one column, the image decodes with a scale of 1.
  - With both as keywords, the image reads as lossless, so an f32 GZIP image reads its
    quantized integers as float bits.

  Target:
  - `TileValue<T> { Absent, Fixed(T), PerTile(Vec<T>) }`, with
    `read(header, table, name)` in cfitsio's precedence (keyword, then column, then
    absent) and `at(row) -> Option<T>`.
  - An image is lossless when `ZQUANTIZ = 'NONE'`, or when both `ZSCALE` and `ZZERO`
    resolve to `Absent`.
  - `read_f64_column` (`:144-150`) widens any numeric `TFORM`, as `read_i64_column`
    does and as cfitsio reads with `TDOUBLE`.
- [ ] **(checked) `null_mask.rs:53`: an integer image never reads `ZBLANK`.** Nulls
  stored under `ZBLANK` with no `BLANK` read as data, and a null mask fails with
  `MissingKeyword(BLANK)`. Target: `ZBLANK` resolves through `TileValue` (keyword, then
  column), then falls back to `BLANK`, as cfitsio does. The float path
  (`float_quantization.rs:125`, column first today) uses the same order.
- [ ] **`float_quantization.rs:75-80`: `ZDITHER0` outside 1..=10000 is refused even for
  `NO_DITHER`, which never reads it.** Target: check the range only for a dither
  method that reads it.
- [ ] **`decode_buffer.rs:59, 80`: `ImageDecodePlan::new` parses the header parameters
  on every read.** These are the `CodecParams` scan, `ZQUANTIZ`, `ZDITHER0`, `ZBLANK`,
  `ZMASKCMP` and the PLIO offset. A bad keyword fails a read instead of `image()`.
  Target: `TiledImage` (built at `reader/image_reader.rs:91`) holds the resolved header
  half, and the plan holds only the table half.
- [ ] **(checked) `decode_sample.rs:70-79`, `wide_plane.rs:45-71`,
  `convert.rs:129-136, 152-169`: a signaling NaN comes back quiet** (ISSUES.md). The f64
  plane also doubles the memory of each f32 tile and adds a narrowing pass. Target:
  - `impl DecodeSample for f32 { type Wide = f32 }`, and one
    `impl<F: FloatSample> WidePlane for F`.
  - `FloatSample` gives `from_be_bytes` (a bit copy), `from_f64` (one rounding) and
    `NAN`.
  - The lossless and fallback paths decode straight into `F`.
  - Dequantization computes in f64 and stores `F::from_f64`.
  - The integer plane stays `i64`, because overshoot and clamping need it.
- [ ] **(checked, local) `hcompress.rs:962`: the decoder checks only `nx·ny`.** A
  stream with swapped dimensions decodes transposed. Target: the decoder takes the tile
  shape and checks `(nx, ny)` against it, as the encoder takes `tdims` (`:64`).
- [ ] **(checked) `tile_decoder.rs:129-138`: `Overshoot` follows the image codec, not
  the tile source.** GZIP or uncompressed fallback cells in an HCOMPRESS image are
  clamped with no error. Target: the tile source returns its overshoot, and only an
  HCOMPRESS primary cell gets `Clamp`.
- [ ] **`convert.rs:152-169`, `endian.rs:74`: cell type and cell length.** An `E` cell in
  a `-64` image converts with no error. A byte cell of 4n+1 bytes loses its tail in
  `as_chunks`. Target:
  - At the boundary, the cell type must match `ZBITPIX`, and the length must be exactly
    the tile's element count times the element size.
  - `decode_be_into` gets a `debug_assert!` that its input is a whole number of
    elements.
- [ ] **`tile_sources.rs:28-40`: each tile decodes the descriptors of all three data
  columns.** A broken descriptor in an unused fallback column fails a good tile, where
  cfitsio reads a fallback only when the primary cell is empty. Target: a lazy
  `TileSources::source(row)`. Delete `TileCells`.
- [ ] **`quantize.rs:90-115`, `tile_decoder.rs:184-186`, `quantize.rs:408-420`.**
  `dequantize_into` takes 7 params, and its caller unpacks the `Dequant` struct that
  already holds them. Each sample tests four branches that are fixed for the whole
  tile. Target: `Dequant::apply(&self, ints, out)` beside `Dequant`, with one loop per
  dither method and null mode. Apply the same shape to `Quantizer::quantize`.
- [ ] **Cut the `decode/` cycles.** These are:
  - `image_decode_plan` ↔ `tile_scratch_set` (`tile_scratch_set.rs:4`);
  - `wide_plane.rs:3` → `image_decode_plan` → `decode_sample.rs:3`;
  - `tiled_image.rs:9` ↔ `decode_buffer.rs:7`;
  - `null_mask.rs:7-8` and `tile_decoder.rs:12-13` → `tiled_image`.

  Target:
  - `WidePlane::decode_tile` and `TileScratchSet::decode` take the decoder, the tile
    source and the dequantization, not `&ImageDecodePlan`.
  - `CodecScratch` moves to `tile_decoder.rs`.
  - `NullMask::read` takes `blank`.
  - `DecodeBuffer` takes a built plan.
- [ ] **`docs/design.md`, Compression.**
  - The first bullet changes from "no `ZSCALE` and no `ZZERO` column" to "neither as a
    keyword nor as a column".
  - New decision: a tile parameter resolves as keyword, then column, then absent, as
    cfitsio's `imcomp_get_compressed_image_par` does.
  - New decision: a float tile decodes in its stored type.

### Batch 3: Header records

Files: `src/header_model/{mod.rs, card/mod.rs, value.rs, repair.rs, defect.rs,
hierarch_name.rs}`, `src/keyword.rs`, `src/reserved_keywords.rs`, `src/error.rs`,
`src/fits_writer/mod.rs`, `src/fits_editor/mod.rs`, `src/reader/mod.rs`,
`src/compress/mod.rs`, `src/world_coordinates/declared.rs`, `docs/design.md`.

- [ ] **(checked, local) `card/mod.rs:867-875`: the null string becomes one blank.**
  `Value::Text("")` is padded to `'        '`, which `parse_string` (`:488-497`) reads
  as `" "`. Every header that the crate copies turns `KEY = ''` into `' '`. cfitsio
  `ffs2c` pads `""` the same way, and astropy writes `''`. Target:
  - Write `''` with no padding for the empty string, as astropy does.
  - `From<&str>` and `From<String>` (`value.rs:279-289`) strip the trailing blanks, as
    the `Value::Text` doc says. An all-blank string keeps one blank.
  - Add the `""` case to `card/tests.rs:568`.
- [ ] **`card/mod.rs:141-151`, `mod.rs:209, 735`: a malformed HIERARCH record is
  invisible.** `get_hierarch` returns `Ok(None)`. `set_hierarch` appends a second record,
  and nothing can remove the bad one. Target:
  - Add `RecordName`, with `CommentaryName` (inline, `Copy`), which replaces the
    `String` that each COMMENT and HISTORY name allocates today (`card/mod.rs:103`).
  - `Malformed` carries `Option<RecordName>`.
  - The index holds the malformed HIERARCH records, and `set_hierarch` replaces them.
- [ ] **`card/mod.rs:95-97, 568-570`, `reader/mod.rs:834`, `fits_editor/mod.rs:224`: an
  `END` followed by stray bytes makes the file unreadable.** The block scan uses the
  same strict `is_end_record`. cfitsio compares only the first 8 bytes, and astropy
  accepts the record with a warning. Target: a new `RepairKind` that accepts the record
  as `END` and reports it. The scan and the parse share the one test.
- [ ] **`mod.rs:894-904`: an orphan `CONTINUE` is rebuilt with `format!`, not kept.** A
  full record then grows past 72 bytes of text and fails to write with
  `HeaderCardTooLong`. Target: `ParsedRecord::Continue` carries the raw tail, and the
  record becomes `free_text(&text[8..])`.
- [ ] **`card/mod.rs:24-52`, `mod.rs:60-167, 630-662`: one record kind is defined three
  times.** Target: `Header` stores `Record`. `RecordView` borrows it, one name method
  exists, and `Keyword` passes by value.
- [ ] **(checked) `mod.rs:594-606, 620-629, 728-739`: each insert rebuilds the index.**
  It also clones each `HierarchName`. `index_of`, `contains` and `remove`
  (`:389-398, 609-617`) scan with string compares. Target: each edit maintains the
  index, lookups use it, and the HIERARCH index holds positions only.
- [ ] **`reserved_keywords.rs:10-59`, `mod.rs:679-706`, `keyword.rs:19`,
  `compress/mod.rs:280`, `world_coordinates/declared.rs:111, 191-217`: record names
  travel as `&str`.** Target: predicates take `RecordName`, and the reserved lists are
  `[Keyword; N]`. `Keyword::index_after(root) -> Option<usize>` replaces
  `keyword::index`.
- [ ] **`mod.rs:43`, `fits_writer/mod.rs:42-64, 355-387, 484-492`: rendering lives in
  the writer and reads the `pub(crate) cards` field.**
  - `prepare_header_with_data_sum` clones the header to sign it (`mod.rs:191`,
    `image_writer.rs:91, 175`, and twice for each editor `replace_header`).
  - `patch_checksum` then searches the bytes again.

  Target:
  - The field is private.
  - `Header::render_into(&self, Signing, spare, out)` patches the checksum cards at
    their known position.
  - `Header::record_count()` lets the editor size the spare records with one render.
- [ ] **`mod.rs:743-790`: HIERARCH records have no remove and no comment read.** A
  parsed HIERARCH comment is visible only through `iter`. Target: `remove_hierarch` and
  `get_hierarch_comment`. `comments()` (COMMENT records) becomes `commentary()`, so that
  "comment" keeps one meaning beside `get_comment`/`set_comment`.
- [ ] **`mod.rs:620, 832-855`: `remove_at` has no inverse.** Target:
  `insert_record(index, Record)` and `Extend<Record>`.
- [ ] **`error.rs`: the header errors carry strings.** Target:
  - `ValueTypeMismatch { name: RecordName, expected: ValueKind }`.
  - `InvalidValue { name: Option<RecordName>, defect: Defect }`.
  - `InvalidAscii` carries the record name.
  - `HeaderCardTooLong { name: RecordName, length }`.
  - `IntegerOutOfRange` carries the keyword and a target enum.
  - `InvalidHeaderValue.reason` becomes an enum.
- [ ] **`docs/design.md`.**
  - Header: a record's name is a `RecordName`.
  - Header: add the `END` repair to the repair bullet.
  - New "Errors" section: one variant per case, carrying the name and the kind as
    types. Refused: a `detail: String`.

### Batch 4: Checksum ownership

Files: `src/checksum.rs`, `src/reader/{data_checksum.rs, mod.rs}`,
`src/fits_writer/image_writer.rs`, `src/fits_editor/mod.rs`, `docs/design.md`.

- [ ] **`reader/data_checksum.rs:278-284` vs `fits_editor/mod.rs:161-186`: `DATASUM` is
  parsed in two ways.**

  | `DATASUM` value | Reader | Editor |
  |---|---|---|
  | `''` | `Invalid` | unknown, and recomputes |
  | malformed | `Invalid` | `KeywordOutOfRange` error |

  The standard (§4.4.2.7) makes a blank value unknown, and cfitsio `ffvcks` also reads
  `''` as unknown. So the reader's `Invalid` for `''` is a bug
  (`data_checksum.rs:156-158` requires at least one blank). Target:
  - `checksum::Declared::read(header)` and one pure `report`, used by both callers.
  - An empty or all-blank value is `Unknown`. A value that is not a number is
    `Invalid`.
  - When the declared `DATASUM` is unknown or invalid, the editor recomputes it from
    the data.
- [ ] **`fits_writer/image_writer.rs:37-42, 200-222` vs `reader/data_checksum.rs:26-28,
  102-127`: the running sum is written twice.** Target: `checksum::Running`.
  `DataChecksum` keeps only its offset bookkeeping on top of it.
- [ ] **`fits_editor/mod.rs:183-185`: `data_sum` copies the whole data unit into a `Vec`
  to sum it.** Target: sum in bounded chunks through `Running`, as
  `DataChecksum::finish` already does with `CHUNK`.
- [ ] **`reader/mod.rs:707-742`: `ChecksumStatus` and `ChecksumReport` live in the
  reader.** Target: move them to `checksum.rs`.
- [ ] **`checksum.rs:13`, `data_checksum.rs:274`: release asserts on internal
  contracts.** `checksum.rs:13` runs on each 2880-byte block. Target: `debug_assert!`.
- [ ] **`docs/design.md`, Reading:** the checksum decision names `checksum.rs` as the
  one parse and the one sum.

### Batch 5: Writer and editor

Files: `src/fits_writer/{mod.rs, image_writer.rs, builder.rs, ascii_builder.rs}`,
`src/fits_editor/mod.rs`, `src/hdu/mod.rs`, `src/block/mod.rs`,
`src/compress/{encode/mod.rs, table/mod.rs}`, `src/error.rs`, `docs/design.md`.

- [ ] **(checked) `image_writer.rs:120-189`: the stream continues after an I/O
  failure.** `write_chunk` sets `Failed` but never checks it. A retried chunk lands
  after the partial bytes, and `finish` signs the result and returns `Ok`. Target:
  - `FitsWriter::emit(&[&[u8]])` is the only write and the only poison site, and it
    refuses on a poisoned writer.
  - `next_position()` contains `ensure_writable`, which removes the `unreachable!`
    (`mod.rs:195-208, 389-401`).
  - The child modules stop touching `sink`, `state`, `scratch` and `form`.
- [ ] **`mod.rs:117-123, 166-191, 219-247`, `image_writer.rs:55-63`: each data unit is
  staged, and the opening and the fill are passed beside the header.**
  - `write_raw_hdu` copies the caller's `raw` into `scratch` only to pad it.
  - `write_image` stages the whole encoded image, 96 MB extra for a 24 MP f32 frame.
  - A raw `XTENSION` header written first fails with `MissingKeyword(SIMPLE)`, where
    `write_table` writes the empty primary. lumos `rewrite_fits`
    (`lumos/src/internals/fits.rs:33-40`) fails so when its edit drops HDU 0.

  Target: `finish_hdu(header, template, Body)` (see Target design).
  - It classifies the header once. An extension at the primary position gets the empty
    primary, and the fill comes from the kind.
  - A `Bytes` body is summed, written, then followed by the fill from a constant block.
  - A `Samples` body is encoded in chunks of `scratch`. With checksums (decided), a
    first pass encodes and sums the chunks, then the header is written, then a second
    pass encodes and writes. Without checksums, there is one pass.
- [ ] **(checked, local) `fits_editor/mod.rs:130-136`: `append` writes HDUs that no
  reader sees.** A file whose last HDU is followed by special records or zero blocks
  passes the `len % 2880` check, and the reader's scan stops at that block
  (`reader/mod.rs:811-815`). Target (decided): the last HDU's end must equal the file
  length. If it does not, `append` refuses with a new
  `TrailingContent { hdu_end, file_len }` error, and nothing is written. cfitsio writes
  over the trailing content, which this refusal avoids.
- [ ] **(checked, local) `fits_editor/mod.rs:140-156`: `replace_header` accepts a change
  of HDU kind.** Only the data size is compared, so a `TABLE` ↔ `BINTABLE` change keeps
  fill bytes of the wrong kind (§7.2.3, §3.3.2). Target: compare `HduKind::classify` of
  the old and the new header, and refuse a change.
- [ ] **(checked) `ascii_builder.rs:36-47, 254-273, 332-337`: ASCII float format.**
  - `decimals` defaults to 0, and with `d = 0` the cell has no decimal point. §7.2.5
    says that a real field should have one.
  - Only `Fw.d` exists, so 1e-30 under `F20.6` is written as `0.000000`.
  - `with_decimals` is ignored on text and integer columns.

  Target (decided): `AsciiWriteColumn::{text, integer, float(name, data, AsciiFloatFormat)}`.
  - `AsciiFloatFormat` is `F`, `E` or `D` with (w, d), and has no default.
  - Each cell has a decimal point, and the format is the `TFORMn` that the header
    declares.
  - A value is rounded to `d` digits, as the caller declared.
  - A value that is too wide for `w` is refused, as it is today.
- [ ] **Six structural prologues:** `mod.rs:405-417`, `image_writer.rs:238-264`,
  `builder.rs:585-603`, `ascii_builder.rs:292-310`, `compress/encode/mod.rs:216-228`
  and `compress/table/mod.rs:385-392`. Their comments differ, and one site spells
  `NAXISn` through `format_args!` (`image_writer.rs:281`). Target:
  `hdu::Prologue::write_into(&mut Header)`.
- [ ] **`builder.rs:491-496`, `compress/table/mod.rs:240-249, 378-383`: the
  compressed-table write renders `TableLayout` into a header, then parses it back.**
  Target: `compress_table(schema: &Schema, preserved: &Header, …)`, with the `Schema`
  built from `TableLayout`. `finish_hdu`'s doc then states the one merge rule.
- [ ] **`builder.rs:429-438, 550, 849-875, 889-911`: per-cell dispatch in the
  binary-table encode.** Each cell passes two `match`es and a resize. The `Bits` mask
  (`:899-905`) is dead, and `Bits` packs one row at a time. Target:
  - Size `out` to `main_len` once.
  - Scatter column by column at the stride `row_len`.
  - Delete the mask.
  - Copy the `Bits` range with one `extend_from_slice`.
- [ ] **`ascii_builder.rs:230, 234, 272`: one `String` for each numeric cell.** Target:
  `write!` into one reused scratch.
- [ ] **`builder.rs:229-239`: `with_wide_descriptors` panics on two layouts.** Target:
  the descriptor width is an argument of `vla` and `vla_bits`.
- [ ] **`builder.rs:527-532`: `BuilderCells::new` allocates `row_count × vla_columns`
  offsets for a compressed table.** Target: offsets for one tile at a time.
- [ ] **`fits_editor/mod.rs:108`: `replace_header_into` renders with 0 spare records.**
  The space that `with_spare_records` reserved is lost. Target: keep the spare count.
- [ ] **Coding-rule items.**
  - Wide signatures: `write_vla_descriptor`, `ascii_table_header` and
    `validate_scaling`. Write `bintable_header` inline in `Builder::header`.
  - `ColumnType` (`builder.rs:37-49, 652-696`) becomes `ColumnData::tform_kind()`.
  - The `AsciiWriteColumn` and `AsciiBuilder` fields become private.
  - `stream_template` validates the scaling twice (`image_writer.rs:84`).
  - `RECORDS_PER_BLOCK` (`fits_editor/mod.rs:190`) moves to `block`.
  - `HduPosition::of_index` replaces the derivations at `fits_editor/mod.rs:142`,
    `reader/hdu.rs:66` and `reader/mod.rs:249`.
  - The stream chunk mismatch (`image_writer.rs:121-126`) becomes
    `ChunkTypeMismatch { expected: Bitpix, got: Bitpix }`.
- [ ] **`docs/design.md`, Writing.**
  - New decision: a data unit is written from a typed body and never staged. With
    checksums, the samples are encoded twice. Refused: a whole-unit copy.
  - New decision: an ASCII real column declares `Fw.d`, `Ew.d` or `Dw.d`, with no
    default, and every cell has a decimal point. Refused: astropy's default widths.
  - New decision: `append` refuses trailing content.

### Batch 6: Binary and ASCII table model

Files: `src/bintable/{tform/mod.rs, tform_kind.rs, descriptor/mod.rs, schema/mod.rs,
table_view.rs, column.rs, column_data.rs, column_view.rs, column_reader/mod.rs,
bit_column/mod.rs, vla_column.rs}`, `src/ragged.rs`, `src/ascii/{mod.rs, ascii_text.rs}`,
`src/column.rs`, `src/endian.rs`, `src/hdu/mod.rs`, `src/reader/hdu.rs`,
`src/fits_writer/builder.rs`, `docs/design.md`.

- [ ] **(checked, local) `tform/mod.rs:55-82`: TFORM trailing characters are refused.**
  §7.3.1 makes the trailing characters *a* optional and undefined, and §7.3.5 allows
  more characters after `emax`. A file with `'60A10'` or `'1PE(5)…'` fails
  `Schema::parse`, and every column of the HDU is lost. Target:
  - Ignore anything after the code.
  - For P/Q, take `t` and an optional `(digits)`, then ignore the rest.
  - `tform/tests.rs:38-39` become accepted cases.
- [ ] **(checked, local) `descriptor/mod.rs:86-89`: a zero-count descriptor with a
  negative offset is refused.** §7.3.5 says that the offset is then undefined. Target:
  check the offset only when the count is above 0.
- [ ] **(checked) `column_reader/mod.rs:257-263`, `column.rs:148-158`,
  `tform_kind.rs:142`: a `PX`/`QX` row loses its bit count.** `X` decodes to
  `ColumnData::U8`, and `vla_ends` counts a `PX` row in bytes. So rows of 3 bits and of
  8 bits both come back as 1 byte, and `ColumnData::U8` stands for both `B` and `X`.
  Target: `ColumnData::Bit(BitVec<u8, Msb0>)` and `ColumnView::Bit(&BitSlice)`, so that
  `X` and `PX` decode to bits and a row ends at its descriptor count. The dead
  descriptor arm of `heap_kind` (`tform_kind.rs:184`) goes away with `ElementKind`.
- [ ] **`schema/mod.rs:58-97`: one bad optional column keyword fails the whole table**
  (`TDIMn`, `TSCALn`, `TZEROn`, `TNULLn`, `TDISPn`). cfitsio (`ffgtbp`) and astropy
  ignore such a keyword, so the column reads with wrong values. Target (decided):
  - The defect stays on its column.
  - A read of that column's values fails with an error that names the keyword, when
    the keyword changes the values (`TSCALn`, `TZEROn`, `TNULLn`, `TDIMn`).
  - A bad `TDISPn` fails only `tdisp()`, because it changes no value.
  - The other columns read normally.
- [ ] **`tform_kind.rs:17-44`, `tform/mod.rs:12-18`: `TformKind` mixes the element type
  with the storage.** This causes:
  - an `unreachable!` at `tform_kind.rs:176`, `column.rs:128` and `builder.rs:824`;
  - an `expect` at `column.rs:153`;
  - a panic in `DescriptorWidth::of`;
  - the paired matches at `column_reader/mod.rs:206, 226`.

  Target: `Tform { repeat, element: ElementKind, storage: Storage }`, with private
  fields.
- [ ] **(checked) `endian.rs:7` ↔ `tform_kind.rs:8`: the P/Q encode lives in `endian`.**
  `write_pq_descriptor` converts each value twice and runs a release `assert_eq!` for
  each VLA cell (`endian.rs:141`). Target: `PqDescriptor::{validate, write}` in
  `bintable/descriptor`, with a `debug_assert!`.
- [ ] **`schema/mod.rs:23-32, 154-167`, `table_view.rs:22-30, 54-73`: the table geometry
  is held twice.** `Schema::compacted` also clones every `Column` on each
  `read_table_rows`. Target: a `Copy` `TableExtent` in both, with the compaction on the
  extent.
- [ ] **`column_reader/mod.rs:142-147, 176-180, 216-220, 256-291`: each VLA row is
  validated twice.** `vla_unsigned` resolves the column twice, and `vla_bits` writes
  `VlaColumn::cell` again. Target: one fallible pass that pushes each end and appends
  the row's values.
- [ ] **`ragged.rs:9-13, 75-214, 235-240`: `Ragged` is specialized for each value type
  inside `ragged.rs`.** This makes the cycle `ragged` ↔ `bintable`, and
  `Ragged<UnsignedData>` has no `row` or `rows`, against design.md. Target:
  - `impl<V: RowValues> Ragged<V> { new, row, rows }`, with each value type
    implementing `RowValues` in its own module.
  - Move the `AsciiText` test (`ragged.rs:248`) to `ascii`.
  - Delete the public `Ragged<String>` impls (`:105-143`), which have no public
    producer.
- [ ] **`bit_column/mod.rs:96`: `BitColumn::iter` borrows `&self` of a `Copy` type.** So
  `col.bits()?.iter()` does not compile. No row collection implements `IntoIterator`,
  and `row`/`rows`, `row`/`iter` and `get`/`iter` name one operation in three ways.
  Target: capture `self` by copy, use the `Ragged` names, and implement `IntoIterator`.
- [ ] **`ascii/mod.rs:123-193`: ASCII has no header-only schema and fewer accessors.**
  - `AsciiColumnData` has no `as_*` and no `TryFrom`.
  - `AsciiColumn` has no `tdisp`, although §7.2.2 defines `TDISPn` for `TABLE`.
  - The extent read is the same as `schema/mod.rs:48-53`.

  Target:
  - `AsciiSchema::parse` and `Ascii::new`.
  - `Hdu::ascii_schema()`, beside `table_schema()` (`reader/hdu.rs:191`).
  - The accessors match those of `ColumnData`.
  - `column.rs` holds the extent reader, `validate_table_field_count` and
    `MAX_TABLE_FIELDS` (from `hdu/mod.rs:15-24`).
- [ ] **`crate::column` and `crate::bintable::column` share a name.** Target: rename the
  bintable module to `bintable::field`.
- [ ] **`docs/design.md`, Tables.**
  - New decision: TFORM trailing characters are read and ignored.
  - New decision: a column defect fails only its column.
  - New decision: `X` and `PX` decode to bits.

### Batch 7: Image model and the sample trait

Files: `src/words.rs`, `src/image_model/{mod.rs, scaling.rs, view.rs, data.rs,
unsigned_data.rs, physical_sample.rs}`, `src/bitpix/mod.rs`, `src/bintable/tform_kind.rs`,
`src/bintable/column_data.rs`, `src/compress/plane.rs`, `src/hdu/image_geometry.rs`,
`src/reader/image_reader.rs`, `src/compress/decode/decode_buffer.rs`.

- [ ] **(checked, local) `image_model/mod.rs:196-207`, `words.rs:40-60`: a safe fn can
  cause UB.** `view_words` is safe and calls `words::samples`, whose bound is only a
  `debug_assert!`. Target: `samples` and `samples_mut` are safe fns with one `assert!`
  per call, as `bytemuck::cast_slice` checks. Remove the `unsafe` blocks at
  `image_model/mod.rs:170, 198` and `compress/decode/decode_buffer.rs:39`. Add
  `words::bytes_mut` for batch 8.
- [ ] **(checked) `reader/image_reader.rs:164` vs `:241`, `scaling.rs:36, 71-98`: `read`
  refuses what `view` accepts.** `read` validates through `Image::new`, while
  `Scaling::from_header` validates nothing. A float image with `BLANK`, or a `BLANK`
  outside the `BITPIX` range, fails `read` but passes `view`, `physical_into` and
  `image_metadata`. Target:
  - `Scaling::from_header` takes the `Bitpix`, ignores a float `BLANK`, and range-checks
    an integer `BLANK`. cfitsio ignores a float `BLANK`, and astropy warns and ignores
    it. The `Hdu::image_metadata` doc states the rule.
  - The same scaling then serves every path. The reader builds images from validated
    geometry, without the second check.
  - `Image::new` keeps its check for images that a caller builds.
- [ ] **One sealed `Sample` trait.** Defect: per-type dispatch is written by hand:
  - the physical loops, `image_model/mod.rs:98-161` and `tform_kind.rs:193-218`;
  - the sign flips, `view.rs:97-111` and `unsigned_data.rs:47-58`;
  - the null ranges, `scaling.rs:85-91` and `builder.rs:833-838`;
  - the `TformKind` → `Bitpix` maps, `tform_kind.rs:110-116` and `compress/plane.rs:46-53`;
  - the macros, `data.rs:22` and `column_data.rs:33`.

  Target:
  - Grow `words::Sample` with `const BITPIX`, `from_be`/`to_be`, `View` wrap and
    unwrap, and the null range.
  - Add `ElementKind::bitpix() -> Option<Bitpix>`.
  - The table physical decode reuses the image loop.
  - The named `as_*` methods delegate to the trait.
  - lumos can then drop `PlaneSample` and `ExtensionSample`.
- [ ] **`scaling.rs:63-69`, `image_model/mod.rs:105-159`: the physical loop tests `blank`
  for each sample and runs the affine for identity.** lumos rewrote this conversion as
  branch-free loops (`lumos/src/io/fits/decode/pixels/mod.rs:440-498`). Target:
  - Match `blank` and identity once, outside the loop.
  - Use `bscale.mul_add(raw, bzero)` (decided), which gives the correctly rounded f64
    of the exact affine. cfitsio rounds twice, and astropy rounds twice in float32 for
    `BITPIX` ≤ 16. A test that compares exactly with a cfitsio output can move by one
    ulp, so derive that expectation again.
  - Measure with `image_model/bench.rs`.
- [ ] **`physical_sample.rs:5-6`, `view.rs:82`: the claim "each value is the correctly
  rounded `f32`" is false.** The f64 result is rounded again to f32. Target: the doc
  states the double rounding.
- [ ] **`scaling.rs:116`: the `is_identity` doc says "decoding needs no arithmetic",
  but the check ignores `BLANK`.** Its one caller (`add_to_header`, `:102`) needs the
  affine test alone. Target: the doc says that the affine is the identity, and that
  `BLANK` is a separate fact.
- [ ] **API gaps.**
  - `Metadata` has no `sample_type()`, so lumos calls `SampleType::from_scaling` by hand
    (`decode/plan.rs:57`). `Borrowed::sample_type` delegates to the new method.
  - `View` has no `From<&[T]>`, so lumos spells `View::F32(..)` (`plane_hdu.rs:28, 36`).
  - `UnsignedData` has no `len`, `is_empty`, `as_*`, `TryFrom` or borrowed twin.
- [ ] **Free fns become methods:** `extend_physical`, `physical_view`,
  `extend_physical_be`, `swap_into_words` and `view_words`
  (`image_model/mod.rs:98-196`). Write `Data::view_range` (`data.rs:101`) inline at its
  one caller.

### Batch 8: Reader I/O and the HDU list

Files: `src/reader/{mod.rs, source.rs, data_source.rs, image_reader.rs, hdu.rs,
hdu_selector.rs}`, `src/allocation.rs`, `src/hdu/mod.rs`, `src/groups/mod.rs`,
`src/fits_editor/mod.rs`, `src/compress/table/mod.rs`, `src/bintable/selection.rs`,
`src/lib.rs`, `src/error.rs`, `docs/design.md`. Needs batches 4, 6 and 7.

- [ ] **(checked) `reader/source.rs:119-127`, `allocation.rs:18-29`: `read_append` grows
  with `try_reserve_exact` and zero-fills.** One call for each picked row and each VLA
  cell (`data_source.rs:175, 193`) gives O(n²) copying, a memset, and a seek for each
  call. Target: the `Source` of the Target design (amortized `try_reserve`,
  `take(len).read_to_end`, `read_ranges`, a tracked position). `try_resize` keeps
  `try_reserve_exact` only for a buffer sized once.
- [ ] **(checked) `reader/mod.rs:648-660`, `data_source.rs:80-92`: a partial read of a
  compressed table reads the whole container unit.** A loop of `read_table_cell`
  reads it N times. Each call also binds the cells again
  (`compress/table/mod.rs:527-553`) and allocates the whole restored heap. Target:
  - Pick the touched tile rows, as the image path does (`TableRows::Picked`).
  - Bind the cells once for each `Hdu`, cached as `TiledTable` is.
  - Allocate only the heap ranges that the selected tiles reference.
- [ ] **`image_reader.rs:234-264`, `data_source.rs:31`, `mod.rs:597`: a whole-image read
  stages the padded unit in `DataSource.scratch`, which never shrinks.** Sections
  gather runs into `section_bytes`, then swap them. For a `SliceSource` that copy is
  waste, and lumos's chunked `view` loop pays it for each chunk. Target:
  - Read straight into the caller's `words` (`words::bytes_mut`), feed the checksum
    with the big-endian bytes, then swap in place.
  - A slice source swaps from the borrowed runs.
  - `physical_into` and `read_groups` stage in bounded chunks.
- [ ] **`reader/mod.rs:618-694`, `image_reader.rs:293`, `data_source.rs:116`,
  `fits_editor/mod.rs:245`: the HDU list has no owner.** Target:
  - `crate::hdu::Hdus` with `scan`, `select`, `images` and `find`, shared by the reader
    and the editor.
  - Reader methods borrow `self.hdus` and `self.data` apart, and the rest of the wide
    signatures become param structs.
  - `FitsEditor::hdu(selector)` and `FitsEditor::images()` come from `Hdus`.
  - Delete `read_table_as` (`:353`), a fn-pointer param that serves only a test.
  - `resolve_columns` and `read_table` stop the index round trip (`:339-349, 415-439`).
- [ ] **`data_source.rs:116-231`: `DataSource` also compacts rows, repacks the heap and
  plans region runs.** So decoded bytes are wrapped in a fake `SliceSource`
  (`reader/mod.rs:489, 629`). Target:
  - `bintable::selection` is generic over a fetch, and takes the `Selection` types from
    `reader/mod.rs:56-126`. The public path `table::Selection` stays.
  - `DataSource` only fetches.
  - `visit_image_region_runs` moves beside `Region`.
- [ ] **(checked, local) `reader/hdu.rs:107`: with no `compression` feature, `is_image`
  counts `CompressedImage`.** `images()` then yields HDUs that `image()` refuses
  (`hdu.rs:180`). Target: gate the arm, as `table_schema` is gated (`hdu.rs:199`).
- [ ] **`reader/mod.rs:132-174, 316-324`: `DataUnit.data_range` always starts at 0, and
  `read_data_unit` reads up to 2879 fill bytes that nothing uses.** `DataUnitView` and
  `into_padded` serve only tests. Target: `read_data_unit` reads `data_bytes` and
  returns `Vec<u8>`. Delete the view and the range. `read_table` and `read_ascii_table`
  skip the fill.
- [ ] **`reader/mod.rs:537-547`: a missing `-TAB` table is a formatted `InvalidWcs`.**
  Also, the `BinTable` filter misses a `-TAB` array in a compressed table. Target:
  `Hdus::select`, then `table_schema()` and `select_rows`, which handle compressed
  tables, and `HduNotFound` passes through. The `read_wcs` doc states that it resolves
  `-TAB` for image headers only.
- [ ] **`reader/hdu.rs:133`: a non-text `EXTNAME` on any HDU fails every name lookup.**
  lumos repeats the `EXTVER` default (`lumos/src/io/fits/decode/selection.rs:17-21`).
  Target: `Hdu::{extname, extver, extlevel}` with the defaults, `matches` built on them,
  and a non-text name that matches nothing.
- [ ] **`groups/mod.rs:159-206`: random groups.**
  - `parameters_physical` and `array_physical` return a new `Vec<f64>` for each group,
    which design.md refuses ("A physical read appends to the caller's `Vec`").
  - An absent `PTYPEn` is stored as `""`, so `parameter_physical(g, "")` sums every
    unnamed parameter.

  Target: `Option<String>` names, `parameters_physical_into` and
  `array_physical_into<T: PhysicalSample>`, and a name resolved once to its indices.
- [ ] **`error.rs:238-248`: one failure has several variants.** `DataSizeMismatch`
  carries elements, bytes or tile rows. A u64 → usize failure is `DataUnitTooLarge` in
  one place and `DataUnitOverflow` in another, and a P/Q range failure is
  `DataUnitOverflow`. Target: `DataSizeMismatch` carries a unit and `u64` sizes, one
  overflow variant exists, and a new `DescriptorOutOfRange` holds the P/Q case.
- [ ] **Minor items.**
  - `groups/mod.rs:105` returns an error for an invariant; make it a `debug_assert!`.
  - The random-groups axes are validated twice (`hdu/mod.rs:53, 148`).
  - `padded_bytes` is computed twice (`reader/mod.rs:256`, `hdu.rs:157`).
  - `Keyword::new("NAXIS1")` (`hdu/mod.rs:232`) becomes `NAXIS.indexed(1)`.
  - `decode_be_into` (`endian.rs:68`) repeats `decode_be_cells_into`.
  - The `HduKind::CompressedTable` doc (`hdu/mod.rs:87`) names the refused
    `read_compressed_table`.
  - The crate doc (`lib.rs:25`) calls a crate-private formula public.
- [ ] **`docs/design.md`, Reading.**
  - New decision: a read lands in the caller's buffer, and a source reads merged
    ranges.
  - New decision: `Hdus` is the one owner of the HDU list.

### Batch 9: Time coordinates

Files: `src/time_coordinates/{frame.rs, time_axis_kind.rs, datetime.rs, phase_axis.rs,
bounds.rs, mod.rs}`, `src/unit.rs`, `src/error.rs`.

- [ ] **(checked, local) `frame.rs:96-99, 134-141`: `TIMEOFFS` is scaled by `CUNITi`.**
  With `TIMEUNIT = 's'`, `CUNIT = 'd'` and `TIMEOFFS = 10`, the time moves by 10 days.
  Target: convert the offset with `self.unit` and the value with its own unit, then
  add the two expansions.
- [ ] **`frame.rs:125-139`, `time_axis_kind.rs:226-240`: `time_axis_mjd` parses text on
  each call.** That is `Ctype::parse` twice, `Scale::from_str` twice with an uppercase
  copy, and one `Unit` parse. `TIME` matches with no regard to case, but the other
  heads match in uppercase only. Target:
  - A `TimeAxis { scale, unit }`, resolved once with one case rule.
  - It takes the world value and the unit, not `&Wcs`. This cuts the edge
    `time_coordinates` → `world_coordinates`.
- [ ] **`error.rs` `InvalidTime { detail }`.**
  - `datetime.rs:77` says `DATE` for every date keyword.
  - `phase_axis.rs:91-93` builds a message where `MissingKeyword` exists.
  - `phase_axis.rs:94-104` checks finiteness, which `get_real` already guarantees.

  Target: `InvalidDatetime { keyword }`, `InvalidTimeScale { keyword }` and
  `TimeOutOfRange`. Delete the dead checks.
- [ ] **`datetime.rs`.**
  - Its public fields carry invariants, which are checked twice (`:178-200`, `:240`).
  - `to_mjd(scale)` uses the scale only to choose an error.
  - `day_fraction` (`:174-176`) rounds twice, against its doc.
  - `"23:59:59.99999999999999999"` parses to 60.0 and is refused.
  - The pre-2000 form `DD/MM/YY` (`docs/refs/fits_standard40.md:1415-1421`) is refused,
    which cfitsio `ffs2dt` reads.

  Target:
  - Private fields and a checked constructor.
  - `to_mjd(self)`.
  - `Expansion::two_sum(…).divided(86400)` for the fraction.
  - The seconds are parsed as an integer part and a fraction, so the value cannot round
    to 60.
  - The legacy form is read.
- [ ] **`bounds.rs:38-70`: `Bounds::from_header` is all or nothing.** A leap-second
  `DATE-END` also loses `XPOSURE` and `TELAPSE`, and the scale is parsed once for each
  date. Target: `Bounds` takes the `Frame`, and each field fails on its own.
- [ ] **`frame.rs:61`: `TRPOSn` past column 999 errors.** Target: `Keyword::fitting`, as
  `PhaseAxis` uses it.
- [ ] **`time_coordinates/mod.rs:23` `SEC_PER_DAY` repeats `unit.rs:33`
  `SECONDS_PER_DAY`.** Delete one. `unit::SI_PREFIXES` becomes private.

### Batch 10: WCS precision

Files: `src/world_coordinates/{celestial_pole.rs, celestial_transform.rs,
projection/mod.rs, projection/healpix.rs, axis_transform/spectral_kind.rs, tests.rs}`,
`docs/design.md`, `docs/research/`.

- [ ] **(ISSUES.md: TAN round trip) `celestial_pole.rs:152-172`,
  `projection/mod.rs:553, 750`.** The world → native direction takes
  θ = `asin(sin_t)` with `sin_t` near 1. That turns an error of k·1.1e-16 into
  k·1.1e-16/sin ε. At the IRAC corner (ε ≈ 0.061°) the result is about 6e-12°. At 1e5
  px of 1e-6° it is about 3.6e-12°. Both match the logged misses. The native → world
  direction has the same `asin` at the celestial poles (`:154`). θ in degrees across
  the projection boundary adds a floor of about 7e-15°. `dpr.cos()` at δp = 90° gives
  6e-17, not 0. Target:
  - Precompute the rotation at construction with `sind`/`cosd` of δp and αp.
  - For each point, x = sin(δ−δp) + 2·cosδ·sinδp·sin²(Δα/2) (an exact identity for
    sinδ·cosδp − cosδ·sinδp·cosΔα), y = −cosδ·sinΔα, and z = sinθ as today.
  - ζ = atan2(hypot(x, y), z). This is the well-conditioned form of the wcslib `sphs2x`
    branch `|z| > 0.99 → θ = ±acos(√(x²+y²))`. Do the same in the native → world
    direction.
  - Pass ζ in radians, or (sin θ, cos θ), to the projection. TAN becomes x = R2D·X/Z.
  - Bound the round-trip tests by the ulp of the offsets. `world_coordinates/tests.rs:65-80`
    allows 1e-5 px and claims "~1e-9°".
- [ ] **(checked, local) `projection/mod.rs:947-964`: the ZPN inverse returns an
  unconverged ζ with no error.** After 100 clamped false-position steps, the bracket is
  only guaranteed to 2.7e-5 of its start. wcslib `zpnx2s` has the same gap, and the
  code copies its loop. Target: a bracketed Newton on the exact
  derivative from `evaluate_zpn`. It falls back to bisection when a step leaves the
  bracket, and returns `WcsNoConvergence` when it does not converge, as AIR does. The
  degree-2 branch (`:918-933`) uses the stable quadratic.
- [ ] **`projection/mod.rs:409-423, 498-523`, `healpix.rs:95-96`: SIN, SZP and the
  HEALPix polar cap cancel near the centre.** For SIN, σ = 1 − √(1−r²), then
  θ = asin(1−σ), loses about 1e-16/r rad. Target:
  - The stable roots q = −(b + sgn b·√d)/2, with q/a and c/q.
  - ζ = 2·asin(√(σ/2)).
  - In the HEALPix cap, ζ = 2·asin(σ/√(2K)).
- [ ] **`projection/mod.rs:563-564, 620, 786`: MER and SFL.** MER cancels near the
  equator. SFL and BON use `(y·D2R).cos()`, which is never 0 at ±90°. Target:
  asinh(tan θ) and atan(sinh y), and `cosd`.
- [ ] **`axis_transform/spectral_kind.rs:105-113, 253, 264`: cancellation near the rest
  value.** Target: c(ν₀−ν)/ν₀, (λ−λ₀)/λ₀ and c(ν₀−ν)(ν₀+ν)/(ν₀²+ν²).
- [ ] **`docs/design.md`, World coordinates:** new decision: the native colatitude
  crosses the projection boundary in radians, from the `atan2` form. Write the measured
  round-trip errors before and after to `docs/research/`.

### Batch 11: WCS structure

Files: `src/world_coordinates/{mod.rs, axis.rs, declared.rs, linear_transform/mod.rs,
tabular/mod.rs, table_wcs/mod.rs, distortion/tpv.rs, projection/*, axis_transform/*,
celestial_transform.rs, bench.rs}`, new `angle.rs`, `header_read.rs` and `wcs.rs`,
`src/error.rs`, `docs/design.md`, `docs/research/`.

- [ ] **(checked, local) `table_wcs/mod.rs:203`: a table WCS copies `PV` for m in 0..=20
  only.** TPV reads up to m = 39 (`distortion/tpv.rs:12`), so a pixel-list or
  array-column `-TPV` loses its degree-6 and degree-7 terms. Target: one PV index range
  for every reader. The standard allows m ≤ 99.
- [ ] **(checked) `tabular/mod.rs:172`, `mod.rs:211, 252-265, 489`: a TAB axis has the
  wrong unit label.** Its coordinates are scaled to SI by `world_unit_scale`, but
  `world_units` stays `None`, so `axis_world` falls back to the declared `CUNIT` and
  reports "nm" for a value in metres. Target: one per-axis unit resolution for both
  paths, and a per-axis world unit in `Metadata`. This also gives the unit of a CUNIT
  rad axis and a FREQ-W2F axis.
- [ ] **(ISSUES.md: TAB regression) `tabular/mod.rs:495-539`: the 1-D inverse scans
  O(K) intervals for each point.** wcslib `tabx2s` scans too, because the standard
  allows non-monotonic arrays and the first containing interval wins. The regression is
  one more cycle in each iteration of that scan. Target:
  - In `from_table`, split the coordinate array into maximal strictly monotonic runs.
    Break a run at equal neighbours and at equal index values.
  - For each point, binary-search the runs in order. The first run that holds the
    target gives the scan's interval, and `partition_point` takes the lower interval at
    a node. The result does not change, at O(R·log(K/R)).
  - Delete the measurements written in `bench.rs:71-80`, and write the new figure to
    `docs/research/`.
- [ ] **`tabular/mod.rs:15, 311-333, 541-595, 636-653, 707-746`: TAB allocations and the
  M-D inverse.**
  - 1-D allocates two `Vec`s for each point.
  - M-D searches every cell under a budget of 4M work units, so valid points beyond
    about 262k cells fail with `WcsNoConvergence`.
  - M-D allocates about 13 times for each point.

  Target:
  - A 1-D variant with no `Vec`.
  - An M-D variant with cell extrema built at construction, as wcslib `tabset` builds
    them. The extrema prune the cells, and the budget counts only the cells that remain.
  - Correct `docs/research/2026-10-24-wcs-into-slice.md`.
- [ ] **`projection/mod.rs:281-284, 476, 502, 533, 661-663, 690, 744, 1069, 1138`,
  `healpix.rs:65, 106`: construction constants are computed for each point.** For COO,
  that is two `powf`, two `ln_1p` and about six trig calls per point. Target:
  - A private kernel enum for each family, built in `Projection::parameters`.
  - Each family has its own file, with the forward, the inverse and the constants.
    Today `projection/mod.rs` has 1201 lines, and each pair is about 200 lines apart.
  - One code table replaces `code()` and `from_code()` (`:135-198`).
- [ ] **`mod.rs:103-125, 292-296`, `linear_transform/mod.rs:240-276`: per-axis facts in
  parallel vectors.** These are `reference`, `world_units`, `axis_transforms`,
  `Axis.crval`, the copy in `SpectralTransform`, and `Declared`. `distortion` is stored
  but derivable, and `&self.axes` goes into each linear call for `CRPIX`. Target: one
  `AxisPlan` for each axis, `LinearTransform` owns `CRPIX`, and `distortion()` is a
  method.
- [ ] **`mod.rs:185-199, 230, 439`, `tabular/mod.rs:101`, `axis_transform/mod.rs:63`,
  `axis.rs:9`: `CTYPE` is compared as text after the parse.** Target: a typed per-axis
  kind in `AxisPlan`.
- [ ] **`mod.rs:470-528, 510-515, 621-623, 648-658`: `axis_world` repeats the pipeline.**
  It runs SIP twice and allocates for TAB. Target: `CelestialTransform::{to_world,
  to_intermediate}`, which own the latitude domain check. `axis_world` is built on them.
- [ ] **Module cycles.** The children import header readers and angle math from the
  root:
  - `tabular:10-11`, `table_wcs/mod.rs:27-28` and `table_wcs_resolver.rs:8, 14`;
  - `tpv.rs:10` and `celestial_transform.rs:12`;
  - `linear_transform:12, 14`, and both frame modules at `:9`;
  - `celestial_pole.rs:5-10`, `projection/mod.rs:7`, `cube.rs:9` and `healpix.rs:4-6`.

  `axis_transform` and `spectral_kind` share the physical constants in both
  directions. Target: `angle.rs`, `header_read.rs` and `wcs.rs`. The constants move into
  `spectral_kind`.
- [ ] **`error.rs:155` `InvalidWcs { detail }` (about 35 sites).** Target: one variant
  per case. `ConflictingWcsKeywords` and `WcsInvalidPole` get enum details.
- [ ] **`Wcs::write_into` writes the image form only.** Target: the doc states it.
- [ ] **`docs/design.md`, World coordinates.**
  - New decision: a 1-D TAB inverse searches monotonic runs.
  - New decision: one plan for each axis holds its unit.

### Batch 12: Compression codecs and parallel output

Files: `src/compress/{mod.rs, gzip.rs, rice.rs, plio.rs, hcompress.rs, quantize.rs,
bench.rs}`, new `src/compress/nocompress.rs`, `src/compress/encode/mod.rs`,
`src/compress/decode/{tile_decoder.rs, null_mask.rs, image_decode_plan.rs,
float_quantization.rs}`, `src/compress/table/mod.rs`, `src/error.rs`.

- [ ] **Codec dispatch is written six times:** `encode/mod.rs:101-144, 366-406`,
  `tile_decoder.rs:217-279`, `null_mask.rs:110-146`, the table's
  `compress_payload_into` and `decompress_column_into`, and `bench.rs:277-332`.
  Problems that follow:
  - NOCOMPRESS is inlined four times, with different size checks.
  - Five codec enums exist: `ImageCodec`, `Compression`, `Algo`, `QuantizedCodec` and
    `LosslessCodec`.
  - The lossless-float GZIP rule is written twice (`encode/mod.rs:304-308`,
    `float_quantization.rs:103-114`).
  - Every codec gets `gzip_level()` and `hcompress_scale()` (`encode/mod.rs:83`), and
    `Gzip.shuffle: bool` is a mode flag.
  - The signatures are asymmetric (gzip bytes → i64, PLIO i16 → BE), with owning and
    `_into` twins.
  - The bench measures its own copy, not the production path.

  Target: one `TileCodec` (see Target design). Every path and the bench call it, and
  NOCOMPRESS gets its own module.
- [ ] **`image_decode_plan.rs:70-124`: the parallel decode allocates a `Vec` for each
  tile and a `Vec<Vec<D>>` for each wave, then scatters serially.** The section decode
  is always serial. Target:
  - For an image whose higher axes are not tiled, use `par_chunks_mut(band_len)` and
    decode the tiles of each band in place.
  - Other images keep the waves, with one reused buffer.
- [ ] **`table/mod.rs:354, 552-553, 568`: VLA tables.** A table with a VLA column
  decodes serially, a read of a few rows zeroes the whole heap, and the write gzips the
  descriptor blocks inside the serial fold. Target:
  - A layout from prefix sums of the array lengths, then a second parallel map.
  - A heap staging area for each tile.
- [ ] **`encode/mod.rs:89-147, 456-565, 638`, `hcompress.rs:57`: each compressed tile is
  kept, then appended.** The peak is twice the compressed size. PLIO words are pushed
  one at a time, and `hcompress_tile_into` copies the plane for each tile. Target: the
  prefix-sum layout writes in place, PLIO uses `extend_be`, and HCOMPRESS swaps the
  plane.
- [ ] **API symmetry.** An integer image ignores `with_quantization`
  (`encode/mod.rs:71-76`), but a table refuses it (`table/mod.rs:220-224`).
  `resolve_tile_shape` turns a tile of 0 into 1 with no notice (`encode/mod.rs:666`).
  Target: both refuse.
- [ ] **`compress/mod.rs:128-152`.** `HcompressScale::tile_scale` serves only the
  encoder, so it moves there.
- [ ] **`error.rs`: the compression string variants become cases.** These are the scale
  overflow, lossy HCOMPRESS with nulls, HCOMPRESS that needs 2-D, a bad Rice field, the
  PLIO bounds, and `table/mod.rs:222-232, 435`.

### Batch 13: Test suite migration and quality

Files: each `tests.rs`, `tests/*.rs`, `src/bench/`, and the inline test mods named
below. Needs batch 1. A test file moves to the tree when a batch above rewrites its
code. The other files move here, in this order of savings.

- [ ] **`world_coordinates/projection/tests.rs`:** about −375 lines.
  - Five astropy tests (`:28, 71, 103, 112-257, 735-861`) become one
    `PROJECTION_GOLDEN` table, with a round trip for each row.
  - Four wcslib native goldens (`:47, 500, 630, 711`) become one `NATIVE_GOLDEN`.
  - One PV list replaces three (`:448-476`, `:121`, `world_coordinates/tests.rs:491-501`).
  - Add an ARC golden, and round trips at the poles and in the south.
    `projections_round_trip` has three points, all with θ ≥ 50°.
- [ ] **`fits_writer/tests.rs`:** about −350 lines.
  - `fixed_columns_by_stored_type` and `vla_columns_by_stored_type` (`:220-356`) become
    one list.
  - `NullBoundary` (`:953`) with `ImageBlankBoundary` (`:1629`), and
    `NonfiniteTableScale` (`:1010`) with `NonfiniteImageScale` (`:1708`), become two
    tables.
  - The three checksum tests (`:1420-1492`) become one table of (byte flipped →
    report).
  - `assert_ne!(DATASUM, "999")` (`:603-604`) becomes the exact `"655380"`: i16
    [10, 20] is the word 0x000A0014, which is 655380.
- [ ] **`reader/tests.rs`:** about −250 lines.
  - `ramp`, `gather` and `assert_same_bits` replace `:1031-1158`, `:1277-1308` and
    `assert_image_data_exact` (`:1046`).
  - `counted` replaces the four `CountingSource` setups.
  - The `-TAB` loop (`:141-157`) uses `tab_header`.
  - Delete the function-local `use`s (`:342-343, 496, 519, 548, 576-577`).
- [ ] **`compress/decode/tests.rs`, `compress/encode/tests.rs`, `compress/table/tests.rs`:**
  about −380 lines.
  - Use `OneTile`, `nan_blind_bits` and `odd_plane`.
  - One half-step tolerance helper replaces four copies (`encode/tests.rs:222, 362,
    430, 720`).
  - `hcompress_lossless_write_round_trips_exactly` (`:845-905`) becomes one loop.
  - One plain-versus-compressed helper replaces `check_table_roundtrip`,
    `compressed_round_trip`, `plain` and `compressed`.
  - `table/tests.rs:630-647` asserts the values and the error variant.
  - Assert the tiling that `encode/tests.rs:821` claims.
- [ ] **`world_coordinates/axis_transform/tests.rs`:** about −235 lines. `SpectralGolden`
  derives `scale` from the kind. The VELO-F2V golden gets one bound: it is checked at
  1e-9 in `spectral_frame/tests.rs:71, 101, 138` but at 2.7e-7 at
  `axis_transform/tests.rs:386`.
- [ ] **`time_coordinates/tests.rs` (934 lines for 9 submodules).** Move the tests
  beside their owners: `mjd`, `datetime`, `frame`, `scale`, `unit`, `phase_axis` and
  `bounds`. Turn `:363-406` and `:629-658` into tables. Add an exact cross-check of the
  `Mjd` double-double helpers against i128 arithmetic.
- [ ] **`world_coordinates/tests.rs`, `distortion/tests.rs`, `table_wcs/tests.rs`,
  `spectral_frame/tests.rs`, `celestial_frame/tests.rs`, `tabular/tests.rs`,
  `linear_transform/tests.rs`:** move to `WcsHeader` and `assert_round_trip` (14 inline
  round trips).
  - One SIP header replaces two (`world_coordinates/tests.rs:655`,
    `distortion/tests.rs:90`).
  - `WcsHeader::alternate` replaces the hand loops (`:518-536`,
    `distortion/tests.rs:188-200`).
  - The SIP and TPV tests at the end of `sip.rs` and `tpv.rs` move to
    `distortion/tests.rs`.
- [ ] **Tolerances that contradict their claim.**
  - BON at θ₁=0 is "exactly" SFL, but is checked at 1e-10 (`projection/tests.rs:430`).
  - HPLN/HPLT are "exactly" RA/DEC, but are checked at 1e-12
    (`world_coordinates/tests.rs:176`).
  - `projections_round_trip` is checked at 1e-7° (`:485`).
  - Round trips are checked at 1e-6 and 1e-7 px (`:857, 438`).
  - Unit normalization is checked at 1e-10 relative (`axis_transform/tests.rs:480`).
  - CRPIX → CRVAL is checked at 1e-9° (`linear_transform/tests.rs:167`,
    `axis_transform/tests.rs:127`).

  Target: each one becomes `Tolerance::EXACT` or a stated bound. `CEA_GOLDEN`,
  `CROTA_GOLDEN`, the grism, cube and HPX goldens, and `slant_sin` (`:50`) get a source.
  The generating script and its date go to `docs/research/`.
- [ ] **Bare `is_ok()` and `is_err()`** at `projection/tests.rs:700, 701, 923, 947, 957,
  960`, `linear_transform/tests.rs:126`, `time_coordinates/tests.rs:121, 150, 175`,
  `hierarch_name.rs:100-102`, `compress/tests.rs:6, 7, 18, 30, 38`,
  `decode/tests.rs:233, 236`, `tdim/tests.rs:15`, `image_model/tests.rs:229` and
  `ascii/tests.rs:35`. Target: assert the value or the variant. `reader/tests.rs:511`
  (`capacity() > len()`) states its reason.
- [ ] **Layout.**
  - Split the fat inline test mods into `foo/{mod.rs, tests.rs}`: `rice.rs` (158
    lines), `quantize.rs` (174), `checksum.rs` (60%), `character_field.rs` (41%),
    `allocation.rs` (42%) and `table_wcs_resolver.rs` (40%).
  - Fix the path `super::super::column::Column::byte_offset`
    (`column_reader/tests.rs:32`).
  - `hdu/tests.rs:5-50` becomes one classify table.
  - Delete the local re-imports at `fits_writer/tests.rs:754, 841` and
    `time_coordinates/tests.rs:289, 342`.
- [ ] **Slow candidates.** Time `encode/tests.rs:819` (HCOMPRESS of 6000×40) and `:973`
  (4.1 M samples) in the debug profile. Make smaller any test that takes more than 1 s.
- [ ] **Coverage.** Add direct tests of `celestial_transform.rs`, `celestial_pole.rs`,
  `Header::require_usize`, `optional_usize`, `append_filtered_from` and
  `rename_keywords`.
