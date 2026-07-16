# Recent changes review

## Executive summary

This review compares the complete production tree at `d6a3f17` (the last commit
before the July 15 code burst) with `68c8e30` (`HEAD`). The production code grew
by a net 1,632 lines across non-test source files, while source clone density fell
from 1.03% to 0.92%. The current default and no-default-feature test suites both
pass.

The burst should not be reverted wholesale. Most of the growth closes real
correctness gaps or removes full-size intermediate buffers. Keep the codec bounds
checks, writer invariant types, `VlaCell`, direct raw/physical conversions,
per-worker codec scratch, streaming checksum carry, and WCS domain/convergence
errors.

The parts worth removing or consolidating are narrower:

| Area | Recommendation | Reason |
| --- | --- | --- |
| Parallel output scatter | Remove | Adds a broadly unsound raw-pointer abstraction and contradicts the documented map/serial-fold design. |
| Fallible allocation layer | Narrow | It is applied to caller-owned writer data yet remains incomplete on the paths whose OOM recovery it advertises. |
| Header typed getters | Collapse | Two public getter families coexist, while critical semantic parsers still use the permissive one. |
| WCS allocation/caching refactor | Measure before changing | Correctness changes are valuable, but no workspace production caller or WCS benchmark currently justifies the performance-specific API churn. |

## Batch 1 — Remove raw-pointer parallel scatter (high priority)

