# fits-well simplification plan

Consolidation and simplification work found in a full read of `src/` (~15k lines of
production code). Each batch below is scoped to a single sitting: one coherent area,
one verification run, one reviewable diff. Batches are ordered by priority — earlier
ones either unblock later ones or carry the best value-to-risk ratio.

Nothing here is a defect. These are duplication, parallel type families, and dispatch
that costs more than it buys.

## Shared context

**Verification chain** (per batch, scoped to what it touches):

```
cargo fmt -p fits-well \
  && cargo clippy -p fits-well --all-targets --all-features -- -D warnings \
  && cargo test -p fits-well --lib --tests --all-features
```

`--all-features` matters: it compiles the `compression`/`internals`-gated code several
batches touch.

**Downstream.** `lumos` is the only in-workspace consumer. It imports
`FitsReader`/`FitsWriter`/`Header`/`Bitpix`/`Image`/`Scaling`/`SampleType`/
`ChecksumStatus`/`HduKind`/`SliceReader`/`StreamReader` and
`table::{ColumnData, TableBuilder, WriteColumn}`, and names exactly four `FitsError`
variants: `Io`, `KeywordOutOfRange`, `MissingKeyword`, `TypeMismatch`. Batches that
change public API add `-p lumos` to the chain; those flagged *no public impact* do not.

**Submodule constraint.** `fits-well` is pulled in as a git submodule and must stay
valid built standalone. No batch here requires a `Cargo.toml` change.

---

## Batch 2 — Cross-cutting small duplicates

**Priority: 2** · Size: small · Risk: low · No public impact

Independent one-liners, all the same character. Fast, and it clears noise before the
structural batches.

- [ ] `fits_i64` ×3 → one shared helper: `writer/mod.rs:330`, `compress/encode.rs:531`,
      `compress/table.rs:1009`.
- [ ] Image-region validator ×2, **identical bodies**: `reader/mod.rs:1074`
      (`validate_image_region`) and `compress/decode.rs:797` (`validate_region`).
- [ ] "`PREFIX` + non-zero-leading digits" predicate ×3: `writer/mod.rs:321`
      (`indexed_keyword`), `compress/table.rs:548` (`indexed_compression_key`),
      `compress/mod.rs:191` (`parameter_index`).
- [ ] Delete `data/mod.rs:174` (`map_be`) — it is exactly
      `decode_be(bytes, |x| map(decode(x)))`. Consider whether `table/mod.rs:984`
      (`map_cells`) can also route through `endian::decode_be`.
- [ ] Merge `hdu/mod.rs:143` (`random_groups_array_elements`) and `hdu/mod.rs:152`
      (`array_elements`) — they differ only in the `is_empty()` clause. Check against
      `data::shape_product`, a third copy of the same product.
- [ ] Share the ~15-line preamble between `reader/mod.rs:393` (`read_image`) and
      `reader/mod.rs:446` (`read_image_view`): kind check, bitpix, axes, scaling,
      `DataLengths`, `source.slice`.
- [ ] `reader/mod.rs:857` (`read_wcs`) inlines an EXTNAME/EXTVER/EXTLEVEL scan that
      partly re-implements `hdu_index`; reuse it (extending for EXTLEVEL).

---

## Batch 3 — Binary-table type consolidation

**Priority: 3** · Size: large · Risk: medium · Public API change (`table::ColumnType`)

The largest structural win, and contained to `table/` + `writer/table.rs`.

- [ ] **Delete `ColumnType`** (`writer/table.rs:28`) in favour of `TformKind`
      (`table/mod.rs:36`). `ColumnType`'s 10 variants are `TformKind` less `Bit`,
      `ArrayDesc32`, `ArrayDesc64`; `ColumnType::letter()` duplicates `TformKind::code()`
      and `ColumnType::elem_size()` duplicates `TformKind::elem_size()` — two identical
      10-arm tables. `WriteColumnData`'s variants already exclude bit and descriptor
      kinds structurally. **~90 lines and one enum.**
- [ ] **`BinTable { schema: TableSchema, bytes: Vec<u8> }`** (`table/mod.rs:316`). The
      struct's first five fields *are* `TableSchema` (`:343`), and `from_data`
      destructures a schema field-by-field to rebuild them.
- [ ] Unify `TDIM` validation across read and write: `table/mod.rs:966,972` vs
      `writer/table.rs:773–807` (`validate_tdim`, `validate_vla_tdim`,
      `validate_tdim_shape`, `validate_tdim_product`).
