//! Allocation counts of the `-TAB` transforms, under an allocator that counts the
//! calling thread's allocations only, so the harness's other threads do not reach
//! the count.

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use fits_well::internals::{
    tabular_forward_at_pixel, tabular_inverse_at_fraction, tabular_inverse_at_world,
};

thread_local! {
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}

#[derive(Debug)]
struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn count() {
    // A const-initialized `Cell` has no destructor to register, so this access
    // cannot allocate.
    let _ = ALLOCATIONS.try_with(|count| count.set(count.get() + 1));
}

unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        count();
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        count();
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

fn allocations(run: impl FnOnce() -> f64) -> usize {
    ALLOCATIONS.with(|count| count.set(0));
    black_box(run());
    ALLOCATIONS.with(Cell::get)
}

/// A one-dimensional `-TAB` transform allocates four vectors each way: forward,
/// the intermediate coordinates, the interpolation location's base and fraction,
/// and the world result; inverse, the intermediate coordinates, the location's two
/// vectors, and the pixel result.
#[test]
fn one_dimensional_tabular_transforms_allocate_four_vectors() {
    let pixel = 12_345.25;
    // The first call builds the cached fixture.
    let world = tabular_forward_at_pixel(pixel);
    assert_eq!(
        allocations(|| tabular_forward_at_pixel(black_box(pixel))),
        4
    );
    assert_eq!(
        allocations(|| tabular_inverse_at_world(black_box(world))),
        4
    );
}

/// The two-dimensional inverse bisects sub-voxels in a scratch it sizes once, so a
/// fraction that takes 20 halvings to reach allocates as much as one that takes one.
#[test]
fn tabular_inverse_allocations_do_not_grow_with_search_depth() {
    let deep = 0.5 + 2.0_f64.powi(-20);
    black_box(tabular_inverse_at_fraction(0.5));
    assert_eq!(
        allocations(|| tabular_inverse_at_fraction(black_box(0.5))),
        allocations(|| tabular_inverse_at_fraction(black_box(deep)))
    );
}
