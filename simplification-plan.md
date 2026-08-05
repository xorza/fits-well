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

## Batch 1 — WCS: collapse the Table-22 double dispatch

`TableWcsResolver` (`wcs/mod.rs:392–555`) exposes 14 methods that are pure keyword
spellings, in `pixel_*`/`vector_*` pairs (`pixel_axis_key`/`vector_axis_key`,
`pixel_matrix_key`/`vector_matrix_key`, `pixel_parameter_key`/
`vector_parameter_key`, …). `TableWcs` (`:569–719`) then re-dispatches each pair on
`PixelList` vs `ArrayColumn` — so the same one decision is made twice, at two
layers.

What is reducible is that second layer: fold each resolver method pair into the
`TableWcs` arm that already chooses between them, and the resolver disappears.
Three supporting enums (`TableAxisKeyword`, `TableMatrixKeyword`,
`TablePoleKeyword`, `:297–389`) exist only to return static string pairs and can
become one const table.

What is **not** reducible is the key shapes themselves — an earlier draft of this
item proposed "one const table plus a single `TableWcs::key(family, index)`", which
does not work. The families are genuinely different templates:
`T{root}{column}{a}` vs `{axis}{root}{column}{a}`, `{root}{row}_{input}{a}` vs
`{row}{input}{root}{column}{a}`, and the string-parameter family exists only in the
vector form. Any single entry point still matches on family to pick a template.

Expect the ~330-line block to shrink by roughly a third, not a half, and treat it
as a careful mechanical edit of the most keyword-dense code in the crate.

## Batch 2 — Mechanical cleanups

Small, independent, low risk. Good filler for the end of a session.

1. **`AsciiColumnReader::raw`'s two `unreachable!` loops.** `ascii/mod.rs:247–273`
   — two near-identical loops each with an `unreachable!` arm, because
   `parse_numeric_field` (`:304`) returns an untyped `ParsedNumeric`. Split it
   into `parse_integer_field` / `parse_float_field`, or have `physical` be the
   only `ParsedNumeric` consumer. Removes both `unreachable!`s.

2. **`TimeReferencePosition::parse`.** `time/mod.rs:432–451` — 15 `match` guard
   arms of `value.starts_with("XXX")`. A `const` `[(&str, TimeReferencePosition)]`
   table plus a `find` is half the length and self-evidently exhaustive.

3. **Sign-flip constants and helpers.** `data/mod.rs:542–580` — four `f64`
   offsets, four integer sign masks, four `flip_*` and four `store_*` const fns.
   A three-line macro cuts ~35 lines. Low value — the current spelling is at
   least explicit — so only worth it if you are in the file anyway.

4. **Naming consistency.** `ColumnData::element_count` vs `AsciiColumnData::len`
   vs `ImageData::len` for the same question. Pick one.

5. **`Header::set_card` clones to validate.** `header/mod.rs:337–351` clones the
   existing card, mutates the clone, validates, then assigns. Validating the
   proposed `(keyword, value, comment)` directly avoids a `String` clone per
   `set` on an existing keyword — and `set_internal` is called dozens of times
   per written HDU.

---

## Batch 3 — `FitsError`: fold the two structurally identical families

Last, and optional. `error.rs` is 671 lines with 58 variants and a 230-line
`Display`. Most of that is *precision*, not duplication, and should stay: six
distinct "wrong HDU kind" variants and five distinct "wrong column accessor"
variants each carry a specific message, a caller handles exactly the one its call
site can produce, and `Display`'s exhaustive match already stops the arms drifting
out of sync with the variants.

Two families are genuine duplication — the same variant written five and three
times over with a different noun:

### 3.1 Five index-out-of-bounds variants → one

`HduIndexOutOfBounds`, `HeaderIndexOutOfBounds`, `ColumnIndexOutOfBounds`,
`GroupIndexOutOfBounds`, `WcsAxisIndexOutOfBounds` all carry `(index, len)` and
identical semantics, differing only in two nouns per message. Fold to
`IndexOutOfBounds { indexed: Indexed, index: usize, len: usize }` with a
five-variant `Indexed`. That collapses 5 variants and ~25 `Display` lines into 1
and 5, keeps typed pattern-matching, and gives callers one place to handle "some
index was out of range". `WcsAxisIndexOutOfBounds` is 1-based — carry that in the
`Indexed` variant's message, as it already does today.

### 3.2 Three rank/count-mismatch variants → one

`ImageRegionRankMismatch`, `TileShapeRankMismatch`, `CoordinateCountMismatch` are
all `(expected, got)` with a noun → `RankMismatch { kind, expected, got }`, same
shape as 7.1.

**Use a typed nested enum, not a `&'static str` discriminant.** A string would
make callers and tests match on display wording
(`matches!(e, IndexOutOfBounds { indexed: "column", .. })`) — fragile, and out of
step with a crate that models `HduKind`, `SampleType`, and `ChecksumStatus` as
typed enums. The typed version is close to line-neutral; the win is the smaller
top-level enum and the explicit grouping, not the line count.

