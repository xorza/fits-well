# Crate-wide architecture, simplification, and performance review

## Executive summary

The crate is not broadly overengineered. I reviewed all 37 production Rust files
(21,318 lines, excluding tests and benchmarks). Most of the size is irreducible
FITS 4.0 surface area: distinct HDU layouts, typed tables, WCS algorithms, time
metadata, and five specified compression codecs. The dependency graph is lean,
every non-core dependency is feature-gated, and `cargo tree -e normal -d` reports
no duplicate normal dependencies. The owned/raw/scratch-backed image types also
serve measurably different copy and lifetime requirements; merging them would
hide costs rather than simplify the design.

The main opportunities are narrower:

- `-TAB` WCS resolution defeats the reader's lazy table APIs and its inverse
  search can perform unbounded combinatorial work.
- FITS `P`/`Q` descriptor decoding, byte sizing, and heap validation have three
  implementations with different failure semantics.
- Image-section reads still contain avoidable temporary allocations and copies.
- Common WCS transforms allocate more vectors than their returned value requires.

No production code needs removal solely because it is uncommon: random groups,
ASCII tables, WCS variants, time forms, and compression fallbacks are all part of
the stated whole-standard contract. There are no TODOs or placeholder
implementations, and the default all-target clippy check passes with warnings
denied.

## Batch 1 — Make `-TAB` WCS lazy and computationally bounded (high priority)

- [ ] Resolve only row zero and the referenced columns of a `-TAB` table. `FitsReader::read_wcs` materializes the entire extension at `src/reader/mod.rs:881`, while `first_row_shape_and_values` decodes every VLA row at `src/wcs/tabular/mod.rs:613` or every fixed-width row at `src/wcs/tabular/mod.rs:620` and then retains only the first. Rework the bridge to use the existing `read_table_columns(table_index, 0..1, ...)` path, deduplicate coordinate/index column selectors per extension, and construct `TabularTransform` from that selection. Validate with a multi-row table containing large unrelated columns and a counting `Read + Seek`: transformed coordinates must remain exact while reads are limited to row zero and its selected heap cells.

- [ ] Bound multidimensional inverse-search work. Each `locate_subvoxel` node loops over all interpolation vertices at `src/wcs/tabular/mod.rs:515`, and every iteration calls `interpolate` at `src/wcs/tabular/mod.rs:530`, which loops over all vertices again at `src/wcs/tabular/mod.rs:380`; a node is therefore quadratic in vertex count before it branches into every subdivision at `src/wcs/tabular/mod.rs:563`. The depth limit at `src/wcs/tabular/mod.rs:555` limits stack depth, not total work. Add an explicit interpolation-evaluation budget and a dedicated non-convergence error, then cache/reuse corner evaluations within a node so bracketing is not a full nested vertex sweep. Validate exact inverses for 2-D through at least 6-D tables and assert prompt bounded failure for folded or adversarial arrays that keep every subdivision plausible.

- [ ] Allocate one inverse-search workspace per top-level transform instead of per vertex and recursion level. `interpolate` allocates result and index vectors at `src/wcs/tabular/mod.rs:378`, `locate_subvoxel` allocates three flag vectors plus a delta vector at `src/wcs/tabular/mod.rs:512`, and every child allocates another voxel vector at `src/wcs/tabular/mod.rs:564`. Introduce `interpolate_into` and a `TabularSearchScratch` containing indices, coordinates, flags, deltas, and the mutable voxel path; pass slices through the search. Add an allocation-counting benchmark for multidimensional `world_to_pixel` and require allocations to stay constant with search depth.

## Batch 2 — Establish one canonical `P`/`Q` descriptor path (high priority)