The documented compression architecture says to map codec work in parallel and
fold/scatter serially ([`AGENTS.md:193`](AGENTS.md#L193)). The recent table change
instead generalized the existing image-only raw pointer into `DisjointSlice<T>`,
added a second tile fan-out abstraction, and wrote table cells concurrently into a
shared allocation.

- [x] Delete `try_for_each_tile` and `DisjointSlice` from [`src/compress/mod.rs:189`](src/compress/mod.rs#L189), and make `map_tiles` the single parallel primitive again. `unsafe impl<T> Sync` at [`src/compress/mod.rs:220`](src/compress/mod.rs#L220) is too broad because it has no `T: Send` bound; its methods can move a non-`Send` value into storage later observed on another thread. Even with that bound, every call site must manually prove a global non-aliasing partition. Preserve the useful zero-copy `VlaCell`/`VlaColumn` work, return decoded tile buffers from the parallel map, then scatter them through ordinary mutable slices. Apply this to image scatter at [`src/compress/decode.rs:306`](src/compress/decode.rs#L306) and table scatter at [`src/compress/table.rs:323`](src/compress/table.rs#L323). Verify identical image/table round trips with default parallel features and `--no-default-features --features compression`, then compare the existing compression benchmarks before accepting any throughput loss. If serial table folding is material, parallelize by disjoint row chunks with safe `par_chunks_mut` rather than restoring a generic raw pointer.

## Batch 2 — Make typed metadata parsing one coherent API (high priority)

Commit `4db37a2` introduced strict getters and claims wrong metadata types now fail
throughout the crate, but the old permissive family remains alongside it at
[`src/header/mod.rs:102`](src/header/mod.rs#L102). Core parsers still use the old
family: `BITPIX`/`NAXIS` at [`src/header/mod.rs:168`](src/header/mod.rs#L168),
`PCOUNT`/`GCOUNT` at [`src/header/mod.rs:206`](src/header/mod.rs#L206), table layout
at [`src/table/mod.rs:338`](src/table/mod.rs#L338), compressed-image layout at
[`src/compress/decode.rs:54`](src/compress/decode.rs#L54), and time defaults at
[`src/time/mod.rs:515`](src/time/mod.rs#L515). A mistyped `PCOUNT` or `GCOUNT` is
therefore still treated as absent and can produce a wrong HDU boundary. The
changelog's crate-wide claim at [`CHANGELOG.md:39`](CHANGELOG.md#L39) is not true.

- [ ] Replace `get_logical`/`get_integer`/`get_real`/`get_text` plus the parallel `try_get_*` family at [`src/header/mod.rs:118`](src/header/mod.rs#L118) with one fallible typed family returning `Result<Option<_>>`; raw callers can continue to inspect `Header::get`. Rewrite every semantic parser to distinguish absence, wrong type, and out-of-range values consistently. Prefer small `required_*`/defaulting helpers at the semantic boundary rather than repeating `.ok_or(...)` chains. Add malformed-type fixtures for structural, table, compression, WCS, and time keywords, asserting exact `TypeMismatch` results, then correct the changelog.

## Batch 3 — Narrow the fallible-allocation policy (medium priority)

The new allocation module wraps four ordinary `Vec` operations
([`src/allocation.rs:6`](src/allocation.rs#L6)) and maps every reserve failure to
`DataUnitTooLarge`, whose documentation says the size came from a hostile header
([`src/error.rs:48`](src/error.rs#L48)). The wrappers are also used for writer
buffers and caller-owned data, where that description is false. Meanwhile, header
rendering reserves only one record per model card at
[`src/writer/mod.rs:53`](src/writer/mod.rs#L53), although a long string can expand
into many `CONTINUE` records; subsequent `extend_from_slice` calls can still abort
infallibly. Similar infallible growth remains inside codec scratch buffers. The
result is an OOM-recovery facade rather than a dependable boundary.

- [ ] Restrict fallible allocation to output sizes derived directly from untrusted FITS metadata, chiefly decompressed image/table planes and streaming-source staging. Remove `try_copy` and the writer/caller-owned uses of the module; collapse the remaining helpers to the minimum operation needed to reserve a validated final size. Keep checked arithmetic independently. Either make the remaining guarantee complete or stop claiming crate-wide recoverable OOM behavior at [`CHANGELOG.md:61`](CHANGELOG.md#L61).
- [ ] Change the incremental header scan at [`src/reader/mod.rs:489`](src/reader/mod.rs#L489) from `try_reserve_exact` on every 2,880-byte block to geometric `try_reserve`. Exact growth is suitable when the final size is known; here it can reallocate and copy once per block for long headers.
- [ ] Do not mutate `has_primary` before the new fallible reserve in [`src/writer/mod.rs:303`](src/writer/mod.rs#L303). A reserve failure currently returns before writing any bytes but leaves the writer believing a primary HDU exists, so a retry emits an extension first. Commit writer state only after all recoverable pre-write work succeeds, and add a failure-injection test around that state transition.

Verification: retain exact overflow/error assertions for hostile dimensions, add a
multi-block-header capacity test, and exercise writer reuse after an injected
pre-write allocation failure.

## Batch 4 — Remove release checks from documented hot paths (low priority)

- [ ] Restore debug-only invariant checks in the image hot path. [`src/reader/mod.rs:294`](src/reader/mod.rs#L294) explicitly describes an internal invariant but uses release `assert_eq!`, and [`src/compress/decode.rs:109`](src/compress/decode.rs#L109) uses a release `assert!` immediately before the direct-to-scratch decode. Both sizes were already established by checked layout/allocation code, so use `debug_assert!`/`debug_assert_eq!` and retain the checked errors at the external-input boundary.

Verification: run the default and serial-compression suites in both debug and
release builds; malformed external dimensions must still return their existing
checked errors before reaching these invariants.

## Open question

- [ ] Is WCS transformation intended to be a bulk per-pixel workload or occasional metadata conversion? The workspace has no production call to `pixel_to_world`/`world_to_pixel` and no WCS benchmark, while the performance refactor added `PreparedProjection` at [`src/wcs/mod.rs:180`](src/wcs/mod.rs#L180) and made caller-owned output mandatory at [`src/wcs/mod.rs:1239`](src/wcs/mod.rs#L1239). Keep the projection-domain and convergence errors regardless. If bulk WCS is intended, add a Criterion benchmark and retain the pieces it validates; if not, prefer the simpler allocating API and remove unmeasured caches rather than carrying a performance-oriented public surface without a workload.

## Verification performed

- `cargo test`: 263 passed, 2 ignored; 5 doctests passed, 1 ignored.
- `cargo test --no-default-features`: 217 passed.
- Full production-tree comparison from the disposable `d6a3f17` checkout.
- Clone analysis excluding split test files: 1.03% duplicated lines before, 0.92% now.
