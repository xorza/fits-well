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
