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

**This pass is complete.** One micro is left, and it is explicitly not worth
doing on its own; everything else either landed or was examined and deliberately
left alone.

The "deliberate designs left alone" section below is now the larger half of this
document, and the more useful one. Several items in the original review turned out
to describe code whose similar *shape* was load-bearing — a factorization, a
performance property, or a distinction the types were carrying. Each is recorded
with the reason, so a later pass does not re-propose it. Read that section before
adding anything new here.

---

## Remaining — one micro

**Sign-flip constants and helpers.** `data/mod.rs:542–580` — four `f64` offsets,
four integer sign masks, four `flip_*` and four `store_*` const fns. A three-line
macro cuts ~35 lines. Low value: the current spelling is at least explicit. Only
worth doing if you are in that file anyway.

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
- **The Table-22 keyword layer** (`wcs/mod.rs`) — an earlier revision of this
  document called it a "double dispatch". It is not: no `TableWcsResolver` method
  matches on `TableWcsForm` (`TableWcs` decides the form exactly once), and the
  resolver's `pixel_*`/`vector_*` methods are form-specific *spellings* named
  accordingly, not a second decision. The resolver is also used directly outside
  `TableWcs` — `pole_real`, `column_key`, `vector_rank`, `vector_axis_present` —
  so it cannot fold away. Inlining the five `pixel_*` methods into the `TableWcs`
  arms that call them just moves the same lines.
- **`TimeReferencePosition::parse`** (`time/mod.rs`) — 15 `value.starts_with(..)`
  guard arms. A const table plus a `find` is not shorter once the table is
  written, and the enum's `Other(String)` variant makes it non-`Copy`, so the
  lookup would have to clone. The guard arms read fine.
- **`ColumnData::element_count` vs `AsciiColumnData::len`** — not an inconsistency.
  They count different things: `element_count` is the flattened element total
  across all rows (an array column has `repeat` per row), while an ASCII column is
  always scalar (§7.2) so its length *is* its row count. Both doc comments say so.
  Renaming either would lose the distinction.
- **`Header::set_card`'s validate-then-assign clone** — the clone is what makes
  "an error leaves the header unchanged" obviously true. Replacing it with
  `mem::replace` plus a manual restore-on-error saves one `String` clone on a path
  that is not hot (repeated `set` of an already-present keyword), and trades an
  obviously-correct implementation for one where a forgotten restore is a silent
  bug.
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

The work this file planned is done. What is worth keeping is the "deliberate
designs left alone" record — consider moving it into `AGENTS.md`/`CLAUDE.md` (both
already excluded from the published tarball) and deleting this file, as the
previous plan was.

Until then, note that `Cargo.toml`'s `exclude` list keeps `docs/`, `tests/`,
`AGENTS.md`, `CLAUDE.md`, and `todo.txt` out of the published tarball, but not
this file.
