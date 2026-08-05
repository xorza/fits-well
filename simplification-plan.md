# fits-well simplification plan

A close read of the ~13k lines of production source (tests excluded) looking for
over-engineering, accidental complexity, duplicated data structures, and reuse
opportunities.

The crate is in good shape overall: the layering is clean, the comments explain
*why* rather than *what*, and several earlier consolidation passes are visible
(`SampleType::from_scaling` shared by image and table, `endian::decode_be*`,
`TileGeometry`, the `PROJECTIONS` membership table). What follows is what is
left.

Findings are grouped into batches, each sized to be done in one sitting.
Batches are ordered by value; within a batch, items are ordered by how much
they depend on each other. Line-count estimates are rough.

---

## Batch 1 — Collapse the BITPIX-dispatch triplication

Landed. `src/words.rs` now owns the sole `u64`-backed reinterpretation (behind an
`unsafe trait Sample` marker), `compress/convert.rs` routes every big-endian
conversion through `endian.rs`, and `Scaling::unsigned_kind` is the one place the
FITS sign-bit-offset convention is resolved.

Item 1.2 was **not** done as written: macro-generating the `ImageData` /
`ImageView` / `DecodeBuffer` accessors would have cost the per-variant rustdoc on
two public enums and made the code harder to navigate for no behavioural gain,
and the `Sample` trait as sketched (an associated `[u8; N]`) needs
`generic_const_exprs`. A six-arm match over a tagged union is the clear spelling;
what was worth removing was the *logic* duplicated around those matches, which
1.1/1.3/1.4 covered. Reducing `DecodeBuffer` itself is Batch 4's job.

The numbering below is unchanged.

---

## Batch 2 — Unify the binary-table and ASCII-table structures

Landed. `src/column.rs` now states the §6.7/§7.2.2 name-matching rule once;
`TableSchema` owns the binary lookup (so `BinTable` and the reader's column
selectors share it and `reader::resolve_column_name` is gone); all five `TDIMn`
rules live in `table/mod.rs`; `Hdu::ensure_bintable` replaces the duplicated
three-kind guard; and `writer::accept_row_count` is the builders' one row-count
inference rule.

Two items were done smaller than written, deliberately:

- **2.2** — the two reader handles were left as separate structs, as the item
  itself allowed. What they actually shared was the index bounds-check, which is
  now `column::validate_index` (a third copy of it lived in the reader).
- **2.3** — a generic `RowBuilder<C>` would have put a generic type and a
  `BuilderColumn` trait into the *published* API to save two trivial constructor
  pairs. The only part that could realistically drift is the row-count decision
  in `push`, and that is now shared outright. The two builders stay concrete and
  separately documented, and the public API is unchanged.

The numbering below is unchanged.

---

## Batch 3 — Consolidate `FitsError`

**Demoted after a closer look — read this note before starting.**

The line-count case I made below does not survive contact with the code. It
assumed a `&'static str` discriminant, which would make callers and tests match
on *display wording* (`matches!(e, WrongHduKind { expected: "a binary table" })`)
— fragile, and out of step with a crate that models `HduKind`, `SampleType`, and
`ChecksumStatus` as typed enums. The version that keeps typed matching replaces
each family with a nested enum plus a phrase mapping, which is close to
line-neutral: `error.rs` would land nearer 620 than the 460 estimated.

What survives:

- **3.1 and 3.2 are not duplication.** Six precise "wrong HDU kind" variants and
  five precise "wrong column accessor" variants are a *design*, and the one that
  fits a library whose selling point is whole-standard coverage. A caller invokes
  one accessor and handles its one error; nobody enumerates all six. `Display`
  is exhaustively matched, so the compiler already prevents the arms from
  drifting out of sync with the variants.
- **3.3 is real duplication** — five variants with identical `(index, len)`
  fields and identical semantics, differing only in two nouns. `IndexOutOfBounds
  { indexed: Indexed, index, len }` with a 5-variant `Indexed` collapses 5
  variants and ~25 `Display` lines into 1 and 5, keeps typed matching, and gives
  callers one place to handle "some index was out of range". **3.4** is the same
  shape for `(expected, got)` rank mismatches.

