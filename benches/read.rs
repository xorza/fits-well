//! The read paths: `cargo bench --features bench --bench read`, with `mmap` for its
//! arm and `compression` for the compressed sections.

use criterion::{criterion_group, criterion_main};
use fits_well::bench;

#[cfg(feature = "compression")]
criterion_group!(
    benches,
    bench::read_image,
    bench::read_image_view,
    bench::read_compressed_image_section
);
#[cfg(not(feature = "compression"))]
criterion_group!(benches, bench::read_image, bench::read_image_view);
criterion_main!(benches);