- [ ] Merge `decode_array` (`table/mod.rs:1169`) and `decode_fixed_cells` (`:1003`) —
      two complete 10-arm `TformKind` decode tables. `decode_fixed_cell` is already
      `decode_fixed_cells(once(bytes), 1, col)`.
- [ ] Extract the VLA-rejection guard copy-pasted 3× in `ColumnReader`
      (`table/mod.rs:614,630,655`).

`lumos` imports `ColumnData`, `TableBuilder`, `WriteColumn` but **not** `ColumnType`,
so the rename surfaces only if it calls `WriteColumn::vla_typed`. Verify with
`-p fits-well -p lumos`.

---

## Batch 4 — Unsigned & physical-scaling consolidation

**Priority: 4** · Size: medium · Risk: medium · No public impact (keep `SampleType`)

The single most-repeated logic in the crate: the FITS sign-bit-offset convention.

- [ ] Collapse `UnsignedKind` (`table/mod.rs:1060`) into `SampleType`
      (`data/mod.rs:542`). `unsigned_kind` and `SampleType::from_scaling` are the same
      decision written twice against different type tags.
      **Keep `SampleType` as the public name — `lumos` imports it.**
- [ ] Collapse the 12 hand-written sign flips into `UnsignedData::from_be(bytes, kind)` /
      `from_host(slice, kind)`: `data/mod.rs:209,212,215,218,630,633,636,642,719,728,739,750`
      and `table/mod.rs:1091,1093,1096,1099`.
- [ ] Deduplicate the two scaling tables: `physical_from_be` (`data/mod.rs:190`) vs
      `ImageData::physical_as` (`:139`), and `unsigned_from_be` (`:203`) vs
      `ImageData::unsigned` (`:155`). The borrowed-bytes and decoded paths both need to
      exist; the per-type table does not need to.
- [ ] `groups/mod.rs:200,211` — `elem_f64`/`elem_physical` match `ImageData` **per
      element** inside `.map()` loops (callers at `:151,169,180`) and duplicate
      `physical_as`'s table. Hoisting the match out of the loop is a simplification
      *and* a real speedup.

---

## Batch 5 — Error-variant families

**Priority: 5** · Size: medium · Risk: low · Public API change (mechanical)

16 variants across three shapes collapse to 3, with **no loss of `Display`
specificity** (carry a `&'static str` discriminator). The crate's stated
no-backward-compatibility policy makes this free. Sequenced after batches 3–4 so the
table/data error sites settle before the sweep.

- [ ] Six "wrong HDU kind" → one: `NotAnImage`, `NotABinTable`, `NotRandomGroups`,
      `NotAnAsciiTable`, `NotCompressedImage`, `NotCompressedTable` (`error.rs:163–179`).
- [ ] Five `{index, len}` out-of-bounds → one: `HduIndexOutOfBounds`,
      `HeaderIndexOutOfBounds`, `ColumnIndexOutOfBounds`, `GroupIndexOutOfBounds`,
      `WcsAxisIndexOutOfBounds`.
- [ ] Five `{code: char}` wrong-column-kind → one: `VariableLengthColumn`, `NotAVla`,
      `NotABitColumn`, `NotAComplexColumn`, `NonNumericColumn`.

~13 variants and ~60 lines of `Display` arms. **`lumos` names none of these** (only
`Io`, `KeywordOutOfRange`, `MissingKeyword`, `TypeMismatch`), and `FitsError` is
`#[non_exhaustive]` so no downstream match can be exhaustive. Most of the churn is in
this crate's own tests.

---

## Batch 6 — Compression codec enums

**Priority: 6** · Size: small · Risk: low · No public impact

- [ ] `Algo` (`compress/table.rs:31`) is `ImageCodec` (`compress/mod.rs:182`) minus
      `Plio1`/`Hcompress1`, with a duplicate `parse` and a duplicate name-string table.
      One enum, with a table-path validity check at parse time.
- [ ] `Compression::name` (`compress/mod.rs:236`) routes through `image_codec()` for the
      same strings `Algo::name` (`table.rs:39`) repeats.

---

## Batch 7 — WCS table-keyword resolver

**Priority: 7** · Size: medium · Risk: low · No public impact