Recommendation: do 3.3 and 3.4 if the 58-variant enum bothers you; skip 3.1/3.2.
Blast radius measured: 34 sites for 3.1, 26 for 3.2, 31 for 3.3, 16 for 3.4.

The original write-up follows.

58 variants (`src/error.rs`), with a 230-line `Display`. Three families are
structurally identical and could collapse with no loss of information. The
project's stated policy ("no backward compatibility", `#[non_exhaustive]`)
makes this cheap.

### 3.1 Six "wrong HDU kind" variants → one

`NotAnImage`, `NotABinTable`, `NotRandomGroups`, `NotAnAsciiTable`,
`NotCompressedImage`, `NotCompressedTable` → `WrongHduKind { expected: &'static str }`.
(Keep `ImageHasGroups` — it is a different condition.)

### 3.2 Five "wrong column kind" variants → one

`VariableLengthColumn`, `NotAVla`, `NotABitColumn`, `NotAComplexColumn`,
`NonNumericColumn` all carry exactly `code: char` and differ only in the message
→ `ColumnKindMismatch { code: char, expected: &'static str }`.

### 3.3 Five index-out-of-bounds variants → one

`HduIndexOutOfBounds`, `HeaderIndexOutOfBounds`, `ColumnIndexOutOfBounds`,
`GroupIndexOutOfBounds`, `WcsAxisIndexOutOfBounds` all carry `(index, len)` →
`IndexOutOfBounds { kind: &'static str, index: usize, len: usize }`.
`WcsAxisIndexOutOfBounds` is 1-based; carry that in `kind`'s text as it already
does in the message.

### 3.4 Optional: three rank/count-mismatch variants → one

`ImageRegionRankMismatch`, `TileShapeRankMismatch`, `CoordinateCountMismatch`
are all `(expected, got)` with a noun → `RankMismatch { kind, expected, got }`.

**Estimated: 58 → ~42 variants, `error.rs` 671 → ~460 lines.** The test-side
churn is mechanical (`matches!(e, FitsError::NotABinTable)` →
`matches!(e, FitsError::WrongHduKind { expected: "binary table" })`).

---

## Batch 4 — De-over-engineer the compressed-image decode path

`src/compress/decode.rs` (1172 lines) is the most abstraction-heavy file in the
crate. It carries **seven** cooperating types for one decode: `ImageLayout`,
`ImageDecodePlan`, `DecodeCtx`, `CodecParams`, `Dequant`, `TileColumns`,
`TileScratchSet`, `CodecScratch` — plus two traits and an enum whose only job is
type erasure.

### 4.1 `WidePlane` + `DecodeSample` + `DecodeBuffer` is one idea spelled three times

- `trait WidePlane` (`:230`) — 2 impls, selects "decode in `i64` or `f64`".
- `trait DecodeSample` (`:295`) — 6 impls, every one a single `as` cast.
- `enum DecodeBuffer` (`:343`) — 6 variants, exists only to erase the type so
  `decode_image_into` (`:600`) and `decode_image_section_into` (`:574`) can
  re-dispatch with six identical arms each.

The two `debug_assert_eq!(plan.context.zbitpix.is_float(), output.is_float(), "…must match")`
(`:584`, `:607`) are the tell: the same fact is encoded twice and kept in sync by
assertion rather than by construction. Deriving the buffer from `ImageLayout`
once would make the assertion unnecessary.

Simplest reduction that keeps the "narrow only at scatter time" property: keep
`DecodeBuffer`, drop `DecodeSample` in favour of a `narrow` free function
selected by the buffer variant, and drop `WidePlane` in favour of two explicit
`decode_int_tile` / `decode_float_tile` calls chosen once from
`layout.bitpix.is_float()`.

### 4.2 `run_decode_scatter` has two divergent bodies

`:623–674` is `#[cfg(feature = "parallel")] { … } #[cfg(not(…))] { … }` — two
complete implementations, and they are **not** equivalent: the parallel arm
narrows inside the worker and scatters with `std::convert::identity`; the serial
arm scatters with `D::narrow`. Two paths, two behaviours to test, and the
`convert:` parameter on `scatter_rows` (`:759`) exists only to paper over the
difference.

