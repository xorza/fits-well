use std::hint::black_box;

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use fits_well::internals::{prepare_wcs_benchmarks, spectral_wcs_batch, tabular_wcs_batch};

fn wcs(c: &mut Criterion) {
    prepare_wcs_benchmarks();
    let mut group = c.benchmark_group("wcs");
    group.throughput(Throughput::Elements(1024));
    group.bench_function("spectral", |bench| {
        bench.iter(|| black_box(spectral_wcs_batch()))
    });
    group.bench_function("tabular_index_100k", |bench| {
        bench.iter(|| black_box(tabular_wcs_batch()))
    });
    group.finish();
}

criterion_group!(benches, wcs);
criterion_main!(benches);
