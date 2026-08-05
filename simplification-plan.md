# fits-well simplification plan

A close read of the production source (tests excluded) looking for
over-engineering, accidental complexity, duplicated data structures, and reuse
opportunities.

The crate is in good shape overall: the layering is clean, the comments explain
*why* rather than *what*, and several consolidation passes are visible
(`SampleType::from_scaling` and `Scaling::unsigned_kind` shared by the image and
table paths, `endian`'s big-endian primitives, `words`'s single reinterpretation,
`column`'s single name-matching rule, `TileGeometry`, the `PROJECTIONS`
membership table). What follows is what is left.

Batches are sized to be done in one sitting and listed in recommended order.
Within a batch, items are ordered by how much they depend on each other.
Line-count estimates are rough. Line anchors are current as of this revision.

---

## Batch 1 — Writer: real duplicates and inconsistent template handling

### 1.1 Block padding, three implementations

- `pad_to_block(buf, fill)` — the helper, `writer/mod.rs:48`.
- `finish_hdu` — **inlines the identical rem/`checked_add`/`resize` body**
  instead of calling it, `writer/mod.rs:143–151`.
- `ImageStream::finish` — a third formula,
  `(BLOCK_SIZE - data_bytes % BLOCK_SIZE) % BLOCK_SIZE`, `writer/image.rs:235`.

`finish_hdu` should just call `pad_to_block(&mut self.scratch, fill)?`. This one
is a plain copy-paste.

### 1.2 `merge_header_template` is applied in three different places

- inside `image_header_parts` (`writer/image.rs:343`)
- at the end of `bintable_header` (`writer/table.rs:494`) and
  `ascii_table_header` (`writer/ascii.rs:351`)
- separately, *after* the header builder, in
  `write_compressed_image_template` (`writer/image.rs:111`)

Four builders, three placements. Pick one — applying it once in `finish_hdu`
would be simplest — so a future header builder can't forget it.

### 1.3 `WriteColumnData`'s bit variants double every match