- [x] Replace the three descriptor types and decoders with one crate-internal `PqDescriptor` in `table/descriptor.rs`. The normal table path defines `Descriptor`/`decode_descriptor` at `src/table/mod.rs:1257`, source-bound row reads define `ArrayDescriptor`/`decode_descriptor` at `src/reader/mod.rs:980`, and compressed tables define `VlaDescriptor`/`read_vla_descriptor` at `src/compress/table.rs:710`. Their behavior differs: two saturate out-of-range values while the compression path returns `DataUnitOverflow`. Route all three callers through one checked decoder and keep width validation there. Validate the same malformed descriptor corpus through `read_table`, `read_table_cell`, ranged reads, and compressed-table decode, asserting identical results.

- [x] Decode descriptor fields as the signed integers FITS specifies and reject negative count/offset values directly. The table path currently uses `u32`/`u64` at `src/table/mod.rs:1281`, the source-bound path does the same at `src/reader/mod.rs:1027`, and compressed tables do so at `src/compress/table.rs:721`; they rely on later overflow or heap bounds to turn negative signed fields into an error. Pair the new reader with the existing signed-range writer validation at `src/endian.rs:62`, returning one explicit invalid-descriptor error before size arithmetic. Test `-1`, signed maxima, one-past-signed-max bit patterns, and zero-length descriptors for both `P` and `Q`.

- [x] Centralize VLA payload sizing and checked heap-span construction. Bit-array rounding and scalar multiplication are repeated at `src/table/mod.rs:1243`, `src/reader/mod.rs:1003`, and `src/compress/table.rs:743`; heap bounds are separately implemented at `src/table/mod.rs:543`, `src/reader/mod.rs:1011`, and `src/compress/table.rs:758`. Put `payload_len(element_kind)` and `heap_range(heap_start, heap_end)` on the canonical descriptor (or adjacent helpers), leaving the compressed layout probe to translate errors into a rejected candidate. Cross-check exact spans for empty, one-bit, byte-boundary, maximum in-range, aliased, and out-of-range cells.

## Batch 3 — Finish the allocation-reuse story for image sections (medium priority)

- [x] Stop allocating an owned row for every selected compressed-image tile. `read_table_sparse_rows` preallocates the compact destination at `src/reader/mod.rs:656` but then calls `read_owned` for every row at `src/reader/mod.rs:673` and copies that fresh vector into the destination. Fetch each row with `Source::slice` into the reader's existing scratch and copy once into its final slot. Benchmark sparse sections over both `StreamSource` and `SliceSource`; allocation count should not grow with selected tile count and bytes must match the current output exactly.

- [x] Give the owned compressed-section API a direct typed destination. `read_image_section` creates a `Vec<u64>` at `src/reader/mod.rs:494`, decompresses the complete section into it, then `to_owned_data` allocates and copies the same plane again at `src/reader/mod.rs:496`. Factor the region scatter behind `DecodeBuffer`, as the full-image path already does at `src/compress/decode.rs:203`, and add an owned `decompress_image_section` that allocates `ImageData` once; keep `decompress_image_section_into_words` as the reuse-oriented wrapper over the same core. Validate exact equality between owned and view paths for every `BITPIX`, scaling/null handling, partial edge tiles, and empty sections.

- [x] Generate plain-image byte runs while reading instead of storing the full run plan. `read_plain_image_section` obtains a `Vec<ByteRun>` at `src/reader/mod.rs:554`, and `image_region_runs` reserves up to one entry per higher-dimensional selected row at `src/reader/mod.rs:1132` before the section buffer is filled. Fold the coordinate iteration and adjacent-run coalescing into the read loop (or expose a non-allocating iterator), and replace the infallible metadata-sized reserve at `src/reader/mod.rs:559` with `allocation::try_reserve`. Test highly strided N-D sections where the run count is large, and compare exact source seek/read ranges as well as output bytes.

## Batch 4 — Remove redundant allocations from ordinary WCS transforms (medium priority)

