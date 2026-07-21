use std::alloc::{GlobalAlloc, Layout, System};
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};

use criterion::{Criterion, Throughput, criterion_group, criterion_main};
use fits_well::internals::{
    linear_wcs_round_trip_batch, prepare_wcs_benchmarks, spectral_wcs_batch,
    tabular_forward_at_pixel, tabular_inverse_at_fraction, tabular_inverse_at_world,
    tabular_wcs_batch,
};

static ALLOCATIONS: AtomicUsize = AtomicUsize::new(0);

#[derive(Debug)]
struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        ALLOCATIONS.fetch_add(1, Ordering::Relaxed);
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

fn allocation_count(run: impl FnOnce() -> f64) -> usize {
    ALLOCATIONS.store(0, Ordering::Relaxed);
    black_box(run());
    ALLOCATIONS.load(Ordering::Relaxed)
}

fn assert_inverse_allocations_are_depth_independent() {
    black_box(tabular_inverse_at_fraction(0.5));
    black_box(tabular_inverse_at_fraction(0.5 + 2.0_f64.powi(-20)));
    let shallow = allocation_count(|| tabular_inverse_at_fraction(black_box(0.5)));
    let deep = allocation_count(|| tabular_inverse_at_fraction(black_box(0.5 + 2.0_f64.powi(-20))));
    assert_eq!(shallow, deep, "TAB inverse allocations grew with depth");
}

fn assert_one_dimensional_tabular_allocation_counts() {
    let pixel = 12_345.25;
    let world = tabular_forward_at_pixel(pixel);
    black_box(tabular_inverse_at_world(world));
    let forward = allocation_count(|| tabular_forward_at_pixel(black_box(pixel)));
    let inverse = allocation_count(|| tabular_inverse_at_world(black_box(world)));
    assert_eq!(forward, 4, "TAB forward allocation count regressed");
    assert_eq!(inverse, 4, "TAB inverse allocation count regressed");
}

fn wcs(c: &mut Criterion) {
    prepare_wcs_benchmarks();
    assert_inverse_allocations_are_depth_independent();
    assert_one_dimensional_tabular_allocation_counts();
    let mut group = c.benchmark_group("wcs");
    group.throughput(Throughput::Elements(1024));
    group.bench_function("linear_4d_round_trip", |bench| {
        bench.iter(|| black_box(linear_wcs_round_trip_batch()))
    });
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