`enum WriteColumnData { Fixed, Vla, VlaBits, Bits }` (`writer/table.rs:46`) is
matched in six places (`inferred_rows`, `wide`, `validate_column`, `pack_cell`,
`bintable_header`'s `stores_i64`, and twice in `write_table_template`). The
`Bits`/`VlaBits` variants exist only to carry a bit count that `ColumnData`
can't express.

Folding them into `Fixed`/`Vla` plus a `bit_count: Option<usize>` field on
`WriteColumn` (which already carries four optional fields) halves the arm count.
`validate_column` (`writer/table.rs:605–724`) drops from 120 lines to roughly 70.

*Risk: medium — the widest-reaching change in the batch, and the only one here
that isn't a pure mechanical win. Do 1.1/1.2 first and treat this as separable.*

---

## Batch 2 — De-over-engineer the compressed-image decode path

`src/compress/decode.rs` (1160 lines) is the most abstraction-heavy file in the
crate. It carries seven cooperating types for one decode: `ImageLayout`,
`ImageDecodePlan`, `DecodeCtx`, `CodecParams`, `Dequant`, `TileColumns`,
`TileScratchSet`, `CodecScratch` — plus two traits and an enum whose only job is
type erasure.

### 2.1 `WidePlane` + `DecodeSample` + `DecodeBuffer` is one idea spelled three times

- `trait WidePlane` (`:231`) — 2 impls, selects "decode in `i64` or `f64`".
- `trait DecodeSample` (`:296`) — 6 impls, every one a single `as` cast.
- `enum DecodeBuffer` (`:344`) — 6 variants, exists only to erase the type so
  `decode_image_section_into` (`:562`) and `decode_image_into` (`:588`) can
  re-dispatch with six identical arms each.

The two `debug_assert_eq!(plan.context.zbitpix.is_float(), output.is_float(), "…must match")`
are the tell: the same fact is encoded twice and kept in sync by assertion rather
than by construction. Deriving the buffer from `ImageLayout` once would make the
assertion unnecessary.

Simplest reduction that keeps the "narrow only at scatter time" property: keep
`DecodeBuffer`, drop `DecodeSample` in favour of a `narrow` free function
selected by the buffer variant, and drop `WidePlane` in favour of two explicit
`decode_int_tile` / `decode_float_tile` calls chosen once from
`layout.bitpix.is_float()`.

### 2.2 `run_decode_scatter` has two divergent bodies

`:611–662` is `#[cfg(feature = "parallel")] { … } #[cfg(not(…))] { … }` — two
complete implementations, and they are **not** equivalent: the parallel arm
narrows inside the worker and scatters with `std::convert::identity`; the serial
arm scatters with `D::narrow`. Two paths, two behaviours to test, and the
`convert:` parameter on `scatter_rows` (`:747`) exists only to paper over the
difference.

Make the serial path a one-tile wave through the same code. `scatter_rows`'s
`convert` parameter then disappears.

### 2.3 `ImageDecodePlan` has 15 fields

`:88–104`, built by a 70-line constructor (`:107–180`). It bundles four
unrelated concerns. Split into:

- `TileSources { primary, gzip_fallback, uncompressed }` — `TileColumns::read`
  (`:853`) already takes exactly these three as separate arguments.
- `NullMask { column, codec }`
- `FloatQuantization { method, zdither0, zscale, zzero, zblank_keyword, zblank_column }`
- the existing `DecodeCtx` + `geometry`

### 2.4 Small duplicates in the same file

- `apply_integer_null_mask` (`:968`) and `apply_float_null_mask` (`:990`) are
  identical apart from the fill value → one generic `apply_null_mask<T>(…, masked: T)`.
- `read_f64_column` (`:798`) and `read_i64_column` (`:813`) share the
  `column_index → column_by_idx → raw → match` shape.

**Estimated net: −150 to −200 lines, and one fewer code path under `parallel`.**

---

## Batch 3 — One N-dimensional odometer instead of five

The "decompose a flat index into mixed-radix per-axis coordinates, then walk"
loop is written five times, each subtly different:

| site | file |
| --- | --- |
| image-section run coalescing | `reader/mod.rs:1090–1154` |
| tile row-base emission | `compress/geometry.rs:86–127` |
| tile selection for a region | `compress/decode.rs:427–487` |
| per-row region scatter | `compress/decode.rs:703–745` |
| inverse-search cell walk | `wcs/tabular/mod.rs:516–570` |

They differ in what they do with the coordinates (strides, clipping, range
membership) but the decomposition core is identical, and four of the five repeat
the same `checked_mul`/`checked_add` overflow chain around it.

Extract a small `nd` module:

```rust
pub(crate) fn mixed_radix(flat: usize, extents: &[usize], out: &mut [usize]);
pub(crate) struct Odometer { … }   // incremental `next()` for the walking sites
```

`TileGeometry::tile_into` and `visit_image_region_runs` in particular are the
same algorithm (contiguous fastest-axis run + odometer over the higher axes)
written twice with different variable names.

**Estimated net: −60 to −100 lines, and the overflow handling gets audited once.**

---

## Batch 4 — Keyword-set and index-parse consolidation

### 4.1 Three wrappers around `keyword::index`

- `writer::indexed_keyword(keyword, prefix)` (`writer/mod.rs:353`) — adds a `len ≤ 8` bound.
- `compress::parameter_index(keyword)` (`compress/mod.rs:212`) — adds `1..=999`.
- `compress::table::indexed_compression_key(keyword, prefix, ncols)` (`compress/table.rs:559`) — adds `1..=ncols`.

Give `keyword::index` an optional bound parameter (or add
`keyword::indexed_in(keyword, prefix, range)`) and delete all three.

### 4.2 The Z-table keyword set is spelled three times

- `writer::is_structural_keyword` (`writer/mod.rs:303`) — 34 exact keywords + 15 prefixes.
- `compress::table::reject_compression_metadata` (`compress/table.rs:632`) — 8 Z-keywords + 2 prefixes.
- `compress::table::uncompress_table`'s `remove_where` (`compress/table.rs:534`) — the same 8 + 2, again, in the same file.

Extract `const ZTABLE_KEYWORDS: [&str; 8]` and `const ZTABLE_PREFIXES: [&str; 2]`
and have all three consult them. The two lists in `table.rs` being identical
literals 100 lines apart is the clearest instance.

### 4.3 `Header`'s removal APIs and their O(n) reindex

`remove`, `remove_at`, `remove_all`, `remove_where`, plus `rename_keywords` and
`append_filtered_from` each end in a full `reindex()` (`header/mod.rs:475`, six
call sites). `reader::finish_table_selection` (`reader/mod.rs:773–775`) calls
`remove_all` three times in a row — three full index rebuilds where
`remove_where(|k| matches!(k, "THEAP" | "CHECKSUM" | "DATASUM"))` would do one.

`remove_where` is currently `#[cfg(feature = "compression")]`; ungate it, make
`remove_all` a thin wrapper over it, and use it at the reader call site.

---

## Batch 5 — WCS: split the long functions, collapse the Table-22 double dispatch

### 5.1 `Wcs::from_header_with_context` is 254 lines

`wcs/mod.rs:737–990`. It does six separable jobs: read the axis vectors; resolve
the spectral frames; establish the CD / PC×CDELT / CROTA precedence; apply
`CUNIT` scaling to `CRVAL` and the matrix rows; parse the per-axis transforms and
collect unsupported axes; compute the celestial pole. Each is a natural private
function; the celestial-pole block alone (`:903–967`) is 65 lines nested inside a
`match`.

### 5.2 The Table-22 keyword layer has two dispatch levels for one decision

`TableWcsResolver` (`wcs/mod.rs:392–555`) exposes 14 methods that are pure
keyword spellings (`pixel_axis_key`/`vector_axis_key`,
`pixel_matrix_key`/`vector_matrix_key`, `pixel_parameter_key`/
`vector_parameter_key`, …). `TableWcs` (`:569–719`) then re-dispatches each of
them on `PixelList` vs `ArrayColumn`. Three supporting enums
(`TableAxisKeyword`, `TableMatrixKeyword`, `TablePoleKeyword`, `:297–389`) exist
only to return static string pairs.

Collapse to one const table plus a single `TableWcs::key(family, index)`. That
~330-line block should roughly halve, and adding a keyword family becomes one
table row.

### 5.3 `vector_axis_present` is a performance and complexity outlier

**Worth doing on its own merits even if the rest of this batch is skipped.**

`wcs/mod.rs:529–554` probes, per candidate axis: 6 axis keywords, 100 × 2 × 2
parameter keywords, and 99 × 2 × 2 matrix keywords — ~800 header lookups. It is
called for axes 99 down to 1 until one hits, so inferring the rank of a one-axis
array column costs on the order of 80k hash lookups.

`infer_image_axis_count` (`:1552`) already solves the identical problem the right
way: one pass over `header.iter()`, taking the max parsed index. Rewrite
`vector_axis_present` the same way and the helper disappears along with the
`(1..=99)` / `(0..=99)` scans.

### 5.4 Smaller WCS items

- `first_real(header, a, b)` (`:1621`) exists for two-keyword fallbacks, but the
  text equivalent is open-coded twice: `RADESYS`/`RADECSYS` in `celestial_frame`
  (`:1285`) and `RESTFRQ`/`RESTFREQ` in `spectral_rest` (`:1338`). Add
  `first_text`.
- `wcs/axis.rs`: `convert` (`:645`) and `conversion_derivative` (`:706`) are
  parallel 12-arm `(from, to)` matches, both ending in
  `unreachable!("all … covered")`. Returning both from one match (or pairing
  them in a table) removes one `unreachable!` and guarantees they stay in step —
  a value/derivative pair drifting apart is a silent numerical bug.

---

## Batch 6 — Mechanical cleanups

Small, independent, low risk. Good filler for the end of a session.

1. **Row-offset arithmetic, three copies.** The
   `data_offset + row × row_len` double-`checked_*` chain appears at
   `reader/mod.rs:663–670`, `:697–704`, `:831–839`. Extract
   `fn row_offset(base: u64, row: usize, row_len: usize) -> Result<u64>`.
   The `data_offset + heap_range.start` chain appears twice (`:754`, `:856`).

2. **`verify_checksum`'s duplicated status match.** `reader/mod.rs:979–989` — the
   `datasum` and `checksum` initial-status blocks are identical; one helper
   `fn initial_status(stored: Option<&str>) -> ChecksumStatus`.

3. **`AsciiColumnReader::raw`'s two `unreachable!` loops.** `ascii/mod.rs:247–273`
   — two near-identical loops each with an `unreachable!` arm, because
   `parse_numeric_field` (`:304`) returns an untyped `ParsedNumeric`. Split it
   into `parse_integer_field` / `parse_float_field`, or have `physical` be the
   only `ParsedNumeric` consumer. Removes both `unreachable!`s.

4. **`TimeReferencePosition::parse`.** `time/mod.rs:432–451` — 15 `match` guard
   arms of `value.starts_with("XXX")`. A `const` `[(&str, TimeReferencePosition)]`
   table plus a `find` is half the length and self-evidently exhaustive.

5. **Sign-flip constants and helpers.** `data/mod.rs:542–580` — four `f64`
   offsets, four integer sign masks, four `flip_*` and four `store_*` const fns.
   A three-line macro cuts ~35 lines. Low value — the current spelling is at
   least explicit — so only worth it if you are in the file anyway.

6. **Naming consistency.** `ColumnData::element_count` vs `AsciiColumnData::len`
   vs `ImageData::len` for the same question. Pick one.

7. **`Header::set_card` clones to validate.** `header/mod.rs:337–351` clones the
   existing card, mutates the clone, validates, then assigns. Validating the
   proposed `(keyword, value, comment)` directly avoids a `String` clone per
   `set` on an existing keyword — and `set_internal` is called dozens of times
   per written HDU.

---

## Batch 7 — `FitsError`: fold the two structurally identical families

Last, and optional. `error.rs` is 671 lines with 58 variants and a 230-line
`Display`. Most of that is *precision*, not duplication, and should stay: six
distinct "wrong HDU kind" variants and five distinct "wrong column accessor"
variants each carry a specific message, a caller handles exactly the one its call
site can produce, and `Display`'s exhaustive match already stops the arms drifting
out of sync with the variants.

Two families are genuine duplication — the same variant written five and three
times over with a different noun:

### 7.1 Five index-out-of-bounds variants → one

`HduIndexOutOfBounds`, `HeaderIndexOutOfBounds`, `ColumnIndexOutOfBounds`,
`GroupIndexOutOfBounds`, `WcsAxisIndexOutOfBounds` all carry `(index, len)` and
identical semantics, differing only in two nouns per message. Fold to
`IndexOutOfBounds { indexed: Indexed, index: usize, len: usize }` with a
five-variant `Indexed`. That collapses 5 variants and ~25 `Display` lines into 1
and 5, keeps typed pattern-matching, and gives callers one place to handle "some
index was out of range". `WcsAxisIndexOutOfBounds` is 1-based — carry that in the
`Indexed` variant's message, as it already does today.

### 7.2 Three rank/count-mismatch variants → one

`ImageRegionRankMismatch`, `TileShapeRankMismatch`, `CoordinateCountMismatch` are
all `(expected, got)` with a noun → `RankMismatch { kind, expected, got }`, same
shape as 7.1.

**Use a typed nested enum, not a `&'static str` discriminant.** A string would
make callers and tests match on display wording
(`matches!(e, IndexOutOfBounds { indexed: "column", .. })`) — fragile, and out of
step with a crate that models `HduKind`, `SampleType`, and `ChecksumStatus` as
typed enums. The typed version is close to line-neutral; the win is the smaller
top-level enum and the explicit grouping, not the line count.

Blast radius measured: 31 sites for 7.1, 16 for 7.2.

---

## Deliberate designs left alone

Recorded so a later pass doesn't re-litigate them:

- **`hdu::axis_product` vs `data::shape_product`** — two products with different
  empty-list semantics (1 vs 0) and different widths (`u64` vs `usize`). Both are
  documented as to why; merging them would reintroduce the `NAXIS = 1`
  random-groups size bug the comments describe.
- **`Compression` / `ImageCodec` / `Algo`** (`compress/mod.rs:134,203`,
  `compress/table.rs:34`) — three codec enums, but each is a genuinely different
  set (public choice with config, all image codecs, the §10.3 column subset), and
  `Algo::name` already routes through `ImageCodec::name` so the strings can't
  drift. `Algo` could be folded into a validated `ImageCodec` constructor for
  ~40 lines, but the current shape makes the "no HCOMPRESS on a table column"
  rule unrepresentable rather than merely checked. Leave it.
- **`Source`'s `slice` vs `read_owned`** (`reader/source.rs:50,61`) — looks
  redundant, but the doc comment correctly explains the one-copy vs two-copy
  distinction.
- **The `write_x` / `write_x_with_header` / `write_x_template` triples** — 16
  public forwarding functions across image/table/ascii/compressed, but this is
  the idiomatic Rust shape for optional parameters and the alternative
  (`Option<&Header>` in the public signature) is worse ergonomics.
- **The six-arm `BITPIX` matches** on `ImageData` / `ImageView` / `DecodeBuffer`
  — a match over a tagged union is the clear spelling, and macro-generating them
  would cost the per-variant rustdoc on two public enums. The duplicated *logic*
  around them has already been hoisted out; shrinking `DecodeBuffer` itself is
  Batch 2.1's job, not a macro's.
- **`ColumnType`** (`writer/table.rs:31`) — `TformKind` minus `X`/`P`/`Q`, whose
  remaining job is making "not a bit array, not a descriptor" unrepresentable in
  a writer column. A runtime guard would save ~60 lines and lose that.
- **`ColumnReader` / `AsciiColumnReader`** — two `{ table, index }` handles with
  the same shape but different `descriptor()` types. A shared generic needs an
  associated type to say that, which is more machinery than the duplication.
- **`wcs/projection/`** — the `PROJECTIONS` membership table and `Family` enum
  already do exactly the consolidation this document recommends elsewhere.

---

## Note on this file

`Cargo.toml`'s `exclude` list keeps `docs/`, `tests/`, `AGENTS.md`, `CLAUDE.md`,
and `todo.txt` out of the published tarball, but not this file. Either add
`simplification-plan.md` to `exclude` or delete it once the work lands (as the
previous plan was).