- [ ] `TableWcsResolver` (`wcs/mod.rs:391`) stores `Option<char>` and then writes
      `match self.alternate { Some(a) => key!("…{a}"), None => key!("…") }` **8 times**.
      `Wcs::from_header_with_context` already solved this 300 lines earlier
      (`:592`) by materializing the suffix once and interpolating unconditionally.
      Store the suffix; delete all 8 two-arm matches.
- [ ] `from_pixel_list` (`:917`) and `from_array_column` (`:1004`) are ~80 lines each of
      near-identical "translate table keywords into a synthetic image header": same axis
      loop, same matrix loop, same pole and celestial-frame copy. Share the body.

---

## Batch 8 — ASCII column & field path

**Priority: 8** · Size: small · Risk: low · No public impact

- [ ] Merge `ascii_field` (`writer/ascii.rs:244`) and `append_ascii_field` (`:366`).
      They match `col.data` twice for one field: once to build text + alignment, again
      to re-check `values[r].is_some()` for null collision — and the `left_aligned` flag
      is then `debug_assert!`ed back. One pass returning text + alignment + was-null.
- [ ] Give `AsciiColumnData` a `len()` (mirroring `ColumnData::element_count`). The
      3-arm row-count match is written three times: `writer/ascii.rs:69` (`row_count`),
      `:163` (inlines what `row_count` already does), `:211` (`has_null`).
- [ ] `AsciiTable::field` (`ascii/mod.rs:201`) re-runs a UTF-8 check per field per row
      on every access; validate once at parse.

---

## Batch 9 — Tiled-decode dispatch

**Priority: 9** · Size: large · Risk: high · No public impact

Best readability win in the crate, and the one that needs real care. Do it when the
rest is stable.

- [ ] `decode_image_into` (`compress/decode.rs:539`) and `decode_image_section_into`
      (`:422`) are ~175 lines of nested dispatch: float/integer branch × 4–6
      `DecodeBuffer` arms × a closure differing only in arity (`t` vs
      `table_row, tile_row`) and `run_decode_scatter` vs `run_decode_region`. Every arm
      body is three tokens apart, and there are 10 `unreachable!` arms asserting what
      the type already implies.
- [ ] Collapse `DecodeBuffer` (`compress/decode.rs:197`) into the `ImageView` family as
      a mutable counterpart. It is the fourth BITPIX-parallel enum (`ImageData` owned,
      `ImageView` shared, `DecodeBuffer` mutable, plus `zeroed_samples`), and it brings
      the third `unsafe` reinterpretation match — after `swap_into_words`
      (`data/mod.rs:307`) and `view_words` (`:351`).

Make the scatter target a small trait, or pass a narrowing function keyed off `Bitpix`,
so both outer matches become one path.

---

## Batch 10 — Compressed-table VLA layout probe

**Priority: 10** · Size: small · Risk: medium · No public impact

- [ ] `decompress_vla_column` (`compress/table.rs:770`) takes 8 arguments with an
      `#[expect(clippy::too_many_arguments)]`. Bundle the heap/main/row-layout
      parameters into a struct.
- [ ] The layout probe runs `validate_vla_layout` twice and pattern-matches a nested
      `(Result, Result)` tuple (`:826–841`) to pick which error to surface — a lot of
      machinery for two outcomes. The standard-vs-cfitsio descriptor-order ambiguity is
      real; the error selection can be flattened.

---

## Backlog — opportunistic, not a sitting

Small enough to fold into whichever batch is already touching the file.

- `Card::validate()` is `self.render_into(&mut Vec::new())` (`header/card/mod.rs:231`),
  so every `Header::set`/`insert`/`comment`/`push_*` heap-allocates a throwaway `Vec`
  to validate. Render into a reusable buffer, or validate without rendering.
- `header/value.rs` has two `From` macros (`:91`, `:245`) plus hand-written
  `From<i64>`/`<i32>`/`<u32>` doing exactly what the macros generate.
- `BinTableMetadata` (`table/mod.rs:333`) and `AsciiTableMetadata` (`ascii/mod.rs:66`)
  bundle only `nrows` + `&[Column]` and appear nowhere outside `lib.rs` re-exports and
  tests; two accessor methods serve the same "don't expose owned state" goal without the
  types. (`ImageMetadata` and `RandomGroupsMetadata` bundle enough to earn their keep.)
- `writer/mod.rs:271` (`is_structural_keyword`) lists `"NAXIS"` in both the exact match
  and the indexed-prefix list — harmless, but one of them is dead weight.
