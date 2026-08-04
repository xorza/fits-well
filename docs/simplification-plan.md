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
