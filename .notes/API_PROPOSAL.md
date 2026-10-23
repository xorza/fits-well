# fits-well API proposal

2026-10-23. This document puts every finding of `API_REVIEW.md` and every entry of `ISSUES.md` into one design and one plan of work. The review keeps the evidence: the `file:line` references and the comparison with other libraries. Here, each item names the review section that it answers.

This proposal is temporary. When a phase lands, its decisions go into `docs/design.md`, and the items that it closes are removed from here and from `ISSUES.md`. Phase 1, the quiet traps, phase 2, the image reader, phase 3, writing, phase 4, header and HDU selection, and phase 5, tables and names, have landed.

## Principles

- **One borrowed image type for reads and writes.** The reader gives an `image::Borrowed`, the writer takes one, and `Image::view()` makes one from owned data. Then a caller never copies data only to satisfy a signature.
- **Parse at the boundary, one time.** Keywords, HDU names and WCS alternates are types, checked when they are made. A header lookup cannot fail because of the spelling of its argument.
- **One entry point for each operation.** A region selects the part of an image, a selector selects an HDU or a column, and a config selects the compression. No suffix such as `_view` or `_summed` multiplies the methods.
- **The caller owns the large buffers. The reader owns only small staging buffers.** The documented decision of the view read stays: `ImageReader::view` fills the caller's `words`.
- **No quiet result.** An operation that cannot do what its name says returns an error. It does not return the container, do nothing, or return `None`.

## The design

### Checksums

The review section that this answers: "Verdict helpers".

- `ChecksumReport::is_valid()`: both checksums are `Valid`.
- `ChecksumReport::has_invalid()`: either one is `Invalid`.

A caller's policy for `Absent` and `Unknown` stays its own.

### WCS

The review sections that this answers: "Best-effort access", "Batch transforms" and "The `alt` argument".

- **`WcsAlt`, a checked newtype from `TryFrom<char>`, for `A` to `Z`.** `Option<WcsAlt>` replaces `Option<char>` in `Wcs`, `PhaseAxis` and `read_wcs`.
- **`pixel_to_world(&self, pixel: &[f64], world: &mut [f64]) -> Result<()>`** and the same for `world_to_pixel`. This is a better fix than a batch API. A point costs no allocation, and a caller loops over its points with one buffer.
- **lumos can use `Wcs::from_header` now** for its `CTYPEn`/`CRVALn` lookup. The parse succeeds with unsupported axes, which only the transforms refuse. fits-well needs no change. lumos checks this on its fixtures, and it keeps its hand lookup only for a header that `from_header` refuses.

## Findings not to address

| Finding | Why not |
|---|---|
| Section stride | No caller. A preview can read full-resolution chunks and decimate them. Stride complicates the plain and the tiled section paths. |
| ASCII partial reads | No caller. ASCII tables are a legacy format, and every reader of them reads the whole table. |
| A `serde` feature | lumos's copy of `SampleType` keeps lumos's file format separate from fits-well's enum, so a fits-well rename cannot break saved files. It would also add a dependency. |
| A general split of the header borrow from the data borrow | The image handle covers the read path, because `ImageReader::header` outlives the handle borrow. The other clone (`product/mod.rs:150`) copies one header for each file, which is small and not on a hot path. |
| Verify while reading a bundle | lumos verifies before it trusts a byte, on purpose. |
| A helper for the row chunks of an N-D plane | The ranges are a stack array. lumos's two copies of this code are lumos's own duplication. |

## Plan

Each phase is one change to fits-well and its lumos migration. No phase keeps an old form: the workspace takes no backward compatibility, and lumos is the only caller.

**Verification at the end of each phase:**
- In fits-well, run `cargo drive gate`.
- From the Darkroom root, run `cargo drive check` and `cargo drive test`.
- Run `cargo drive surface` in fits-well, and review its diff as the record of the API change.

### Phase 6: the small items

1. `ChecksumReport::is_valid` and `has_invalid`.
   - lumos: `calibration_bundle/mod.rs:178` and `fits_load_config.rs:82-93`.
2. **The scratch a region read holds, for lumos's memory plan.** lumos plans its memory from an `Hdu`, before any handle exists, and its model of a compressed section (two times the stored unit) is an estimate, not a count. So the query belongs on `Hdu`, and it is added only with a measurement of what a compressed section decode holds at its peak. Until then, lumos keeps its own model, which now reads the padded size from `Hdu::padded_data_bytes`.
3. **lumos's fused conversion.** `pixels/mod.rs` converts, divides and locates nulls in one pass per chunk. It moves to `ImageReader::physical_into` and a second pass over the chunk only if a pinned bench shows no loss.
4. `WcsAlt`, and `pixel_to_world` / `world_to_pixel` into a caller slice.
5. lumos tries `Wcs::from_header` for `celestial_reference`, as described in "WCS" above.
6. Delete this proposal and `API_REVIEW.md`. Their decisions are in `docs/design.md`.

## Order and size

| Phase | fits-well | lumos | Depends on |
|---|---|---|---|
| 6. Small items | small | small | none |
