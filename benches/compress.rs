//! The tiled codecs: `cargo bench --features bench,compression --bench compress`.

use criterion::{criterion_group, criterion_main};
use fits_well::bench;

criterion_group!(
    benches,
    bench::decompress,
    bench::compress,
    bench::decompress_table,
    bench::compress_table
);
criterion_main!(benches);
