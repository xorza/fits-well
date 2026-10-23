//! Allocation counts of the WCS transforms, under an allocator that counts the
//! calling thread's allocations only, so the harness's other threads do not reach
//! the count.

#![expect(
    clippy::disallowed_types,
    reason = "a global allocator has no owner to carry its counters"
)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::hint::black_box;

use fits_well::header::Header;
use fits_well::internals::{
    tabular_forward_at_pixel, tabular_inverse_at_fraction, tabular_inverse_at_world,
};
use fits_well::wcs::Wcs;

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

/// A celestial pair beside a frequency axis sampled in wavelength writes into the
/// caller's buffers and allocates nothing either way.
#[test]
fn celestial_and_spectral_transforms_allocate_nothing() {
    let mut header = Header::new();
    header
        .set("NAXIS", 3)
        .unwrap()
        .set("CTYPE1", "RA---TAN")
        .unwrap()
        .set("CTYPE2", "DEC--TAN")
        .unwrap()
        .set("CRPIX1", 100.0)
        .unwrap()
        .set("CRPIX2", 100.0)
        .unwrap()
        .set("CRVAL1", 83.6)
        .unwrap()
        .set("CRVAL2", 22.0)
        .unwrap()
        .set("CDELT1", -0.0005)
        .unwrap()
        .set("CDELT2", 0.0005)
        .unwrap()
        .set("CTYPE3", "FREQ-W2F")
        .unwrap()
        .set("CUNIT3", "Hz")
        .unwrap()
        .set("CRPIX3", 1.0)
        .unwrap()
        .set("CRVAL3", 1_420_405_751.0)
        .unwrap()
        .set("CDELT3", 1e6)
        .unwrap()
        .set("RESTFRQ", 1_420_405_751.0)
        .unwrap()
        .set("PC1_3", 0.1)
        .unwrap();
    let wcs = Wcs::from_header(&header, None).unwrap();
    let mut world = [0.0; 3];
    let mut pixel = [0.0; 3];
    assert_eq!(
        allocations(|| {
            wcs.pixel_to_world(black_box(&[120.0, 80.0, 5.0]), &mut world)
                .unwrap();
            world[0]
        }),
        0
    );
    assert_eq!(
        allocations(|| {
            wcs.world_to_pixel(black_box(&world), &mut pixel).unwrap();
            pixel[0]
        }),
        0
    );
}

/// A one-dimensional `-TAB` transform allocates two vectors each way, the
/// interpolation location's base and fraction.
#[test]
fn one_dimensional_tabular_transforms_allocate_two_vectors() {
    let pixel = 12_345.25;
    // The first call builds the cached fixture.
    let world = tabular_forward_at_pixel(pixel);
    assert_eq!(
        allocations(|| tabular_forward_at_pixel(black_box(pixel))),
        2
    );
    assert_eq!(
        allocations(|| tabular_inverse_at_world(black_box(world))),
        2
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
