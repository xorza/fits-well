//! The in-memory sample paths: `cargo bench --features bench --bench decode`.
//! For SIMD codegen (a non-portable binary), add `RUSTFLAGS="-C target-cpu=native"`.

use criterion::{criterion_group, criterion_main};
use fits_well::bench;

criterion_group!(benches, bench::decode, bench::encode, bench::physical);
criterion_main!(benches);