Blast radius measured: 31 sites for 3.1, 16 for 3.2.

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
  around them has already been hoisted out.
- **`WriteColumnData`'s `Bits` / `VlaBits` variants** (`writer/table.rs:46`) —
  they look like they should fold into `Fixed` / `Vla`, but they carry what those
  cannot. A `VlaBits` row's bit count is *per row* (`BitVec::len()`, read at
  `:353`, `:404`, `:694`), so no single `bit_count` field on `WriteColumn`
  expresses it; and an `X` column's row width is `bit_count.div_ceil(8)`, not
  `repeat × elem_size`, so `Fixed`'s meaning would become conditionally false.
  Folding moves the branch from a match arm into an `if let Some(bit_count)`
  inside the surviving arms — same branch count, worse locality.
- **`DecodeBuffer` / `DecodeSample` / `WidePlane`** (`compress/decode.rs`) — they
  look like one idea spelled three times, but each carries something the others
  cannot. `DecodeBuffer` factors 2 backing stores (owned `ImageData`, `u64`
  scratch) against 2 traversals (whole image, section): dropping it turns 2+2
  six-arm matches into 4. `DecodeSample` maps a stored type to its wide plane and
  narrowing; `WidePlane` picks the integer or float tile reconstruction. Both
  relations are real and are what let narrowing be deferred to scatter time, so no
  whole-image `i64`/`f64` buffer is ever materialized.
- **`run_decode_scatter`'s two `#[cfg]` bodies** — not accidental duplication. A
  parallel worker cannot scatter into the caller's slice, so it must hand back an
  owned per-tile buffer, and narrowing before that hand-off is what bounds a wave's
  retained memory (`decode_wave_tile_count` sizes the wave from `size_of::<D>()`).
  The serial build has no hand-off and narrows straight into the output, allocating
  nothing per tile. Collapsing them onto the parallel shape would add an allocation
  and a copy per tile to every non-`parallel` build. The per-tile decode they *do*
  share is now `TileScratchSet::decode`; the divergence is documented in place.
- **The five N-dimensional index walks** (`reader/mod.rs`, `compress/geometry.rs`,
  `compress/decode.rs` ×2, `wcs/tabular/mod.rs`) — they share a *concept*, not
  extractable code. Only two are plain flat→coords decompositions; one of those
  (`tile_into`) fuses it with `origin`/`tdims` construction, so hoisting it would
  add a scratch buffer and a second pass to a per-tile path.
  `compressed_image_tile_rows` performs the *inverse* (coords→flat) plus a box
  odometer over `[starts, ends)`. `visit_image_region_runs` and
  `scatter_region_tile` fuse decomposition with strided accumulation and a range
  filter, in near-inverse directions, and both skip axis 0 because it is the
  contiguous run. An iterator form fits four of them but forces an axis-numbering
  re-offset at two, which is a lateral move. (The plan claimed "four of the five
  repeat the same overflow chain"; it is two of five — the other three walk
  pre-validated geometry unchecked, deliberately.)
- **The three `keyword::index` wrappers** (`writer::indexed_keyword`,
  `compress::parameter_index`, `compress::table::indexed_compression_key`) — each
  adds a *different* constraint (card length ≤ 8, `1..=999`, `1..=ncols`), so they
  compose with `index` rather than duplicating it. Inlining a range-taking variant
  would spread the `"ZNAME"`/`999` constants across the two `parameter_index` call
  sites that currently share one name for them.
- **The two Z-table keyword lists** (`compress/table.rs`) — they look identical but
  differ in exactly the entries that matter: `uncompress_table` removes the
  restored `THEAP`/`CHECKSUM`/`DATASUM`, while `reject_compression_metadata`
  rejects the `ZTHEAP`/`ZHECKSUM`/`ZDATASUM` forms whose presence means the table
  is already compressed. Sharing the five common names would hide that.
- **`first_real` has no `first_text` twin.** The two text fallbacks it looks like it
  should serve (`RADESYS`/`RADECSYS` in `celestial_frame`, `RESTFRQ`/`RESTFREQ` in
  `spectral_rest`) are conditional on `alt.is_none()` — the superseded spellings are
  already eight characters, so they have no room for an alternate suffix and apply
  only to the primary description. That is a different shape from `first_real`'s
  unconditional fallback, and a generic helper taking a lazy closure reads worse
  than the explicit `if`.
- **`convert` / `conversion_derivative`** (`wcs/axis.rs`) — two parallel 12-arm
  matches over the same characteristic pairs. Merging them into one match returning
  both would make the hot transform path compute a derivative it does not need
  (`SpectralTransform::to_world` calls `convert` alone). The real risk the pairing
  poses — a formula edited in one and not the other, silently — is now covered by
  `conversion_derivatives_match_the_conversions_they_describe`, which differentiates
  `convert` numerically and holds `conversion_derivative` to it across all twelve
  pairs. A test, not a refactor, was the right tool.
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