Make the serial path a one-tile wave through the same code. `scatter_rows`'s
`convert` parameter then disappears.

### 4.3 `ImageDecodePlan` has 15 fields

`:87–103`, built by a 70-line constructor (`:105–179`). It bundles four
unrelated concerns. Split into:

- `TileSources { primary, gzip_fallback, uncompressed }` — `TileColumns::read`
  (`:865`) already takes exactly these three as separate arguments.
- `NullMask { column, codec }`
- `FloatQuantization { method, zdither0, zscale, zzero, zblank_keyword, zblank_column }`
- the existing `DecodeCtx` + `geometry`

### 4.4 Small duplicates in the same file

- `apply_integer_null_mask` (`:980`) and `apply_float_null_mask` (`:1002`) are
  identical apart from the fill value → one generic `apply_null_mask<T>(…, masked: T)`.
- `read_f64_column` (`:810`) and `read_i64_column` (`:825`) share the
  `column_index → column_by_idx → raw → match` shape.

**Estimated net: −150 to −200 lines, and one fewer code path under `parallel`.**

---

## Batch 5 — One N-dimensional odometer instead of five

The "decompose a flat index into mixed-radix per-axis coordinates, then walk"
loop is written five times, each subtly different:

| site | file |
| --- | --- |
| image-section run coalescing | `reader/mod.rs:1104–1168` |
| tile row-base emission | `compress/geometry.rs:86–127` |
| tile selection for a region | `compress/decode.rs:474–497` |
| per-row region scatter | `compress/decode.rs:729–756` |
| inverse-search cell walk | `wcs/tabular/mod.rs:536–540` |

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

## Batch 6 — Writer: real duplicates and inconsistent template handling

### 6.1 Block padding, three implementations

- `pad_to_block(buf, fill)` — the helper, `writer/mod.rs:48`.
- `finish_hdu` — **inlines the identical rem/`checked_add`/`resize` body**
  instead of calling it, `writer/mod.rs:143–151`.
- `ImageStream::finish` — a third formula,
  `(BLOCK_SIZE - data_bytes % BLOCK_SIZE) % BLOCK_SIZE`, `writer/image.rs:235`.

`finish_hdu` should just call `pad_to_block(&mut self.scratch, fill)?`. This one
is a plain copy-paste.

### 6.2 `merge_header_template` is applied in three different places

- inside `image_header_parts` (`writer/image.rs:343`)
- at the end of `bintable_header` (`writer/table.rs:508`) and
  `ascii_table_header` (`writer/ascii.rs:360`)
- separately, *after* the header builder, in
  `write_compressed_image_template` (`writer/image.rs:111`)

Four builders, three placements. Pick one — applying it once in `finish_hdu`
would be simplest — so a future header builder can't forget it.

### 6.3 `WriteColumnData`'s bit variants double every match