- [x] Eliminate the standalone pixel-offset vector in `pixel_to_world`. The method builds `offset` at `src/wcs/mod.rs:1867`, allocates `intermediate` through `matvec` at `src/wcs/mod.rs:1872`, and then allocates the required returned world vector. Add a matrix helper that subtracts `CRPIX` while accumulating each row, so only `intermediate` and the returned vector remain. Preserve exact results for linear, celestial, spectral, and tabular axes and measure allocations in the existing WCS benchmark harness.

- [x] Reuse the matrix-product vector as the return value in `world_to_pixel`. `matvec` allocates `offset` at `src/wcs/mod.rs:1929`, then the iterator at `src/wcs/mod.rs:1930` allocates another vector solely to add `CRPIX`. Add `CRPIX` to `offset` in place and return it. Round-trip tests should remain bit-for-bit identical for the existing fixtures, with one fewer allocation per call.

- [x] Avoid full-rank temporary vectors when resolving one tabular axis. `axis_world` builds pixel offsets, a matrix product, and a `world` vector sized to every WCS axis at `src/wcs/mod.rs:1647`, even though it returns one value at `src/wcs/mod.rs:1655`. Reuse the fused matrix helper and let `TabularTransform` return the requested member from its interpolation workspace rather than filling an `naxis`-sized output. Cover time-axis calls through `FitsTime::time_axis_mjd`, including a tabular axis embedded in a higher-rank WCS.

## Batch 5 — Split only the two modules whose responsibilities already have clean seams (low priority)

- [x] Turn `writer/mod.rs` into a coordinator rather than a 1,972-line implementation unit. Writer state/commit/checksum mechanics start at `src/writer/mod.rs:497`, image header/stream logic at `src/writer/mod.rs:914`, binary-table encoding at `src/writer/mod.rs:1217`, and ASCII-table encoding at `src/writer/mod.rs:1685`. Move these existing groups to `writer/{mod.rs,image.rs,table.rs,ascii.rs}` without introducing generic builder abstractions; the small amount of duplication between binary and ASCII builders is clearer than a shared type-state framework. Validate with the full suite and confirm the public surface remains defined only by `lib.rs`.

- [x] Separate projection kernels from WCS metadata parsing/orchestration. `wcs/mod.rs` contains the public projection catalog and kernels beginning at `src/wcs/mod.rs:64`, frame/model types beginning at `src/wcs/mod.rs:888`, header resolution beginning at `src/wcs/mod.rs:1330`, and transform execution at `src/wcs/mod.rs:1858`. Move projection-specific code to `wcs/projection/mod.rs` (alongside the existing cube and HEALPix implementations) and keep WCS assembly/execution in `wcs/mod.rs`; do not add a trait hierarchy because dispatch is closed and exhaustive. Run all projection golden/round-trip tests after the move and verify no new `pub` surface is introduced.

- [x] Correct the stale compressed-table reader documentation while touching these boundaries. `src/reader/mod.rs:915` still says only fixed-width columns are supported, whereas the decoder handles VLA layouts in `src/compress/table.rs:804` and the writer documents `P`/`Q` support at `src/writer/mod.rs:840`. Update the public docs and add a doctest or existing-test assertion that reads a compressed VLA table through `read_compressed_table`.

## Open questions

- [ ] Benchmark the header side index before attempting to simplify it. `Header` duplicates every valued keyword into a `HashMap<String, usize>` and must rebuild it after structural edits at `src/header/mod.rs:32` and `src/header/mod.rs:444`; a linear scan would remove state and mutation bookkeeping, but may regress 999-column table parsing and WCS's many indexed lookups. Compare parse/get/mutation workloads for typical headers and standard maxima; retain the index unless linear lookup wins or is acceptably close across both.

- [ ] Confirm whether the distribution should remain batteries-included. `parallel` is the default and implies `compression` at `Cargo.toml:49`, while `--no-default-features` already supplies the lean core and all added dependencies are optional at `Cargo.toml:42`. This is a product/defaults decision rather than a code-quality defect; change it only with downstream build-time or binary-size evidence, not to reduce the manifest cosmetically.
