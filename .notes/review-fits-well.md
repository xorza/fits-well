# Review: fits-well

Three parts, each a proposal and a plan: Part 1 the production code, Part 2 the test
suite, Part 3 the benches. A batch number without a part names a batch of the same part.

## Order across parts

- **Part 1 Batches 1–5 fix bugs and wait for nothing.**
- **Part 2 Batches 1–8 go before Part 1 Batch 6.** Part 1 Batches 6–14 change APIs that many
  tests call (reads into the caller's buffer, the codec shape, error cases). A failure that
  prints its value and a setup that sits in one shared helper make each of those changes one
  edit, not forty.
- **Part 3 Batches 1–6 go before the Part 1 batch that each case measures** (the
  coverage map at the end of Part 3). A change needs a number from before it. Part 3 Batch 4
  changes the scene, so the baseline is taken after Part 3 Batch 4.
- **Part 2 Batches 9 and 11 go after Part 1 Batches 6, 7 and 14.** Those batches move
  `PlainRead`, the HDU record and `Bitpix`, and the test files follow the modules.

# Part 1 — Production code

Scope: `src/` of the crate, test code aside. Paths are relative to `src/`.

## Proposal

### Root causes

1. **A size the file claims reaches an allocation before the file bounds it.** `Hdus::scan`
   keeps an HDU whose data unit runs past the end ("a later read bounds-checks it"), and
   the reads then reserve, zero-fill or grow to `data_bytes`, `NAXIS1·NAXIS2` or a sum of
   descriptor counts before `check_range` runs. Some of those reservations are infallible,
   so a 3-block file aborts the process; the rest commit gigabytes before
   `UnexpectedEof`, and leave the staging buffer at that capacity.
2. **Durable per-HDU state lives in the reusable scratch.** The tile-index cache sits in
   `Scratch`, keyed by a copy of `Hdu::data_offset`, so a handle cannot hold a
   `&TileIndex` and re-finds it with a linear `find` and an `expect` in five places; the
   `ImageReader` dispatch is duplicated and a six-parameter free fn works around the
   borrow.
3. **The HDU record lives in `reader`, and its kind has two models.** `Hdu`, `Hdus`,
   `HduSelector` and `HeaderUnit` serve the reader and the editor alike, so the editor
   imports reader internals; `HduRole` is a coarser `HduKind`, every site runs
   `from_header`, `classify` and `data_extent` in turn, and the editor copies `Hdu` facts
   into `Placement` and re-derives `padded_len`.
4. **Recorded decisions that the code does not follow.** One fused multiply-add (ASCII and
   complex columns round twice); a bad optional column keyword fails only its reads
   (`TTYPEn`/`TUNITn` refuse the table); `CTYPE` parsed once (each axis is parsed 5–7
   times); a codec lends its output (rice, PLIO and NOCOMPRESS take the caller's buffer);
   a data unit is never staged (a binary-table write stages the whole unit).
5. **Some reads allocate per call where the design has the caller own the buffer.** Column
   reads return a new `Vec`; a compressed table decodes per call and per chunk; gzip
   encode builds a deflate state per tile; a compressed `physical_into` keeps a
   region-sized buffer on the reader.
6. **Error variants that merge cases.** `KeywordOutOfRange` covers syntax, range and
   structure; `UnexpectedEof` covers truncation, a descriptor past the heap and a caller
   logic error.

### Target design

- **A read bounds its unit once, at the entry.** `DataSource::unit_len(&Hdu) ->
  Result<usize>` checks `data_offset + data_bytes <= size()` and replaces `data_len`; the
  image handle, `read_groups` and the tile-index load call it before they size anything.
  `StreamSource::slice` checks the range before it grows the staging buffer. A size the
  file claims indirectly (a sum of descriptor counts, which may overlap, §7.3.5) is summed
  first and reserved with `try_reserve_exact`. No reservation from file data is infallible.
- **`Scratch` holds reusable buffers alone.** The tile indexes are a `FitsReader` field,
  `Vec<Option<TileIndex>>` indexed by HDU index; `load` returns `&TileIndex`, which the
  image handle holds. `TileIndex::key`, `TileIndexes::get` and the five expects go.
- **`crate::hdu` owns the HDU record:** the kinds, `Hdu`, `Hdus`, `HduSelector` and
  `HeaderUnit`. `HduKind::from_header(header, position)` replaces `HduRole`, and
  `data_extent(header, kind) -> Result<u64>` replaces `DataExtent`. The editor borrows
  `&Hdu` beside `&mut self.file`.
- **Every read fills a buffer the caller owns,** columns as images do: public `raw_into`,
  `physical_into<T: PhysicalSample>`, `vla_into`, `unsigned_into`. Decoders keep their
  state on the owner: one `flate2::Compress` per encode scratch, a decoded-tile buffer on
  the reader.
- **One codec shape:** `decode(stream, tile, scratch) -> Result<&mut Vec<i64>>` and
  `encode(samples, tile, scratch) -> Result<&[u8]>`, with the stream's element type from
  `TileCodec::stream_kind() -> ElementKind`.
- **WCS parses each `CTYPE` once,** into the axis plans, and `Declared::write` writes the
  frame, the rest values and the pole from the parsed model, not from replayed keywords.

## Plan

### Batch 1 — Bound every file-claimed size before it is allocated

Files: `reader/data_source.rs`, `reader/source.rs`, `reader/image_reader.rs`,
`reader/mod.rs`, `reader/hdus.rs`, `compression/decode/tile_index.rs`,
`bintable/column_reader/mod.rs`, `endian.rs`, `bintable/selection.rs`.

- [ ] `reader/data_source.rs`: add `DataSource::unit_len(&Hdu) -> Result<usize>`, which
  fails `UnexpectedEof` when the unit runs past `size()`. It replaces `data_len` and the
  inline copy in `DataSource::table` (`:129`).
- [ ] `reader/image_reader.rs:269`: `PlainRead::physical` calls an infallible
  `out.reserve(runs.len() / elem_size)`. A 3-block file declaring 10¹² samples aborts.
  Reserve with `allocation::try_reserve` after `unit_len`.
- [ ] `reader/image_reader.rs:248-249`, `:218`, `reader/mod.rs:373-382`: `words`, the
  stream `filled` path and `read_groups` zero-fill the declared size before the range
  check. Call `unit_len` in `ImageReader::new` and `read_groups` first.
- [ ] `reader/source.rs:143-145`: `StreamSource::slice` grows the staging buffer before
  `check_range`, so a bogus size keeps its capacity, inflates `staging_capacity()` and
  reports `DataUnitTooLarge` for what is `UnexpectedEof`. Check the range first.
- [ ] `compression/decode/tile_index.rs:37`: `try_reserve_exact(main_len)` runs before
  `read_rows` checks the range. Drop the pre-reserve; `read_append` checks, then reserves.
- [ ] `reader/data_source.rs:155-158`, `:199-201`: `tile_rows` and `tiled_table` grow
  scratch to a header-derived length before the read. Bound each by `unit_len`.
- [ ] `bintable/column_reader/mod.rs:152-216`, `endian.rs` `decode_be_cells_into`: the
  `vla*` reads grow the output per cell, unbounded. 10⁶ overlapping `1PB` descriptors over
  one 1 MB array (a 9 MB file) decode to 1 TB, then abort. Sum the counts in one
  descriptor pass, `try_reserve_exact`, fail `DataUnitTooLarge`.
- [ ] `bintable/selection.rs:281-303`: `TableSelection::copy_into` copies each row's heap
  span with no total bound, and `copied_len` exists only under `compression`. Size every
  copy through `copied_len`.

### Batch 2 — Header, table and editor correctness

Files: `header_model/card/mod.rs`, `header_model/keywords.rs`, `ascii/mod.rs`,
`bintable/element_kind.rs`, `column.rs`, `bintable/column.rs`, `bintable/schema/mod.rs`,
`groups/mod.rs`, `fits_editor/mod.rs`.

- [ ] `header_model/card/mod.rs:416-451`: `parse_string` stops at the closing quote and
  ignores the rest of the token, so `KEY = 'abc' 123 / c` reads `"abc"` and drops `123`.
  Only blanks may follow the closing quote; anything else is `Defect::Value`, kept as
  malformed.
- [ ] `ascii/mod.rs:457`: with `decimals == 0`, `field.parse()` accepts `nan`, `inf`,
  `infinity`, which §7.2.5 forbids. Refuse non-finite results, as the header's
  `parse_real` does.
- [ ] `ascii/mod.rs:359-361`, `bintable/element_kind.rs:260`: `tzero + tscale * x` rounds
  twice, against "A physical value is one fused multiply-add". Use
  `tscale.mul_add(x, tzero)`; the imaginary part stays `tscale * im` (§7.3.2).
- [ ] `bintable/column.rs:185-289`, `ascii/mod.rs:114-126, 405-429`: both column types
  flatten name, unit, tscale, tzero, tnull and tdisp and repeat the same `Keyed`
  accessors. Move them into one `ColumnKeywords` in `column.rs`, embedded in both.
- [ ] `column.rs:158-169`: `ColumnLabels::read` uses `get_text(..)?`, so a bad `TTYPEn` or
  `TUNITn` fails `Schema::parse` and `Hdu::parse` keeps no schema: every read of the table
  is refused. Make `name` and `unit` `Keyed<Option<String>>` in `ColumnKeywords`; a
  `ColumnSelector::Name` lookup reports the defect, an index read does not.
- [ ] `header_model/card/mod.rs:517`: `validate_valued_keyword` compares
  `keyword.as_str()` against four strings. Add `END` and `CONTINUE` to `keywords` and test
  the `Keyword` values.
- [ ] `groups/mod.rs:190`: `RandomGroups::parameter` matches `PTYPEn` case-sensitively,
  unlike `ColumnSelector` and `HduSelector`. Use the column rule.
- [ ] `fits_editor/mod.rs:93-101`: `replace_header` writes and flushes before
  `Hdu::parse(out)`; a failed parse leaves the file changed and `self.hdus` stale. Parse
  first, then write.

### Batch 3 — Compressed-data correctness

Files: `compression/decode/tiled_image.rs`, `compression/decode/tile_index.rs`,
`compression/decode/tile_value.rs`, `compression/decode/float_quantization.rs`,
`compression/tile_codec.rs`, `compression/decode/tile_decoder.rs`,
`compression/decode/null_mask.rs`, `compression/convert.rs`,
`compression/encode/mod.rs`.

- [ ] `compression/decode/tiled_image.rs:52-78`: nothing checks the container's `NAXIS2`
  against the tile count (§10.1.2). Fewer rows decode the whole image and then fail in
  `VlaColumn::cell`; more rows pass silently. Check `ntiles == row_count` in
  `TiledImage::new`, a `SizeUnit::Rows` mismatch, as the table side does
  (`table/mod.rs:576-580`).
- [ ] `compression/decode/tile_value.rs:136-145`: `TileColumn::read` flattens every element
  of every cell, so with `TFORM = '2D'` tile 1 reads row 0's second element, and with
  repeat 0 `dequant` falls back silently to scale 1, zero 0. Refuse `repeat != 1` with
  `ColumnKindMismatch`.
- [ ] `compression/tile_codec.rs:196`, `tile_decoder.rs:287`, `encode/mod.rs:141,164`,
  `compression/convert.rs:125-145`: `stream_element() == 2` stands in for the stream's element type,
  and `null_mask.rs:153` checks no element type, so a `1PB` PLIO mask is accepted. Add
  `TileCodec::stream_kind() -> ElementKind` as the one source of the TFORM letter and the
  element size, and one `stream_cell(cell, kind)` check that the mask decode uses too.

### Batch 4 — World-coordinate correctness

Files: `world_coordinates/mod.rs`, `world_coordinates/linear_transform/mod.rs`,
`world_coordinates/projection/perspective.rs`, `world_coordinates/projection/mod.rs`,
`world_coordinates/projection/conic.rs`, `world_coordinates/celestial_transform.rs`,
`world_coordinates/celestial_frame/mod.rs`, `world_coordinates/declared.rs`,
`world_coordinates/table_wcs/mod.rs`.

- [ ] `world_coordinates/mod.rs:224-253`, `:533-536`: a celestial axis with no projection
  code (`CTYPE1 = 'RA'`, `CTYPE2 = 'DEC'`) is paired but not projected, and gets
  `Joint`: `pixel_to_world` drops `CRVAL`, and `axis_world` reaches `.expect("a supported
  celestial axis is projected")` on file data. Plan it `AxisKind::Other` with a linear
  transform about `CRVAL`, as wcslib does.
- [ ] `world_coordinates/linear_transform/mod.rs:315,325`: the singular threshold is
  `n·ε·max|a_ij|` over the whole matrix, so a cube with CDELT1 = 2.8e-8° and CDELT3 =
  2.56e8 Hz is refused as `SingularMatrix` (threshold 2.3e-7). Test each pivot against its
  own row's scale (row equilibration, Higham §9.8), as wcslib's `matinv` does.
- [ ] `world_coordinates/projection/perspective.rs:135-139`: the AZP inverse takes the
  root nearest π/2 without wrapping, so for γ ≠ 0 it refuses points its own forward
  produces (μ = 0.03, γ = −56.46°, (0.97°, 72.11°) gives θ = 180.17°; wcslib's `azpx2s`
  gives −3.28°). Wrap each candidate past 90° by −360° and take the larger, as `azpx2s`
  does.
- [ ] `world_coordinates/mod.rs:279-300`: a `-SIP` code on an alternate description is
  dropped and the pair evaluates as plain TAN, with `distortion()` `None`. Refuse it
  (`ConflictingWcsKeywords`), as the crate refuses what it does not read whole.
- [ ] `world_coordinates/mod.rs:575-610`, `declared.rs:190-196`, `table_wcs/mod.rs:183-191`:
  `write_into` of a table WCS loses the spectral frame and rest value, so `from_header` of
  the result fails `MissingRest`, and `crop` and `bin` fail with it. `Declared::write`
  writes the frame, the rest values and the pole from the parsed model.
- [ ] `world_coordinates/celestial_frame/mod.rs:61-80`: `EPOCH` is not read when `EQUINOX`
  is absent, so `EPOCH = 1950` resolves to ICRS. Read it as wcslib's `wcspih` does.
- [ ] `world_coordinates/celestial_transform.rs:93-119`: `LONPOLE` and `LATPOLE` take
  precedence over `PVi_3`/`PVi_4`, the reverse of wcslib, and `PVi_0` (the fiducial
  offset) is not read. Let the `PV` values win and apply the (x₀, y₀) offset when
  `PVi_0 ≠ 0`.
- [ ] `world_coordinates/linear_transform/mod.rs:133-145`: any `CROTAi` beside `PC` is
  `PcAndCrota`, so `CROTA1 = 0` or a `CROTA3` refuses files astropy reads, while `CD`
  beside a `CROTA` reads `CD` with no report. Follow design.md "Two linear forms that
  disagree are refused": `PC` or `CD` beside a nonzero latitude-axis `CROTA` is a
  conflict (a `CdAndCrota` case beside `PcAndCrota`); any other `CROTA` is not read.
- [ ] `world_coordinates/projection/mod.rs:220`, `projection/conic.rs:52-56`,
  `celestial_transform.rs:90`: CEA takes λ > 1, and COE with η = 90° gives NaN at every
  point. A zero cone is "unsupported axes" while other bad parameters are `InvalidWcs`.
  Check 0 < λ ≤ 1 and the COE cone, and give every bad parameter `InvalidWcs`.
- [ ] `linear_transform/mod.rs:186`, `perspective.rs:85, 146-151`: CROTA, AZP γ and SZP
  φc/θc use `(x * D2R).cos()`, so CROTA2 = 90° writes a 6.1e-17 cross term. Use
  `angle::sind`/`cosd`.
- [ ] `world_coordinates/linear_transform/mod.rs:187-194`: a CROTA header does not survive
  `write_into` bit for bit, as the parse and the re-read form different products. Build
  `values` as `cdelt[row]·pc` in every branch.

### Batch 5 — Interop reads, as design.md now decides

Each decision cites `docs/research/2026-10-25-review-interop.md`.

- [ ] `ascii/mod.rs:446-457`: the implied decimal point is skipped when an exponent is
  present (`1E5` in `E10.2` reads 100000), and the comment says wrongly that cfitsio and
  astropy agree. Follow "An ASCII real field with no point takes its implied point, with
  or without an exponent": divide the mantissa by 10^d whenever it has no `.`, then apply
  the exponent (1000). Rewrite the comment to cite the decision.
- [ ] `compression/decode/tiled_image.rs:58-62`, `encode/mod.rs:69`: HCOMPRESS refuses
  `ZNAXIS != 2` on read and write, so fpack's cubes do not open. Follow "HCOMPRESS_1 takes
  a tile of any rank with exactly two extents above 1": the codec gets the first such
  extent as `nx` and the second as `ny`; a tile of another shape stays
  `HcompressDimensions`. Add a cube case to the round-trip tests.
- [ ] `reader/hdus.rs:45, 177`: a missing `END` or a bad header after the primary fails the
  whole open. Follow "A bad header after the primary ends the HDU list, and the reader
  keeps its error": the scan stops there and stores the error, a
  `FitsReader::scan_defect() -> Option<&FitsError>` gives it, and a select of that index
  returns it in place of `HduNotFound`. The editor's `append` keeps refusing such a file,
  as it does not end at its last HDU.

### Batch 6 — The reader owns its tile indexes; one image dispatch

Needs batch 1 (`unit_len`). Files: `reader/mod.rs`, `reader/image_reader.rs`,
`reader/data_source.rs`, `compression/decode/tile_indexes.rs`,
`compression/decode/tile_index.rs`, `compression/decode/tiled_image.rs`, a new
`reader/plain_read.rs`.

- [ ] `reader/mod.rs:57-76`, `tile_indexes.rs:202-233`, `tile_index.rs:82`: the tile-index
  cache sits in `Scratch`, keyed by a copy of the data offset, and is re-found with a
  linear `find` plus `expect` at `image_reader.rs:353, 401, 601, 740, 832`. Make it a
  `FitsReader` field, `Vec<Option<TileIndex>>` indexed by HDU index; `load` returns
  `&TileIndex`, which the handle holds. Delete `key`, `get` and the expects.
- [ ] `reader/image_reader.rs:397-431`, `:597-635`, `:433-482`, `:638-668`, `:815`: `read`
  and `view_region` repeat the compressed dispatch; the plain path builds `StoredRuns` and
  `PlainRead` three times through `ImageUnit`, which names no concept; `tiles()` takes six
  parameters to split a borrow. One private method builds the plain read, one yields the
  tiles; delete `ImageUnit` and `tiles()`.
- [ ] `reader/image_reader.rs`: `PlainRead` and `StoredRuns` serve `read_groups` too. Move
  them to `reader/plain_read.rs`.
- [ ] `compression/decode/tiled_image.rs:158-216, 273-293`: six decode fns for two
  operations, each repeating the sizing and zero-length guard, under the words "all",
  "section" and "region". One `decode_into(&self, table, index, section:
  Option<TileSection>, out: DecodeBuffer)`; "section" everywhere.
- [ ] `reader/data_source.rs:61`: `resident` returns `Option<Result<&[u8]>>`, which callers
  test as `resident(0, 0).is_some()` five times. Split it into `is_resident() -> bool` and
  `resident_range(offset, len) -> Result<&[u8]>` through `check_range`.
- [ ] `reader/data_source.rs:129`: `DataSource::table` takes a `sum` that its one caller
  passes as `None`. Drop it.
- [ ] `reader/mod.rs`: `Scratch` and its fields are `pub(crate)`, used only in `reader`.
  Make them private.

### Batch 7 — The HDU record moves to `hdu`; the editor borrows it

Files: `hdu/`, `reader/hdu.rs`, `reader/hdus.rs`, `reader/hdu_selector.rs`,
`fits_editor/mod.rs`, `fits_writer/mod.rs`, `reader/data_checksum.rs`,
`reader/data_source.rs`.

- [ ] `reader/hdu.rs`, `reader/hdus.rs`, `reader/hdu_selector.rs`: the record, the list and
  the selector serve the reader and the editor, but live in `reader`, so
  `fits_editor/mod.rs:15-18` imports reader internals. Move them into `hdu/`.
- [ ] `reader/hdu.rs:81-83`, `fits_editor/mod.rs:170-178`, `fits_writer/mod.rs:145,
  218-221`: `HduRole` is a coarser `HduKind`, and each site runs `from_header`,
  `classify`, `data_extent`; `write_raw_hdu` runs it, then `finish_hdu` again. Replace
  them with `HduKind::from_header(header, position)` and `data_extent(header, kind) ->
  Result<u64>`; delete `DataExtent`, whose invariant `padded_data_bytes` re-derives behind
  an `expect`.
- [ ] `fits_editor/mod.rs:32-40, 154, 241`: `Placement` copies `Hdu` fields that a split
  borrow of `self.file` and `self.hdus` already gives, and `placement_of` and `append`
  re-derive `padded_len` with error paths that cannot fire. Pass `&Hdu`; delete
  `Placement`.
- [ ] `fits_editor/mod.rs:192-213`, `reader/data_checksum.rs:31`,
  `reader/data_source.rs:84`: three copies of the chunked read-and-sum, with three
  `1 << 20` constants. One `DataChecksum::sum(self, &mut DataSource) -> Result<u32>`
  that `finish` and the editor call, one constant.
- [ ] `reader/hdus.rs:29, 185, 205`: the scan fetches each block into the threaded
  staging buffer and then copies it into `bytes`, two copies per block from a stream, and
  the editor passes `&mut Vec::new()`. `read_append(offset, BLOCK_SIZE, &mut bytes)`;
  drop the parameter.

### Batch 8 — Reads reuse their buffers and decoders

Files: `compression/gzip.rs`, `compression/table/mod.rs`, `reader/mod.rs`,
`reader/image_reader.rs`, `reader/source.rs`.

- [ ] `compression/gzip.rs:128-131`: each gzip encode builds a `GzEncoder`, about 250 KB of
  deflate state allocated and zeroed, per image tile, per table tile, per variable-length
  array and per descriptor block: 1 GB for a 6000×4000 i16 frame in row tiles. Keep a
  `flate2::Compress` in `GzipScratch`, `reset()` per tile, write the member header and
  trailer by hand as `member_body` reads them, and deflate into the stream directly.
- [ ] `compression/table/mod.rs:640-700, 804-838`: `TiledTable::decode` allocates `out`,
  `cells` and a `CodecScratch` per rayon split on every call, reserves no codec buffer, and
  copies the restored heap three times (peak ≈ main + 2× heap); `place_descriptors`
  collects a `Vec<&ColMeta>` per tile. Decode into a reader-owned buffer, size `out` once
  from the decoded descriptor blocks, write each tile's heap slice in place, compute the
  array-column list once.
- [ ] `compression/table/mod.rs:598`, `reader/mod.rs:446`: `has_arrays` ignores the
  selection, so a few rows of fixed columns from a table with any array column read the
  whole container. `has_arrays(columns)`, true only for a selected array column.
- [ ] `reader/mod.rs:271-283`: `read_table_cell` on a compressed table allocates a column
  mask and re-reads and re-decodes the container on every call; a loop over n cells
  inflates it n times. Keep the last decoded tile, keyed by HDU and tile range, on the
  reader, and reuse `scratch.column_mask`.
- [ ] `reader/image_reader.rs:461-467`: a compressed `physical_into` decodes into
  `scratch.words`, which the reader keeps (96 MB after a 24 MP f32 read, uncounted by
  `view_bytes`), the staging design refuses. Convert tile by tile into `out`.
- [ ] `reader/image_reader.rs:248`: `words.clear()` before `try_resize` zeroes the whole
  region on every view, 96 MB of extra writes per 24 MP f32 view. Drop the `clear()`; the
  fill writes every word.
- [ ] `reader/source.rs:116-118, 155-157`: `StreamSource` seeks with `SeekFrom::Start`,
  which discards a `BufReader`'s buffer on every hop. Use `seek_relative` from the known
  position.

### Batch 9 — The writer streams a binary table

Files: `fits_writer/mod.rs`, `fits_writer/builder.rs`, `fits_writer/image_writer.rs`.

- [ ] `fits_writer/mod.rs:401-412`, `builder.rs:366-384`: `Builder::encode` stages the
  whole data unit in the writer's scratch, which keeps that capacity, against "A data unit
  is written from a typed body and never staged". A `Body::Table(&Builder)` encodes bands
  of rows into the 1 MiB chunk, as `Body::Samples` does; the heap follows the bands.
- [ ] `fits_writer/image_writer.rs:29, 42`: `write_template` and `stream_template` are free
  fns over `&mut FitsWriter` that the two methods could call through directly. Inline them
  into the methods.

### Batch 10 — World-coordinate and time structure

Needs batch 4. Files: `world_coordinates/mod.rs`, `world_coordinates/axis_transform/`,
`world_coordinates/projected_celestial_axes.rs`, `world_coordinates/celestial_frame/`,
`world_coordinates/table_wcs/mod.rs`, `world_coordinates/header_read.rs`,
`world_coordinates/declared.rs`, `world_coordinates/world_unit.rs`,
`world_coordinates/celestial_pole.rs`, `world_coordinates/native_rotation.rs`,
`world_coordinates/tabular/mod.rs`, `world_coordinates/distortion/tpv.rs`,
`reader/mod.rs`, `time_coordinates/frame/mod.rs`, `time_coordinates/bounds/mod.rs`.

- [ ] `world_coordinates/mod.rs:175-265`, `axis_transform/mod.rs:54`,
  `projected_celestial_axes.rs:36, 68`, `celestial_frame/mod.rs:69`,
  `table_wcs/mod.rs:189`: each axis's `CTYPE` is parsed 5–7 times, against "parsed once".
  Parse a `Vec<Ctype>` at the top of `from_header_with_context` and pass `Ctype` values;
  `Frame::time_axis` takes the parsed form.
- [ ] `linear_transform/mod.rs:83-89`, `declared.rs:313-318`, `header_read.rs:80-82`,
  `table_wcs/mod.rs:275`: four keyword-index parsers, one lax (`CTYPE0009` is axis 9). One
  canonical `header_read::index`.
- [ ] `axis_transform/mod.rs:20` ↔ `world_unit.rs:5`: cut the cycle by moving
  `spectral_unit` into `world_unit.rs`. `celestial_pole.rs:7` ↔ `native_rotation.rs:7`:
  move `CelestialCoordinate` to its own `celestial_coordinate.rs`.
- [ ] `tabular/mod.rs:3-4, 233-290`: `TabularTransform::from_table` decodes columns, so the
  interpolation math depends on `bintable` and needs a table to test. `from_arrays(..)`
  takes plain arrays; `reader::read_wcs` reads the columns.
- [ ] `tabular/mod.rs:482-483, 715`: a coupled `-TAB` transform allocates two `Vec`s per
  forward point and three per inverse point, against "A transform … allocates nothing".
  Cap the table-axis count at parse and use inline arrays.
- [ ] `distortion/tpv.rs:86`: `forward` evaluates the value and both partials over 40 terms
  and keeps the value, about 3× the multiplies per point. A value-only evaluation.
- [ ] `time_coordinates/frame/mod.rs:90-92, 117-128`: `relative_to_mjd` rebuilds an `Axis`
  per value, cloning a `Scale` that allocates for `TT(TAI)`: 10⁶ allocations for a 10⁶-row
  column. `Frame::relative_axis() -> Result<Axis>`, called once; drop `relative_to_mjd`.
- [ ] `time_coordinates/bounds/mod.rs:189-196`: `Bounds` reads `MJD-BEG`/`DATE-BEG` but not
  `TSTART`/`TSTOP`, the third Table 35 form. Read all three.

### Batch 11 — Codec shape and compression simplification

Needs batch 3 (`stream_kind`). Files: `compression/rice.rs`, `compression/plio.rs`,
`compression/nocompress.rs`, `compression/gzip.rs`, `compression/hcompress.rs`,
`compression/tile_codec.rs`, `compression/table/mod.rs`, `compression/quantize/mod.rs`.

- [ ] `rice.rs:43`, `plio.rs:24, 136`, `nocompress.rs:5`: rice and PLIO decode, PLIO and
  NOCOMPRESS encode take the caller's buffer, against "A codec lends its output from its
  scratch", and the count is named `nx`, `npix`, `expected`, `tile`. One shape,
  `decode(stream, tile, scratch) -> Result<&mut Vec<i64>>`, `encode(samples, tile,
  scratch) -> Result<&[u8]>`; NOCOMPRESS lends `raw`, which removes its three copies.
- [ ] `compression/table/mod.rs:490, 787`: `TiledTable.data_len` is read only as
  `data_len - nrows * naxis1`. Store `heap_len`.
- [ ] `compression/quantize/mod.rs:27-42`: the dither table is a `static OnceLock<Vec<f32>>`
  with a `debug_assert!` of the final seed. A `const RANDOM: [f32; N_RANDOM]` built by a
  `const fn`, with a `const` assert of seed 1043618065.
- [ ] `compression/table/mod.rs:227-475`: `compress_table` holds the codec gating, a
  60-line closure, the descriptor-block pass and the header, and `:451-454` writes `ZFORMn`
  from `get_text(TFORM).unwrap_or("")`, hiding an invariant. Split the array-tile encode,
  the block encode and the header into methods; `.expect("a parsed schema's TFORMn")`.

### Batch 12 — Public API symmetry

Files: `bintable/column_reader/mod.rs`, `ascii/mod.rs`, `image_model/view.rs`,
`image_model/unsigned_data.rs`, `image_model/unsigned_view.rs`, `bintable/column_data.rs`,
`bintable/column_view.rs`, `header_model/value.rs`, `header_model/mod.rs`,
`fits_writer/ascii_builder.rs`, `reader/mod.rs`, `time_coordinates/phase_axis/`,
`world_coordinates/spectral_frame/mod.rs`, `world_coordinates/celestial_frame/mod.rs`,
`lib.rs`.

- [ ] `bintable/column_reader/mod.rs:71-216`, `ascii/mod.rs:302, 344`,
  `image_model/view.rs:92`: column reads and `Borrowed::unsigned` return a new buffer per
  call (48 MB per 24 MP u16 frame), against "A physical read appends to the caller's
  `Vec`"; the `_into` forms are `pub(crate)`, and columns have no `f32` output. Publish
  `raw_into`, `physical_into<T: PhysicalSample>`, `vla_into`, `vla_physical_into`,
  `unsigned_into(&mut UnsignedData)`; drop the allocating forms.
- [ ] `bintable/column_data.rs:45-74`, `image_model/sample.rs:315-342`: `Data`/`View` have
  generic `TryFrom` through `Sample`, `ColumnData`/`ColumnView`/`UnsignedView` per-type
  macro impls; `View::slice` is `pub`, the others `pub(crate)`. Generic `TryFrom` through
  `Sample` and a public `slice` on every view.
- [ ] `header_model/value.rs:12-90`, `header_model/mod.rs`: `FitsInteger` has no `FromStr`
  and only `TryFrom<&FitsInteger> for i64`; `Value` has no `as_complex` and `Header` no
  `get_complex`/`require_complex`, though the parser makes complex values. Add them, and
  `TryFrom` for `u64` and `i128`.
- [ ] `ascii/mod.rs:24-31, 114-126, 503-537`, `fits_writer/ascii_builder.rs:52`: the read
  side holds `AsciiKind` plus `width` plus `decimals` and loses the F/E/D letter, the write
  side holds `AsciiFloatFormat`. One public `AsciiTform { Text, Integer, Float }` with
  `FromStr`/`Display`, held by `AsciiColumn` and taken by `AsciiWriteColumn`.
- [ ] `reader/mod.rs:188, 202, 219`: `read_table_rows`, `read_table_columns`,
  `read_table_columns_into` take `Range<usize>`. `impl RangeBounds<usize>`.
- [ ] `time_coordinates/phase_axis/`: `PhaseAxis::from_header(axis)` and
  `from_array_column(axis, ..)` take one-based axes, `Wcs::axis_world` zero-based. Use
  zero-based.
- [ ] `world_coordinates/spectral_frame/mod.rs:51`, `celestial_frame/mod.rs:39`: an unknown
  `SPECSYS` or `RADESYS` fails the whole WCS parse, celestial axes included, while
  `ReferencePosition::Other` and `Scale::Local` keep unknown text. Keep unknown text in an
  `Other` case, as the time enums do.

### Batch 13 — Errors as cases

Needs batches 1–3, which add cases. Files: `fits_error.rs`, `bintable/tdim/mod.rs`,
`column.rs`, `ascii/mod.rs`, `image_model/scaling.rs`, `bintable/descriptor/mod.rs`,
`bintable/selection.rs`, `bintable/vla_column.rs`.

- [ ] `fits_error.rs:933`: `KeywordOutOfRange { keyword }` (50 sites) covers a TDIM syntax
  error, a TDIM past its cell, a table whose BITPIX/NAXIS/GCOUNT is not 8/2/1, TBCOL past
  the row, a non-finite BSCALE and a BLANK outside its type, with no value and no reason.
  Add a defect enum (syntax, range, structure) to the variant.
- [ ] `bintable/descriptor/mod.rs:145`, `bintable/selection.rs:122, 127`: a descriptor past
  the heap is `UnexpectedEof`, the same as a truncated file. A `DescriptorPastHeap {
  column, row }` case.
- [ ] `bintable/vla_column.rs:42`: `row >= row_count` is a caller logic error returned as
  `UnexpectedEof`. A `debug_assert!`.
- [ ] `fits_error.rs:849`: `InvalidAsciiField { field }` carries no column or row, unlike
  its siblings. Add them.

### Batch 14 — Module cycles and duplicated name types

Files: `bitpix/`, `words/`, `image_model/`, `keyword.rs`,
`header_model/commentary_name.rs`, and every `use crate::bitpix` / `use crate::words`.

- [ ] `bitpix/mod.rs:3-5` ↔ `image_model`: `Bitpix` carries the image decode dispatch
  (`extend_physical_be`, `view_words`) and is published as `image::Bitpix`. Move it to
  `image_model/bitpix.rs`; `words/mod.rs:13` ↔ `image_model` likewise to
  `image_model/words.rs`.
- [ ] `keyword.rs:22-86`, `header_model/commentary_name.rs:142-191`, `keyword.rs:181-213`:
  `Keyword` and `CommentaryName` repeat the inline 8-byte name (bytes+len, const `checked`,
  `as_str`, Debug/Display, three `TryFrom`s), and `KeyBuf` is a third. One private inline
  name type that takes the byte rule.

### Batch 15 — Docs that read wrong

Files as listed; independent of the other batches.

- [ ] `reader/mod.rs:154-156`: image and random-groups reads no longer "stage through the
  reader's reused internal scratch".
- [ ] `reader/image_reader.rs:381-384`: `view` with `BITPIX = 8` from a stream fills
  `words`, not "the reader's scratch".
- [ ] `reader/source.rs:5-8, 86-88`: the "copies the range into the reader's scratch
  first" text ignores `read_into` and `read_append`.
- [ ] `ascii/mod.rs:37`: `AsciiColumnData::Text` does not keep the padding;
  `ascii_text.rs:53` drops the trailing blanks.
- [ ] `image_model/unsigned_data.rs:106-111`: `vla_unsigned` returns one `Ragged`, not an
  `UnsignedData` per row.
- [ ] `compression/decode/null_mask.rs:118`: the first doc line of `capacities` belongs to
  `decode`.
- [ ] `compression/table/mod.rs:39`: "Per-column compression algorithm (`ZCTYPn`)" sits
  above `ColMeta`.
- [ ] `compression/decode/float_quantization.rs:219-222`: two overlapping doc sentences on
  `Lossless`.
- [ ] `world_coordinates/mod.rs:737-738`: an orphan doc comment fused onto the `internals`
  module doc.

# Part 2 — Test suite

Scope: the test code of `fits-well/` — every `tests.rs` and inline `mod tests`, `src/internals/`,
the `mod internals` reach-ins, and `tests/`. Paths are relative to `fits-well/`. Part 1 covers the
production code.

Measured on 2026-10-25: 499 tests (1 ignored), 19 400 lines of test files, 0.31 s for the whole
nextest run. Speed is not a problem: the heaviest tests are `compression/encode/tests.rs:1027`
(4.2 M u8 samples, the parallel wave boundary) and `world_coordinates/tabular/tests.rs:140` (a
513² grid), and both stay inside the budget. The plan keeps every test fast. It does not trade
coverage for time.

## Proposal

### Root causes

1. **The harness has no single front door.** `src/internals/` is good: `Fixture`, `memory`,
   `cards`, `samples`, `tolerance`, `rng`, `wcs_header`, `container`. But each module covers only
   part of its domain, so tests fill the gaps by hand. There are about 40 hand-built
   `FitsWriter::new(Cursor::new(Vec::new()))` writers, about 59 `Header::new()` +
   `set_internal` chains, 68 `to_be_bytes` calls that lay out rows, descriptors and heaps,
   ≈54 ad-hoc `(a - b).abs() <= …` float compares, and four `be` helpers.
2. **A written file and a fixture file are opened differently.** `Fixture` gives `.slice()` and
   `.stream()`. A written file gives `round_trip` (stream only), or `written` followed by a
   reader picked at random: `reader/tests.rs` uses `from_bytes` 28× and `open(Cursor)` 14×,
   with no reason given. Nothing runs one property on both sources. So the slice/stream symmetry
   that the crate promises is tested in some places only. For example, fetched-byte counts exist
   for slice sources alone.
3. **Error assertions hide the actual value.** 383 `assert!(matches!(…))` print only
   `assertion failed` on a failure. About 25 more match a variant with `{ .. }` where the fields
   can be computed. `std::assert_matches!` is stable on the stable toolchain that CI installs
   (rustc 1.99.1 here, checked) and prints the left value.
4. **The same property is written as N near-copies, not as one sweep.** The coverage matrix has
   holes with no visible reason: codec × `BITPIX` × tiling, ASCII vs binary schema, SIP vs TPV,
   forward vs inverse, image vs table decode, slice vs stream.
5. **The file layout does not follow the code.** Four test files exceed 1 100 lines
   (`reader` 1901, `fits_writer` 1848, `world_coordinates/projection` 1290, `compression/encode`
   1112). About 15 tests sit in the file of another module. Two inline mods exceed 150 lines
   (`gzip.rs` 161, `hcompress.rs` 154).
6. **No naming rule.** There are seven suffixes for one meaning (`rejects_`, `_rejected`,
   `_refused`, `_is_an_error`, `_errors`, `returns_an_error`, `reject`), and module prefixes
   that repeat the path (`ascii_`, `table_wcs_`, `tabular_`, `hcompress_`). Helpers are called
   bare, aliased (`from_card_lines as header`, then shadowed at `fits_writer/tests.rs:1423`) or
   module-qualified.

### Bugs in tests

- `src/compression/table/tests.rs:759`: `if keyword == keyword`. The pattern binding shadows the
  loop variable, so the guard is always true and the test accepts any keyword.
- `src/world_coordinates/tests.rs:573` `assert_same_transform`: its doc promises "the same
  refusal", but it compares `.ok()`. Two different errors therefore compare equal.

### Target design

- **`internals` is the one harness, one module per domain, called module-qualified.** A test
  writes `memory::written(…)`, `cards::header_of(…)`, `samples::ramp(…)`,
  `tolerance::assert_close(…)`. It never imports a helper bare or renames it. The module name
  says where a helper lives, and two `ramp`s cannot collide. Refused: a prelude glob, which hides
  the home of each helper.
- **A file in memory and a file on disk have one shape.** `memory::written(…)` returns
  `memory::Written(Vec<u8>)`. `Written` and `Fixture` both have `bytes()`, `slice()` and
  `stream()`, and both readers are the same types (`SliceReader<'_>`,
  `StreamReader<Cursor<&[u8]>>`). `round_trip` and `round_trip_by` go away: they are
  `written(…).stream()`. `rejected_before_output` also asserts that the writer state is still
  `Empty`, which is why `fits_writer/tests.rs:1429` writes it out by hand.
- **The reader rule.** Use `slice()` by default. Use `stream()` only where the test pins
  streaming behavior (staging, seeks, `finish_bytes`). A property that both sources must hold is
  written once as `fn check<S: Source>(reader: &mut FitsReader<S>)` and called with both. Refused:
  an enum over both readers or a macro. A generic fn is the plain Rust form and needs no
  dispatch.
- **`assert_matches!` for every pattern assertion**, with the fields named wherever a test can
  compute them. Refused: a local macro (std has one), and `PartialEq` on `FitsError` (it holds an
  `io::Error`).
- **Every float compare goes through `Tolerance`**, which carries its reason. A new scalar
  `tolerance::assert_within(got, want, tolerance, context)` covers the four `&[x]` wrappers and
  the scalar sites. `assert_astropy_golden` and `assert_near` (`axis_transform/tests.rs:85`)
  delegate to it.
- **`samples` owns sample data and its bytes.** It holds `ramp`, the bit compares, and the
  big-endian encoding: `Data::encode` beside the `Data::decode` reach-in, and `samples::be` moved
  from `container.rs`, ungated. A slice form of `assert_same_bits` and of `nan_blind_bits` (over
  `f32`/`f64`/ints) replaces the hand loops. A test builds an `Image` with `Image::new`, which
  checks the shape. A struct literal skips that check.
- **One raw-table builder.** `internals::raw_table::RawTable` holds cells (fixed bytes, or a
  `P`/`Q` descriptor whose bytes go to the heap) and produces the header (`TFORMn`, `NAXIS1`,
  `PCOUNT`, `TTYPEn`) and the data unit. `OneTile` becomes `RawTable` plus the `Z*` keywords.
  `tabular::internals::lookup_*` and the descriptor layouts written by hand in `column_reader`,
  `tdim`, `bit_column`, `groups` and `compression/table` all use it. `bintable::internals::table_header`
  stays the zero-cell form.
- **One instrumented source.** `reader::source::internals` holds `Counting<R>`, which records the
  byte range of every fetch and the seek count, over a cursor and over a slice. `fetched()` sums
  the ranges. It replaces `CountingCursor` (`reader/tests.rs:51`), `Counted` (`source.rs:316`)
  and `CountingSource`/`CountedReader`, and gives fetch counts on both sources.
- **Headers through `cards` and `wcs_header`.** `cards::header_of` for keyword lists.
  `WcsHeader::with(&[(&str, Value)])` and a ctype-prefix field replace the six local
  "WcsHeader plus extra keywords" helpers. `Family::key(root, axis, alt)` spells the Table 22
  forms once, for the frame, spectral, table and phase-axis tests. `cards::value_field(bytes,
  keyword) -> Range<usize>` replaces the hard-coded card offsets. `cards` also gets
  `ChecksumReport` constants beside `VALID`.
- **Integration tests: one binary per piece of global state.** A global allocator and a rayon
  pool size force a separate binary, and so does a peak count, which counts every thread but
  the harness's: a second test in its binary would add its allocations to the peak under
  `cargo test`, which runs the tests of a binary on threads. So `tests/api/` holds
  `round_trip`, `examples` and `cross_check` as modules of one binary; `view_bytes.rs`,
  `view_bytes_parallel.rs` and `wcs_allocations.rs` stay alone. One counting allocator in
  `tests/support/counting.rs` counts calls per thread and live/peak bytes for all three. This
  gives 4 binaries instead of 6, and one allocator instead of two. Refused: one binary for the
  allocation tests, for the reason above.
- **Test files follow the code.** One test file covers one module. A test file past 150 lines or
  40 % of its module is a `tests.rs`. A `tests.rs` past 600 lines becomes `tests/mod.rs` plus
  one file per topic. One `#[test]` covers one property, and a test past 80 lines splits. Both
  limits are a review threshold, not a measured optimum: they catch the files and tests that
  Batch 9 lists, and nothing that reads well today.
- **Naming rule.** A test name states the behavior: subject first, present tense, no leading
  article, and no prefix that repeats the module path. A refusal is `…_refuses_…` (or
  `…_is_refused` when the subject is the input), which matches "Refused" in `docs/design.md`.
  A name covers everything the test checks, or the test splits.
- **Sweeps over the full matrix.** A property that holds for N siblings is one table-driven
  test over all N. A sibling that it excludes appears in the table with the reason, for
  example `HCOMPRESS × row tiles: refused, a tile needs two extents above 1` (the rule of Part 1
  Batch 5).

## Plan

Each batch leaves `cargo drive gate` green in `fits-well/`. Each batch follows the batches it
lists as dependencies.

### Batch 1 — Bugs and error assertions

Files: `src/compression/table/tests.rs`, `src/world_coordinates/tests.rs`, every test file
(mechanical edit).

- [ ] `src/compression/table/tests.rs:759`: the guard `keyword == keyword` is always true. Bind
  the field as `keyword: got` and compare `got == keyword`.
- [ ] `src/world_coordinates/tests.rs:573` `assert_same_transform`: `.ok()` makes two different
  refusals equal. Compare `Ok` values exactly, and compare `Err` values by their `Debug` text,
  which is the same text for the same refusal.
- [ ] All 383 `assert!(matches!(…))` (count from `rg 'assert!\(matches!\('`): use
  `assert_matches!`. Keep the guards.
- [ ] Variants matched with `{ .. }` where the fields can be computed: `ConflictingWcsKeywords`
  (`world_coordinates/tests.rs:742`, `projection/tests.rs:762`, `linear_transform/tests.rs:89,
  97`, `distortion/tests.rs:70`, `celestial_frame/tests.rs:154`), `WcsInvalidPole`
  (`world_coordinates/tests.rs:364`), `ColumnNotFound` (`ascii/tests.rs:102`,
  `column_reader/tests.rs:104`), `InvalidTform` (`tform/tests.rs:267`), `RowWidthMismatch`
  (`schema/tests.rs:200`), `InvalidValue`/`InvalidAscii` (`header_model/tests.rs:750, 797, 801`;
  `reader/tests.rs:487, 510, 531`), `ValueTypeMismatch` (`reader/tests.rs:905`,
  `tabular/tests.rs:393`), `IntegerOutOfRange` (`tabular/tests.rs:394`), `HduNotFound`
  (`reader/tests.rs:909`), `GroupParameterNotFound` (`groups/tests.rs:339`), `InvalidUnit`
  (`time_coordinates/frame/tests.rs:160`, `axis_transform/tests.rs:568`), `DataUnitTooLarge`
  (`decode/tests.rs:628`), `UnsupportedCompression` (`plio.rs:333`), `Hcompress`
  (`tile_decoder.rs:344`). Name each field.
- [ ] `hcompress.rs:1813` accepts `UnexpectedEof | CorruptCompressedData { .. }`. Pin the one
  error that the input produces.
- [ ] Booleans in place of results: `ascii/tests.rs:158` (`raw().is_err() == bool`),
  `descriptor/tests.rs:184` (`validate().is_ok() == bool`), `tile_decoder.rs:356` (`is_ok()`),
  `quantize/tests.rs:137` (`is_some()`). Assert the value or the variant.
- [ ] `tabular/tests.rs:371` and `keyword.rs:302` use `catch_unwind`. A panic already fails a
  test. Drop the wrapper at `tabular:371`, and use `#[should_panic(expected = …)]` at
  `keyword.rs:302`, as `keyword.rs:285` does.
- [ ] Typed names compared through `to_string()`: `header_model/tests.rs:572–648` (8 sites),
  `card/tests.rs:398, 519`, `tile_decoder.rs:363`. Compare the `RecordName` or `Keyword` value.
- [ ] `fits_writer/tests.rs:1171` compares `offset.to_string()`. Compare the integer.

### Batch 2 — One shape for a file in memory

Files: `src/internals/memory.rs`, `src/internals/fixture.rs`, and the callers in
`src/reader/tests.rs`, `src/fits_writer/tests.rs`, `src/fits_editor/tests.rs`,
`src/compression/{encode,table}/tests.rs`, `src/ascii/tests.rs`.

- [ ] `memory.rs`: `written`/`written_by` return `Written(Vec<u8>)`, with `bytes()`, `slice()`
  and `stream()` as on `Fixture`. Delete `round_trip` and `round_trip_by`, and change their 45
  call sites to `written(…).stream()`. `read_whole` stays generic over `S: Source`.
- [ ] `memory.rs` `rejected_before_output`: also assert that the writer state is `Empty`. The
  `state` field is private, so add a `FitsWriter::is_empty` reach-in in `fits_writer`'s
  `internals`, gated `feature = "internals"` as `memory` is. Replace the hand copy at
  `fits_writer/tests.rs:1429–1439`.
- [ ] Hand-built writers (about 40): `reader/tests.rs:120, 196, 265, 921`;
  `fits_writer/tests.rs:243, 384, 502, 727, 1048, 1369, 1504, 1534` → `written`;
  `fits_writer/tests.rs:448, 960, 990, 1701, 1717, 1800, 1825` → `memory_writer()` kept for its
  state; `fits_editor/tests.rs:35, 132, 164, 207` `three_hdus` → `written_by`;
  `compression/encode/tests.rs:290, 315, 541, 742` and `compression/table/tests.rs:109` →
  `rejected_before_output`; `encode/tests.rs:456`, `table/tests.rs:898` → `written`;
  `ascii/tests.rs:489, 696` → `written`.
- [ ] `FitsReader::open(Cursor::new(written(…)))` at `fits_writer/tests.rs:1132, 1148, 1275`, and
  `written_table_header` (`fits_writer/tests.rs:242`): use `written(…).stream()`.
- [ ] `compression/table/tests.rs:441, 914` and `reader/tests.rs:168` each write an empty primary
  plus `write_raw_hdu` by hand. Add `memory::raw_hdu_file(header, data) -> Written`.
- [ ] Apply the reader rule in `reader/tests.rs` (28 `from_bytes`, 14 `open`) and
  `fits_writer/tests.rs` (7 / 17): `slice()` unless the test pins streaming. Where a property
  holds for both sources, write a `fn check<S: Source>` and call it on both. Candidates: the
  whole-view tests (`reader/tests.rs:1071, 1090`) and the fetch counts (Batch 7).
- [ ] `tests/round_trip.rs:20` `file()`: make the closure return `Result` as `written` does, and
  add `file_by(writer, …)` for the checksum cases at `:75` and `:230`, which bypass it now.

### Batch 3 — Float tolerances

Files: `src/internals/tolerance.rs`, `src/world_coordinates/**/tests.rs`,
`src/world_coordinates/mod.rs` (internals), `src/world_coordinates/projection/{equatorial,perspective}.rs`,
`src/time_coordinates/frame/tests.rs`.

- [ ] `tolerance.rs`: add the scalar `assert_within`. Use it at the four `&[x]` sites
  (`world_coordinates/tests.rs:696, 835`; `projection/tests.rs:610, 1049`).
- [ ] Ad-hoc compares (≈54): `projection/tests.rs` 17, `axis_transform/tests.rs` 12,
  `distortion/tests.rs` 10, `world_coordinates/tests.rs` 3, `spectral_frame/tests.rs` 3,
  `linear_transform/tests.rs` 2, `tabular/tests.rs` 2, `equatorial.rs` 2,
  `perspective.rs` 2, `table_wcs/tests.rs` 1, `time_coordinates/frame/tests.rs` 1. Use
  `Tolerance`. About 20 have no reason now. Derive one for each (ulps of the operation count, or
  the reference library's printed digits), as `spectral_frame/tests.rs:15` does.
- [ ] `world_coordinates/mod.rs` `assert_astropy_golden` and `axis_transform/tests.rs:85`
  `assert_near`: delegate to `Tolerance`.
- [ ] `projection/tests.rs:1158` accepts any finite result. Pin the value with a tolerance.

### Batch 4 — Sample data and bytes

Files: `src/internals/{samples,container}.rs`, `src/image_model/data.rs` (internals),
`src/compression/mod.rs` (internals), and the callers in `src/image_model/tests.rs`,
`src/compression/**`, `src/groups/tests.rs`, `src/reader/tests.rs`, `src/bitpix/tests.rs`.

- [ ] Big-endian encoding exists four times: `container.rs:156` `be` (gated on `compression`),
  `plio.rs:340`, `image_model/tests.rs:22` `encoded`, and `groups/tests.rs:136, 169–182, 205,
  243, 269, 293`. Add `Data::encode()` beside `Data::decode`, and move `be` to `samples`
  ungated. `convert.rs:167` `be` is a different operation (i64 through `IntBitpix`): give it a
  name that says so.
- [ ] `compression::internals::ramp` (`compression/mod.rs:336`) collides with `samples::ramp`.
  Rename it after what it is (a 24×16 i16 codec plane), or replace it with `samples::ramp` where
  the values do not matter.
- [ ] Ad-hoc planes: byte planes `encode/tests.rs:36, 691, 1039`; quadratic planes `:370, 790,
  936`; I16 images `reader/tests.rs:1679, 1761, 1806, 1848`; per-index `SplitMix::new(i)` at
  `encode/tests.rs:105` and `quantize/tests.rs:19`. Use `samples::ramp`, or one named generator
  in `samples` for each shape that a codec needs (smooth, noisy, byte).
- [ ] Hand bit/NaN compares: `image_model/tests.rs:377, 393, 410, 547, 573, 592`;
  `decode/tests.rs:315, 334`; `reader/tests.rs:1159, 1297`; `groups/tests.rs:278–308`;
  `ascii/tests.rs:136, 385, 717`; `column_reader/tests.rs:95, 308`. Add slice forms of
  `assert_same_bits` and `nan_blind_bits` and use them.
- [ ] `Image` struct literals: `encode/tests.rs` (9), `fits_writer/tests.rs` (13),
  `reader/tests.rs` (6). Use `Image::new`.
- [ ] `Scaling { bscale, bzero, blank: None }` (24 sites in test files) and `Metadata { shape: &[2], … }`
  (`fits_writer/tests.rs:1773, 1804, 1828`): add `samples::linear(bscale, bzero)` and use the
  local closure at `image_model/tests.rs:613` as the model.
- [ ] Reference-fixture compares are written five ways (`decode/tests.rs:58, 83, 244`;
  `encode/tests.rs:597, 627`; `table/tests.rs:240, 555`). Add one
  `assert_decodes_to(fixture, hdu, &Data)`.
- [ ] The half-step quantization bound is written three times with different rounding terms
  (`encode/tests.rs:143, 253`; `quantize/tests.rs:60`). Add one
  `assert_within_half_step(original, back, zscale, precision)` in `compression::internals`,
  gated `feature = "internals"`: the bench check of Part 3 Batch 2 calls it too.
- [ ] The compressed round trip (`round_trip` + `write_compressed_image` + `read_whole(r, 1)`)
  is repeated at about 16 sites in `encode/tests.rs`, and `decompress_image`
  (`decode/tests.rs:33`) is inlined again at `encode/tests.rs:666`. Move both to
  `compression::internals` as `compressed_round_trip(image, config)` and `decompress_image`.
- [ ] The per-codec constructors (`rice.rs:456–502`, `gzip.rs`, `plio.rs`, `hcompress.rs:1757–1842`):
  one `encode`/`decode` helper in each codec's test mod.

### Batch 5 — Headers and cards

Files: `src/internals/{cards,wcs_header}.rs`, and the callers in `src/world_coordinates/**`,
`src/time_coordinates/**`, `src/reader/tests.rs`, `src/fits_writer/tests.rs`,
`src/checksum/tests.rs`, `src/fits_editor/tests.rs`, `src/header_model/**`, `src/hdu/tests.rs`.

- [ ] `wcs_header.rs`: add `WcsHeader::with(&[(&str, Value)])` and a ctype-prefix field. Replace
  `celestial_with` (`world_coordinates/tests.rs:336`), `transform` (`celestial_transform.rs:182`),
  `image` (`celestial_frame/tests.rs:12`), `build`/`parse` (`projection/tests.rs:633, 667`,
  identical bodies), `base` (`linear_transform/tests.rs:83`), `build`
  (`axis_transform/tests.rs:135`), and the hand-built HPLN/GLON headers
  (`world_coordinates/tests.rs:211`, `projection/tests.rs:750, 764`).
- [ ] `wcs_header.rs`: add `Family::key(root, axis, alt)`, and let `axis_header` take a
  `Family`. Replace the hand spellings at `celestial_frame/tests.rs:88–130`,
  `spectral_frame/tests.rs:110–163`, `table_wcs/tests.rs:186–217`, `phase_axis/tests.rs:11–30`,
  and the WAVE-TAB axis at `reader/tests.rs:108, 226` and `tabular/tests.rs:306`.
- [ ] `Header::new()` + `set_internal` chains (≈44 outside the header-builder tests): use
  `cards::header_of`. Largest sets: `world_coordinates/tests.rs` 11, `time_coordinates/frame` 8
  (plus a closure at `:385` that repeats `header_of`), `table_wcs` 5, `reader/tests.rs:97,
  162, 263, 916`.
- [ ] Card offsets: add `cards::value_field(bytes, keyword)`. Use it at
  `fits_writer/tests.rs:193, 1165, 1203`, `reader/tests.rs:438, 507` and `checksum/tests.rs:84`,
  which also repeats `card_index` by hand.
- [ ] The literal 36 (records per block) at `fits_editor/tests.rs:79, 135, 182`: use
  `BLOCK_SIZE / CARD_SIZE`.
- [ ] `ChecksumReport` literals (`fits_writer/tests.rs:1239, 1243, 1280, 1296, 1310, 1320`) and
  split field asserts (`reader/tests.rs:1707, 1775`; `tests/round_trip.rs:95`): add constants
  beside `cards::VALID`.
- [ ] Render and parse boilerplate: `header_model/tests.rs:337, 353, 753, 922` (render into a
  `Vec`), `card/tests.rs:482, 499` (records + END + parse → `from_card_lines`),
  `card/tests.rs:468` (repeats `record_lines`), `Record::Valued { keyword: Keyword::new(..) }`
  six times in `card/tests.rs`. Add one `card` helper each where the count is 3 or more.
- [ ] Duplicate helpers: `header_model/tests.rs:7 names` = `card/tests.rs:38 name_text`;
  `world_coordinates/tests.rs:76 assert_round_trip` shadows `wcs_header::assert_round_trip`;
  `table_wcs/tests.rs:51 assert_same_world` ≈ `world_coordinates/tests.rs:558
  assert_same_transform`; the `written` closure at `world_coordinates/tests.rs:655` repeats
  `written_back` (`:550`). Keep one of each.
- [ ] `projection/tests.rs:22, 1153, 1171, 1210, 1229, 1245`: `[0.0; 21]` + `copy_from_slice`.
  Use the `raw()` helper beside them.
- [ ] Imports: call every `internals` helper module-qualified. Remove the alias
  `from_card_lines as header` (`fits_writer/tests.rs:34`, `hdu/tests.rs:3`).

### Batch 6 — One raw-table builder

Needs Batch 4 (`samples::be`). Files: new `src/internals/raw_table.rs`,
`src/internals/container.rs`, `src/world_coordinates/tabular/mod.rs` (internals),
`src/bintable/{column_reader,tdim,descriptor,bit_column}/tests.rs`, `src/groups/tests.rs`,
`src/compression/table/tests.rs`, `src/fits_writer/tests.rs`.

- [ ] Add `RawTable`, which builds the cells, the header and the data unit (row + heap).
  `OneTile` keeps only the `Z*` keywords on top of it.
- [ ] `tabular::internals::{lookup_header, lookup_bytes, lookup_table}`: build through `RawTable`.
- [ ] Descriptor and heap layouts written by hand: `column_reader/tests.rs:173, 290, 314, 354,
  403`; `tdim/tests.rs:47, 64, 103`; `bit_column/tests.rs:198`;
  `compression/table/tests.rs:411–432, 859`; `fits_writer/tests.rs:743–755`. Build them with
  `RawTable`, and read descriptors with `PqDescriptor::decode` (as `table/tests.rs:411` does),
  not with byte offsets.
- [ ] `hcompress.rs:1749, 1816, 1827, 1838` and `encode/tests.rs:815`: header fields at 6..10,
  10..14, 14..22, 22. Name the offsets once in `hcompress`'s test mod.

### Batch 7 — One instrumented source

Needs Batch 2. Files: `src/reader/source.rs` (internals and tests), `src/reader/tests.rs`.

- [ ] Replace `CountingCursor` (`reader/tests.rs:51`), `Counted` (`source.rs:316`) and
  `CountingSource`/`CountedReader` (`source.rs:226`) with one `Counting` that records fetch
  ranges and seeks, over a slice and over a cursor.
- [ ] Fetched-byte tests run on slice sources only (`reader/tests.rs:1321, 1690, 1724, 1781,
  1792, 1821, 1889`). Run them on both through `fn check<S: Source>`, and add the partial-read
  fetch case for a plain table beside the compressed one at `:1879`.

### Batch 8 — Integration binaries

Files: `tests/`.

- [ ] Add `tests/support/counting.rs`, one allocator that counts calls per thread and live/peak
  bytes. Delete the allocator in `tests/wcs_allocations.rs:24` and the one in
  `tests/support/peak.rs:20`.
- [ ] Merge `tests/round_trip.rs`, `tests/examples.rs` and `tests/cross_check.rs` into
  `tests/api/main.rs` with one module each.
- [ ] `tests/view_bytes.rs`, `tests/view_bytes_parallel.rs` and `tests/wcs_allocations.rs` use
  the shared allocator and stay one binary each. Update the `cross_check` run command in its
  doc comment.

### Batch 9 — Layout and placement

Do after Batches 2–7 and after Part 1 Batches 6, 7 and 14, so that the moved code and the
modules are in their final form. Files: the test files named below.

- [ ] Move the scan tests of `src/reader/tests.rs` to the tests of `hdu`, where Part 1 Batch 7
  puts `Hdus`. Split the rest into `reader/tests/{mod,image,table,wcs,checksum}.rs`, and
  `src/fits_writer/tests.rs` into `fits_writer/tests/{mod,image,table,checksum,stream,template}.rs`.
- [ ] Split `src/world_coordinates/projection/tests.rs` by kernel file (zenithal, cylindrical,
  conic, cube, HEALPix, polyconic…), to match the source layout, and
  `src/compression/encode/tests.rs` by topic (sweeps, HCOMPRESS, fpack parity).
- [ ] Move the inline mods of `src/compression/gzip.rs` (161 lines) and `hcompress.rs` (154) to
  a `tests.rs`. `rice.rs` (148) moves with them so that the three codecs stay alike.
- [ ] Misplaced tests. Header render/parse/`set`: `reader/tests.rs:483`,
  `fits_writer/tests.rs:1066, 1081` → `header_model`. `Image::into_parts`: `reader/tests.rs:952`
  → `image_model`. `Borrowed::new`, `Ragged::from_rows`: `fits_writer/tests.rs:282, 589` → their
  modules. Checksum statuses and their helpers: `fits_writer/tests.rs:173–199, 1269–1359` →
  `checksum/tests.rs`. `Bitpix::swap_into_words`, `view_words`, `extend_physical_be`:
  `image_model/tests.rs:305, 553` → the tests of `Bitpix`, in `image_model` after Part 1
  Batch 14. `padded_len`: `hdu/tests.rs:66, 73, 170,
  185` → drop (`block/tests.rs` covers it). `norm360`/`norm180`:
  `world_coordinates/tests.rs:521, 532` → `angle.rs`. Error `Display`:
  `fits_writer/tests.rs:1032` → `fits_error.rs`.
- [ ] Oversized tests: `reader/tests.rs:1346` (278 lines), `:849` (127), `:416` (118);
  `header_model/tests.rs:529` (120); the single-test files `spectral_frame` (210),
  `celestial_frame` (145), `phase_axis` (112), `tdim` (five scenarios in 124). Split them, one
  property each.
- [ ] Duplicate tests: `compression/table/tests.rs:226` repeats `:553`; `image_model/tests.rs:733`
  repeats `:49`, and `:111` overlaps `:76`; `:796` belongs inside `:750`; `bitpix/tests.rs:5`
  and `:12` overlap; `reader/tests.rs:638, 683` repeat `hdu/tests.rs:201, 252` through the
  reader. Keep one of each.

### Batch 10 — Sweeps and symmetric coverage

Needs Batches 2, 4 and 5. Each item is one table-driven test.

- [ ] Compressed images: codec × `BITPIXES` × tiling over `samples::ramp`
  (`encode/tests.rs:41–62` has no GZIP_2 u8, i32/i64 only for Rice and HCOMPRESS, 2-D tiles only
  for GZIP_1). PLIO and HCOMPRESS appear with their limits as rows, and HCOMPRESS with the
  cube tiles of Part 1 Batch 5. Merge the HCOMPRESS
  lossless tests `:891` and `:931` into it. Add BSCALE/BZERO/BLANK preservation and empty images
  (`:965`, `:1069`, GZIP only now) for every codec.
- [ ] Null masks: the float mask for every codec (`decode/tests.rs:307` covers NOCOMPRESS only),
  and every mask column spelling (`NULL_PIXEL_MASK_COLUMN` has no test).
- [ ] Image vs table decode: the oversized `try_reserve` path (`decode/tests.rs:617`, images
  only) and "a corrupt untouched tile still reads" (`table/tests.rs:595`, tables only) for both.
  Put the wrong-tile-rank tests (`encode/tests.rs:734`, `compression/tests.rs:106`) together.
- [ ] Table codecs: one `TABLE_CODECS` const in place of the list at `table/tests.rs:84, 365, 794`.
- [ ] `hdu/tests.rs`: one table of (cards, `HduKind`, `Ok(bytes)` or the error), over the
  `data_extent(header, kind)` of Part 1 Batch 7, in place of the six
  `data_extent` success tests (`:55, 69, 118, 151, 174, 189`) and the four error tests.
- [ ] `Fixture` layout: one sweep over `Uit`, `Ddts` and `Iue` in place of
  `reader/tests.rs:283, 319, 348`, with `header_sum` for all three.
- [ ] `HduKindMismatch`: one sweep of accessor × HDU kind in place of `reader/tests.rs:557, 586,
  829` and `groups/tests.rs:219`. Include `ascii_schema`.
- [ ] Unsigned images: one write/read sweep over `from_u16`, `from_u32`, `from_u64` and
  `from_i8` (`fits_writer/tests.rs:1121, 1144, 1161`; `u32` and `i8` have none).
- [ ] Non-finite scaling (`samples::NONFINITE`): binary columns, images, ASCII columns and
  compressed images in one sweep (`fits_writer/tests.rs:880`; `ascii/tests.rs:667` has 2 cases).
- [ ] Single-column binary round trips `fits_writer/tests.rs:555, 786, 913, 1567` → one table.
  Stream-write failure modes `:1764, 1798` → one table; the two identical blocks at `:1701–1726`
  → one loop.
- [ ] ASCII and binary tables: schema `schema/tests.rs:109, 145, 205` with `ascii/tests.rs:183,
  724, 763` as one sweep over both kinds. Add the ASCII TFORM `Display` round trip
  (`tform/tests.rs:210` is binary only) and TDISP preservation on ASCII (`column_reader/tests.rs:194`
  is binary only).
- [ ] Column kind mismatches: `column_reader/tests.rs:143, 156, 230, 418` and
  `bit_column/tests.rs:180` → one table.
- [ ] Cards: one parse table that mirrors the render table at `card/tests.rs:588`, in place of
  `:75, 103, 147, 154, 160, 525`, with the same rows both ways (`-0.0`, `F`, `ComplexInteger`).
  Integer boundaries are tested in three places (`value.rs:345`, `header_model/tests.rs:131`,
  `card/tests.rs:403`), and `Value::from` in two (`value.rs:327`, `header_model/tests.rs:510`).
  Keep one of each.
- [ ] Distortion: SIP has alternate, malformed-keyword and fold-refusal tests. Add the same rows
  for TPV in one sweep over both.
- [ ] Forward vs inverse: spectral units (`axis_transform/tests.rs:546`) and rest velocities
  (`:79`) both ways. Projection domain refusals in one inverse table beside the forward table at
  `projection/tests.rs:1104`, in place of `:905, 954, 975, 996, 1170`. `projection/tests.rs:727`
  and `:783` have identical bodies.
- [ ] Families: rank inference for the image family (`world_coordinates/tests.rs:236`, 5
  keywords) with the 12 keywords of the vector-cell family (`table_wcs/tests.rs:261`); matrix and
  parameter keys for the primary description too (`table_wcs_resolver/tests.rs:106–168`);
  phase-axis period cases for every family (`phase_axis/tests.rs`, image only).
- [ ] Time: one table each for `time_coordinates/unit/tests.rs:103–112`, `frame/tests.rs:322,
  337, 359, 383`, the datetime refusals (`datetime/tests.rs:33, 72, 119, 227`, which repeat
  `"2024-13-01"` and `"14/03/94"`), text → MJD (`:13, 246`), and `time_axis_kind.rs` (11 asserts).

### Batch 11 — Names

Do last, after the files move. Files: every test file.

- [ ] Rename to the naming rule: one refusal form, `…_refuses_…` / `…_is_refused` (seven forms
  now, about 40 tests); no leading article (61 tests); no module prefix (`ascii_` 10,
  `table_wcs_` 5, `tab_`/`tabular_`/`multidimensional_tab_`, `hcompress_`, `fits_time_`,
  `read_column_`, `read_vla_`).
- [ ] Names that cover less than the test checks: `table/tests.rs:724`,
  `hcompress.rs:1770`, `decode/tests.rs:323`. Split each test or rename it.
- [ ] Add the naming rule and the reader rule to `fits-well/AGENTS.md` under "Test code", so new
  tests follow them.

# Part 3 — Benches

Scope: `src/bench/`, every `bench.rs`, `src/internals/scene.rs`, `benches/fits_well.rs`. Paths
are relative to `fits-well/`. The callers' workload comes from lumos (`lumos/src/io/fits/`):
section views over a `StreamSource<File>`, `stream_image` + `write_chunk` with checksums,
`write_image` with checksums, files of several HDUs selected by `EXTNAME`/`EXTVER`, binary
tables, and compressed files from capture programs and fpack.

Inventory on 2026-10-25: 7 drivers in 1 253 lines. There are 45 atomic cases (decode 5, encode
5, physical 10, unsigned 4, checksum 1, header 2, compress 5, decompress 5, WCS 8) and 84
integration cases (read_image 12, read_section 12, read_table 2, read_table_columns 2,
write_image 6, write_table 1, write_compressed 8, read_compressed 16, read_compressed_section
16, write_compressed_table 3, read_compressed_table 6). The structure is sound: one id scheme,
`Kind` and `Scale` as the one place that sets sampling, `SourceKind` over every read, inputs
built inside each case's closure, files written once and served from the page cache.

## Proposal

### Root causes

1. **Plain and compressed data are two families of groups.** `read_image` / `read_compressed`,
   `read_section` / `read_compressed_section`, `read_table` / `read_compressed_table`,
   `write_image` / `write_compressed`, `write_table` / `write_compressed_table` are separate
   groups in separate drivers. A plain/compressed comparison crosses groups, and an operation
   gets written for one storage only: `read_table_columns` has no compressed twin. The
   drivers repeat themselves: `ReadImage`/`ReadCompressed`, `ReadSections`/
   `ReadCompressedSections` and `ReadTable`/`ReadCompressedTable` differ only in the HDU
   index (0 or 1). Four lazy file caches (`frame_path`, `catalog_path`, `image_path`, and an
   inline write in `read_compressed_table`) hold the files. One of them is
   `&mut [Option<PathBuf>]`, and the code zips it with `image_codecs()` by position.
2. **Coverage follows the history of the code, not the public entry points or the hot
   paths.** Eight public entry points have no case: `ImageReader::physical_into`,
   `stream_image`/`write_chunk` (the lumos write path), `read_table_rows`, `read_table_cell`,
   `read_ascii_table`, `write_ascii_table`, `FitsEditor::replace_header` and
   `verify_checksum`. No case opens a file of many HDUs. Part 1 changes 15 hot paths, and 9 of
   them have no case that measures the change (the coverage map at the end of this part).
3. **Some cases time something different from what their names say.**
   - `integration/read_table` returns a `Table`, which holds the raw data unit
     (`bintable/mod.rs:39`). So it decodes no column, and it breaks the rule "to the last
     sample" (`src/bench/mod.rs`).
   - `atomic/physical` times `View::physical`, a reach-in that only `internals` builds
     (`image_model/view.rs:265`). It scales host-endian samples into a new `Vec` on every
     iteration. The read path runs the fused `Bitpix::extend_physical_be` from big-endian
     bytes (`reader/image_reader.rs:277`), and no case times that kernel. The case gives
     `f64` only, but lumos and `physical_into` callers take `f32`.
   - `atomic/unsigned` also allocates in the timed loop, while `decode` and `encode` reuse
     their buffers.
4. **The scene is not the frame that its own header describes.** `scene::sky` adds noise of
   σ = 0.01 of full scale, which is 655 ADU on the i16 light. The header says
   `EGAIN = 0.25` e⁻/ADU, and the sky is 0.1 of full scale (6 553.5 ADU = 1 638 e⁻), so its
   photon noise is √1 638 = 40.5 e⁻ = 162 ADU. The scene is 4× too noisy, which adds about
   2 bits per sample to every codec. The stars are single pixels of uniform random amplitude
   on 1 pixel in 50, not point-spread functions. They do not agree with the catalog, which
   draws its own positions and a FWHM of 2.5 px. The `PLIO_1` mask marks those 2 % of
   scattered pixels, the worst input for a run-length codec. A real defect map holds hot
   pixels, bad columns and saturated star cores. Codec speed and ratio follow the noise bits
   and the run lengths, so every codec case times a harder input than a real light.
5. **No integration case checks its output.** A read that returns wrong data passes and gets
   timed. `atomic/decompress` already asserts the round trip once before it times.
6. **Ids and policy are not symmetric.**
   - `atomic/header/light` and `atomic/header/render` mix an input name and an operation name
     at one level of the id, and `atomic/wcs/<direction>/<axes>` holds two operations in one
     group, against "the group is the operation" (`src/bench/mod.rs`).
   - Every write has the variant `file`, a dimension with one value.
   - Image writes compute checksums, but table writes do not, although lumos writes every
     HDU with checksums.
   - `Scale::Sample` names a row or a header. "Sample" already means a pixel value and a
     criterion sample.
   - `bench::fixture` holds the policy, but `internals::fixture` holds files.
   - `header_model/bench.rs:44` builds its writer by hand, beside `memory::written_by`.

### Target design

- **One case matrix: operation × source or sink × storage × input.** A group is one operation,
  in atomic and integration cases alike. An integration id is
  `integration/<operation>/<source|sink>/<storage>/<input>`, for example
  `integration/read_image/stream/rice_1/i16`; the storage segment is always there, `plain`
  where only plain data applies. An atomic id is `atomic/<operation>/<variant>/<input>`, the
  variant left out where an operation has one. Plain and compressed results sit side by side in
  one group. Every operation runs on every storage that its input takes, with two exceptions:
  `verify_checksum` sums the same raw bytes on every storage, and stream writes, ASCII tables
  and header edits have plain data alone. Refused: separate compressed groups, which are the
  root cause above.
- **One storage table.** `bench::policy::Storage` lists the storage of each input once:
  - The i16 light: `plain`, `gzip_1`, `gzip_2`, `rice_1`, `hcompress_1`.
  - The u8 defect mask: `plain`, `plio_1`.
  - The f32 frame: `plain`; `rice_1`, the fpack default (q = 4, `SUBTRACTIVE_DITHER_1`,
    which is `Quantization::default()`); `gzip_2_lossless` (`Quantization::NONE`), as lumos
    writes it.
  - u8, i32, i64 and f64 frames: `plain`.
  - Tables: `plain`, `gzip_1`, `gzip_2`, `rice_1`.

  The atomic codec cases come from the same table, so each compressed storage has its tile
  kernel by construction. Refused: the current quantized `gzip_1`/`gzip_2` f32 rows. They
  time the same integer codecs as the i16 rows, and they leave out the lossless float path.
- **Sinks mirror sources.** `SinkKind { File, Memory }` beside `SourceKind { Stream, Slice }`.
  `memory` times the encode alone, and `file` adds what a caller pays for the writes.
- **One file cache.** `SceneFiles` lives on the `ScratchDir`. It is keyed by `SceneFile`
  (input × storage), writes each file the first time a case needs it, and knows the HDU that
  holds the data. One `Reading` per operation takes that HDU, so each `Reading` pair becomes
  one type.
- **Each read case checks once, then times.** Before the timed loop, the case compares what it
  reads with the scene input it came from (the samples of an image, the columns of a table):
  bit-exact for lossless storage through `samples::assert_same_bits`, within half a
  quantization step for `rice_1` f32 through `assert_within_half_step` (Part 2 Batch 4). A
  check that fails stops the bench run.
- **The scene is one observation.**
  - One star list makes both the frame and the catalog. Each star is a Gaussian PSF with the
    catalog's FWHM and a power-law flux.
  - The noise is the photon noise that the header's `EGAIN` gives at the sky level, plus a
    read noise of 1.5 e⁻.
  - The defect mask holds hot pixels, bad columns and saturated star cores.
  - Two more inputs: `mosaic`, a file of a primary plus 60 image extensions with the light
    header and `EXTNAME`s, as DECam lays out its CCDs; and `spectra`, a table with a
    variable-length `f32` column, the shape that tiled-compressed tables and spectral
    catalogs store.
- **Atomic covers each hot kernel, on the unit that production hands it, into reused
  buffers.** A case allocates in its timed loop only when the API under test allocates, and
  its doc says so. The kernels are listed in Batch 6.
- **Integration covers each public read and write entry point, on the scene.** The entry
  points are listed in Batch 5.
- **Runtime.** The matrix has about 200 integration cases at about 4.5 s each (`Scale::Frame`),
  so a full run takes about 15 minutes. Daily use takes a criterion filter (`'/read_image/'`,
  `'/rice_1/'`) or `--test`. Refused: smaller frames, because a 24 MP frame is what lumos
  reads, and caches and page-cache effects change with size.
- **Refused cases.**
  - Serial and parallel variants: rayon's global pool size is fixed for each process, the
    atomic codec cases already give the serial cost per tile, and scaling with threads is a
    question for a profiler.
  - Random groups: a legacy radio format whose read takes the plain image path, and no hot
    path in Part 1 touches it.
  - `read_wcs`: it resolves a `-TAB` table once per file, and the atomic tabular cases time
    the transform.
  - `NOCOMPRESS`: no writer uses it for real data.
  - `read_data_unit` and `write_raw_hdu`: raw copies with no decode, the same I/O as
    `verify_checksum`.
  - Peak memory: a bench times. Part 1's memory items (the staged table unit, the kept region
    buffer) are held by allocation tests (`tests/view_bytes*.rs`, Part 2 Batch 8), not here.

## Plan

The id rules and the case lists live in `src/bench/mod.rs` and in each driver's doc, so this
plan does not repeat them after it lands. Each batch ends with `cargo drive bench --test` and
`cargo drive gate` green.

### Batch 1 — Cases that time the wrong thing

Independent. Files: `src/reader/bench.rs`, `src/compression/bench.rs`,
`src/image_model/bench.rs`, `src/header_model/bench.rs`, `src/fits_writer/bench.rs`.

- [ ] `reader/bench.rs` `ReadTable`, `compression/bench.rs` `ReadCompressedTable`: they return a
  `Table` and decode no column. Decode every column through the public column API, as a
  caller reads a catalog. The column reads allocate until Part 1 Batch 12 adds the `_into`
  forms; until then, the doc of the case says that it times those allocations too.
- [ ] `image_model/bench.rs` `physical`: it times the reach-in `View::physical` on host
  samples, with an allocation in each iteration. Time `Bitpix::extend_physical_be` from the
  big-endian row into a reused `Vec<f32>` and a reused `Vec<f64>`:
  `atomic/physical/<output>/<scaling>-<type>`.
- [ ] `image_model/bench.rs` `unsigned`: it allocates in the timed loop. Use the `unsigned_into`
  that Part 1 Batch 12 adds, into a reused `UnsignedData`. Until that batch lands, the doc of
  the case says that it times the allocation too.
- [ ] `header_model/bench.rs`: `atomic/header/light` and `atomic/header/render` →
  `atomic/parse_header/light` and `atomic/render_header/light`. Build the unit with
  `memory::written_by`.
- [ ] `world_coordinates/bench.rs`: `atomic/wcs/<direction>/<axes>` → `atomic/to_world/<axes>`
  and `atomic/to_pixel/<axes>`.
- [ ] `fits_writer/bench.rs` `write_table`, `compression/bench.rs` `write_compressed_table`: use
  `with_checksums()`, as the image writes and lumos do.

### Batch 2 — Policy and plumbing

Files: `src/bench/{mod,fixture}.rs` → `src/bench/{mod,policy}.rs`, every `bench.rs`.

- [ ] `src/bench/fixture.rs` → `src/bench/policy.rs`, which removes the name clash with
  `internals::fixture`.
- [ ] `Scale::Sample` → `Scale::Row`.
- [ ] Add `Storage` with its table of inputs, `SinkKind { File, Memory }`, and a `SinkKind::bench`
  that mirrors `SourceKind::bench`.
- [ ] Add `SceneFiles`, keyed by `SceneFile` (input × storage), with the HDU of the data. Delete
  `frame_path`, `catalog_path`, `image_path`, the inline write in `read_compressed_table`, and
  the `images: &mut [Option<PathBuf>]` slots.
- [ ] Add the check that runs once before timing: `SceneFile::assert_read`, over the samples of
  an image or the columns of a table, through the helpers named in the target design.

### Batch 3 — One group per operation

Needs Batch 2. Files: `src/reader/bench.rs`, `src/fits_writer/bench.rs`,
`src/compression/bench.rs`, `src/bench/mod.rs`.

- [ ] Move `read_compressed`, `read_compressed_section` and `read_compressed_table` into
  `read_image`, `read_section` and `read_table` in `reader/bench.rs`, with the storage segment
  in the id. Delete `ReadCompressed`, `ReadCompressedSections` and `ReadCompressedTable`.
  `ReadImage`, `ReadSections` and `ReadTable` take the HDU from the `SceneFile`.
- [ ] Move `write_compressed` and `write_compressed_table` into `write_image` and `write_table`
  in `fits_writer/bench.rs`, over both sinks.
- [ ] `read_table_columns`: run it on every table storage. Today the compressed tables have
  no case for it.
- [ ] Replace the float rows of `image_codecs()` with the storage table: `rice_1` f32 at the
  default quantization, `gzip_2_lossless` f32.
- [ ] `compression/bench.rs` keeps only the atomic codec cases, now generated from the storage
  table (Batch 6).
- [ ] `src/bench/mod.rs`: state the matrix, the storage segment, the sinks, and the check
  before timing.

### Batch 4 — A realistic scene

Needs Batch 2 (the file cache). Files: `src/internals/scene.rs`, `src/internals/rng.rs`, and a new
`docs/research/<date>-bench-scene.md`.

- [ ] `scene.rs` `sky`: the noise is 655 ADU where the header implies 162 ADU. Derive σ from
  `EGAIN` and the sky level (Poisson), plus a read noise of 1.5 e⁻, as constants beside the
  header values that state them.
- [ ] `scene.rs` `sky`, `catalog`: the stars are uniform single pixels, and the catalog does
  not agree with them. Draw one star list (positions, power-law flux, FWHM), render it as
  Gaussian PSFs into the frame, and write the same list as the catalog.
- [ ] `scene.rs` `mask`: 2 % of pixels at random. Mark hot pixels, bad columns and the
  saturated cores of the star list.
- [ ] `rng.rs`: add the power-law draw that the star fluxes take.
- [ ] Add `scene::mosaic_file()` (a primary plus 60 `EXTNAME`d image extensions under the light
  header) and `scene::spectra()` (a table with a variable-length `f32` column).
- [ ] Record the compressed sizes and the timings before and after the change in a dated research
  note. Every codec number moves with the scene, so the note holds the new baseline.

### Batch 5 — Integration coverage

Needs Batches 2–4. Files: `src/reader/bench.rs`, `src/fits_writer/bench.rs`, a new
`src/fits_editor/bench.rs`, `src/bench/mod.rs` (registration).

- [ ] `integration/open/<source>/plain/{frame,mosaic}`: open the file and select its last HDU,
  by `EXTNAME` in the mosaic. This measures the header scan and the stream seeks (Part 1
  Batch 7, the copy per block; Part 1 Batch 8, `seek_relative`).
- [ ] `integration/read_physical/<source>/<storage>/<type>`: `physical_into` a reused
  `Vec<f32>`, a section at a time, as `read_section` does (Part 1 Batch 8, the compressed
  `physical_into` buffer).
- [ ] `integration/read_table_rows/<source>/<storage>/catalog`: every column, in chunks of
  `CHUNK_ROWS` rows through `read_table_rows`, beside `read_table_columns`.
- [ ] `integration/read_table_cells/<source>/<storage>/{catalog,spectra}`: 1 000 cells at
  seeded random rows through `read_table_cell` (Part 1 Batch 8, the decode per call).
- [ ] `integration/read_table_columns/<source>/<storage>/spectra`: the fixed columns of a table
  with an array column (Part 1 Batch 8, `has_arrays` ignores the selection).
- [ ] `integration/verify_checksum/<source>/plain/<type>`: the plain frames. The sum reads raw
  bytes, so the storage does not change the work.
- [ ] `integration/write_image_stream/<sink>/plain/<type>`: `stream_image` plus `write_chunk` of
  `SECTION_ROWS` rows with checksums, the lumos write path. This mirrors `read_section`.
- [ ] `integration/{read,write}_ascii_table/<source|sink>/plain/catalog`: the catalog as an ASCII
  table.
- [ ] `integration/edit_header/file/plain/light`: `FitsEditor::replace_header` that adds the plate
  solution to a light on disk, as a solver does.

### Batch 6 — Atomic coverage

Needs Batch 2 (the storage table). Files: `src/compression/bench.rs`,
`src/world_coordinates/bench.rs`, new `src/bintable/bench.rs`,
`src/ascii/bench.rs`, `src/time_coordinates/bench.rs`, `src/bench/mod.rs`.

- [ ] `atomic/{compress,decompress}/<storage>/<width>`: every codec of the storage table on one
  default tile at i16 and i32 width (the quantized f32 ints are i32), `plio_1` on the mask,
  and the byte path of `gzip_2_lossless` on an f32 row. `CodecTile` takes every width and the
  byte path.
- [ ] `atomic/{quantize,dequantize}/f32`: one row of the f32 frame at the default quantization.
- [ ] `atomic/parse_wcs/light`: `Wcs::from_header` of the light header (Part 1 Batch 10, `CTYPE`
  parsed 5–7 times).
- [ ] `atomic/{to_world,to_pixel}/{sip,tpv,tabular_coupled}`: the light's plate solution with an
  order-3 SIP, without `AP`/`BP` so that the inverse iterates; the same as TPV; the 2-D
  coupled `-TAB` (Part 1 Batch 10, the allocations per point and TPV's partials).
- [ ] `atomic/to_mjd/<scale>`: one row of relative times to MJD in `utc`, `tt` and
  `tt_tai` (Part 1 Batch 10, `relative_to_mjd`'s allocation per value).
- [ ] `atomic/read_column/fixed/<type>`: one catalog column's big-endian cells into a reused
  buffer for i16, i32, f32 and f64; `atomic/read_column/vla/f32`: the spectra column (Part 1
  Batch 12, the column `_into` reads).
- [ ] `atomic/{parse_ascii,format_ascii}/catalog`: one catalog row as ASCII fields.

### Coverage map

Each hot path that Part 1 changes, and the case that measures the change. A dash marks a
case that one of the batches above adds. Two rows change memory more than time; their
allocation test is named beside the case.

| Part 1 change | Case |
|---|---|
| Batch 7: scan copies each block twice | — `open` |
| Batch 8: gzip deflate state per tile | `compress/gzip_1`, `write_image/*/gzip_*` |
| Batch 8: `TiledTable::decode` allocates per call | `read_table/*/{gzip,rice}*` |
| Batch 8: `has_arrays` ignores the selection | — `read_table_columns/*/spectra` |
| Batch 8: compressed `read_table_cell` decodes per call | — `read_table_cells` |
| Batch 8: compressed `physical_into` keeps a region buffer | — `read_physical`; `tests/view_bytes.rs` |
| Batch 8: `words.clear()` zeroes each view | `read_section` |
| Batch 8: `StreamSource` seeks from the start | `read_section/stream`, — `open/stream` |
| Batch 9: binary-table write stages the unit | `write_table`; a peak count under the shared allocator |
| Batch 10: `CTYPE` parsed 5–7 times | — `parse_wcs` |
| Batch 10: coupled `-TAB` allocates per point | — `to_world/tabular_coupled`, `to_pixel/tabular_coupled` |
| Batch 10: TPV evaluates its partials | — `to_world/tpv` |
| Batch 10: `relative_to_mjd` allocates per value | — `to_mjd` |
| Batch 11: codecs fill the caller's buffer | `compress`, `decompress` |
| Batch 12: column reads allocate | — `read_column`, `read_table` |