`enum WriteColumnData { Fixed, Vla, VlaBits, Bits }` (`writer/table.rs:46`) is
matched in six places (`inferred_rows`, `wide`, `validate_column`, `pack_cell`,
`bintable_header`'s `stores_i64`, and twice in `write_table_template`). The
`Bits`/`VlaBits` variants exist only to carry a bit count that `ColumnData`
can't express.

Folding them into `Fixed`/`Vla` plus a `bit_count: Option<usize>` field on
`WriteColumn` (which already carries four optional fields) halves the arm count.
`validate_column` (`:619–738`) drops from 120 lines to roughly 70.

*Risk: medium — this is the widest-reaching change in the batch. Optional.*

### 6.4 `ColumnType` may not earn its keep

`ColumnType` (`writer/table.rs:31`, methods at `:518–581`) is `TformKind` minus
`X`/`P`/`Q`, and its `letter()`/`elem_size()` already forward to `TformKind`. Its
only remaining job is proving "not X, not a descriptor". A
`fn stored_element(TformKind) -> Result<TformKind>` guard would do the same for
~60 fewer lines — at the cost of moving the check from compile time to run time.
Flagging as a judgement call, not a recommendation.

---

## Batch 7 — Keyword-set and index-parse consolidation

### 7.1 Three wrappers around `keyword::index`

- `writer::indexed_keyword(keyword, prefix)` (`writer/mod.rs:322`) — adds a `len ≤ 8` bound.
- `compress::parameter_index(keyword)` (`compress/mod.rs:212`) — adds `1..=999`.
- `compress::table::indexed_compression_key(keyword, prefix, ncols)` (`compress/table.rs:559`) — adds `1..=ncols`.

Give `keyword::index` an optional bound parameter (or add
`keyword::indexed_in(keyword, prefix, range)`) and delete all three.

### 7.2 The Z-table keyword set is spelled three times

- `writer::is_structural_keyword` (`writer/mod.rs:272`) — 34 exact keywords + 15 prefixes.
- `compress::table::reject_compression_metadata` (`compress/table.rs:632`) — 8 Z-keywords + 2 prefixes.
- `compress::table::uncompress_table`'s `remove_where` (`compress/table.rs:534`) — the same 8 + 2, again, in the same file.

Extract `const ZTABLE_KEYWORDS: [&str; 8]` and `const ZTABLE_PREFIXES: [&str; 2]`
and have all three consult them. The two lists in `table.rs` being identical
literals 100 lines apart is the clearest instance.

### 7.3 `Header`'s five removal APIs and their O(n) reindex

`remove`, `remove_at`, `remove_all`, `remove_where`, plus `rename_keywords` and
`append_filtered_from` each end in a full `reindex()` (`header/mod.rs:475`, six
call sites). `reader::finish_table_selection` (`reader/mod.rs:771–773`) calls
`remove_all` three times in a row — three full index rebuilds where
`remove_where(|k| matches!(k, "THEAP" | "CHECKSUM" | "DATASUM"))` would do one.

`remove_where` is currently `#[cfg(feature = "compression")]`; ungate it, make
`remove_all` a thin wrapper over it, and use it at the reader call site.

---

## Batch 8 — WCS: split the long functions, collapse the Table-22 double dispatch

### 8.1 `Wcs::from_header_with_context` is 265 lines

`wcs/mod.rs:737–990`. It does six separable jobs: read the axis vectors; resolve
the spectral frames; establish the CD / PC×CDELT / CROTA precedence; apply
`CUNIT` scaling to `CRVAL` and the matrix rows; parse the per-axis transforms and
collect unsupported axes; compute the celestial pole. Each is a natural private
function; the celestial-pole block alone (`:903–967`) is 65 lines nested inside a
`match`.

### 8.2 The Table-22 keyword layer has two dispatch levels for one decision

`TableWcsResolver` (`wcs/mod.rs:391–555`) exposes 14 methods that are pure
keyword spellings (`pixel_axis_key`/`vector_axis_key`,
`pixel_matrix_key`/`vector_matrix_key`, `pixel_parameter_key`/
`vector_parameter_key`, …). `TableWcs` (`:583–719`) then re-dispatches each of
them on `PixelList` vs `ArrayColumn`. Three supporting enums
(`TableAxisKeyword`, `TableMatrixKeyword`, `TablePoleKeyword`, `:296–389`) exist
only to return static string pairs.

Collapse to one const table plus a single `TableWcs::key(family, index)`. That
~330-line block should roughly halve, and adding a keyword family becomes one
table row.

### 8.3 `vector_axis_present` is a performance and complexity outlier

`wcs/mod.rs:529–554` probes, per candidate axis: 6 axis keywords, 100 × 2 × 2
parameter keywords, and 99 × 2 × 2 matrix keywords — ~800 header lookups. It is
called for axes 99 down to 1 until one hits (`:1112–1118`), so inferring the rank
of a one-axis array column costs on the order of 80k hash lookups.

`infer_image_axis_count` (`:1552`) already solves the identical problem the right
way: one pass over `header.iter()`, taking the max parsed index. Rewrite
`vector_axis_present` the same way and the helper disappears along with the
`(1..=99)` / `(0..=99)` scans.

### 8.4 Smaller WCS items

- `first_real(header, a, b)` (`:1621`) exists for two-keyword fallbacks, but the
  text equivalent is open-coded twice: `RADESYS`/`RADECSYS` in `celestial_frame`
  (`:1283–1286`) and `RESTFRQ`/`RESTFREQ` in `spectral_rest` (`:1336–1339`).
  Add `first_text`.
- `wcs/axis.rs`: `convert` (`:645`) and `conversion_derivative` (`:706`) are
  parallel 12-arm `(from, to)` matches, both ending in
  `unreachable!("all … covered")`. Returning both from one match (or pairing
  them in a table) removes one `unreachable!` and guarantees they stay in step —
  a value/derivative pair drifting apart is a silent numerical bug.

---

## Batch 9 — Mechanical cleanups

Small, independent, low risk. Good filler for the end of a session.

1. **Row-offset arithmetic, three copies.** The
   `data_offset + row × row_len` double-`checked_*` chain appears at
   `reader/mod.rs:663–668`, `:697–702`, `:831–837`. Extract
   `fn row_offset(base: u64, row: usize, row_len: usize) -> Result<u64>`.
   The `data_offset + heap_range.start` chain appears twice (`:752`, `:854`).

2. **`verify_checksum`'s duplicated status match.** `reader/mod.rs:978–988` — the
   `datasum` and `checksum` initial-status blocks are identical; one helper
   `fn initial_status(stored: Option<&str>) -> ChecksumStatus`.

3. **`AsciiColumnReader::raw`'s two `unreachable!` loops.** `ascii/mod.rs:261–286`
   — two near-identical loops each with an `unreachable!` arm, because
   `parse_numeric_field` (`:318`) returns an untyped `ParsedNumeric`. Split it
   into `parse_integer_field` / `parse_float_field`, or have `physical` be the
   only `ParsedNumeric` consumer. Removes both `unreachable!`s.

4. **`TimeReferencePosition::parse`.** `time/mod.rs:432–451` — 15 `match` guard
   arms of `value.starts_with("XXX")`. A `const` `[(&str, TimeReferencePosition)]`
   table plus a `find` is half the length and self-evidently exhaustive.

5. **Sign-flip constants and helpers.** `data/mod.rs:559–597` — four `f64`
   offsets, four integer sign masks, four `flip_*` and four `store_*` const fns.
   A three-line macro or a `SignFlip` trait cuts ~35 lines. Low value; do it only
   if Batch 1's `Sample` trait lands, since it fits naturally there.

6. **Naming consistency.** `ColumnData::element_count` vs `AsciiColumnData::len`
   vs `ImageData::len` for the same question. Pick one.

7. **`Header::set_card` clones to validate.** `header/mod.rs:337–351` clones the
   existing card, mutates the clone, validates, then assigns. Validating the
   proposed `(keyword, value, comment)` directly avoids a `String` clone per
   `set` on an existing keyword — and `set_internal` is called dozens of times
   per written HDU.

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
- **`wcs/projection/`** — the `PROJECTIONS` membership table and `Family` enum
  already do exactly the consolidation this document recommends elsewhere.

---

## Suggested order

1. **Batch 6** (writer) — small and self-contained; 6.1 is a five-minute fix.
2. **Batch 4** (decode path) — the largest remaining structural win.
3. **Batch 5** (odometer), **Batch 7** (keywords) — independent.
4. **Batch 8** (WCS) — largest but most isolated; 8.3 is a standalone
   performance fix worth doing regardless.
5. **Batch 9** — filler.
6. **Batch 3** (errors) — **demoted.** See the note in that section: on a closer
   look its line-count case is much weaker than estimated, and the typed-enum
   version that preserves pattern-matching is close to line-neutral. Worth doing
   only for 3.3/3.4, and only if the top-level enum's size is bothering you.

## Note on this file

`Cargo.toml`'s `exclude` list keeps `docs/`, `tests/`, `AGENTS.md`, `CLAUDE.md`,
and `todo.txt` out of the published tarball, but not this file. Either add
`simplification-plan.md` to `exclude` or delete it once the work lands (as the
previous plan was).
