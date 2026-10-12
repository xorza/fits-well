//! WCS transform throughput: `cargo bench --bench wcs`.

use criterion::{criterion_group, criterion_main};
use fits_well::bench;

criterion_group!(benches, bench::wcs);
criterion_main!(benches);
