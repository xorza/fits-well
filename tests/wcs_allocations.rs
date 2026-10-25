//! Allocation counts of the WCS transforms, under the counting allocator's count of the
//! calling thread's allocations, which the harness's other threads do not reach.

#![expect(
    clippy::disallowed_types,
    reason = "a global allocator has no owner to carry its counters"
)]

use std::hint::black_box;

use fits_well::header::{Header, Value};
use fits_well::internals::{
    tabular_forward_at_fraction, tabular_forward_at_pixel, tabular_inverse_at_fraction,
    tabular_inverse_at_world,
};
use fits_well::wcs::Wcs;

#[path = "support/counting.rs"]
#[expect(dead_code, reason = "these tests count calls, not the bytes held")]
mod counting;

use crate::counting::ALLOCATOR;

fn allocations(run: impl FnOnce() -> f64) -> usize {
    ALLOCATOR.calls(run)
}

/// A celestial pair beside a frequency axis sampled in wavelength writes into the
/// caller's buffers and allocates nothing either way.
#[test]
fn celestial_and_spectral_transforms_allocate_nothing() {
    let mut header = Header::new();
    let cards: [(&str, Value); 16] = [
        ("NAXIS", 3.into()),
        ("CTYPE1", "RA---TAN".into()),
        ("CTYPE2", "DEC--TAN".into()),
        ("CRPIX1", 100.0.into()),
        ("CRPIX2", 100.0.into()),
        ("CRVAL1", 83.6.into()),
        ("CRVAL2", 22.0.into()),
        ("CDELT1", (-0.0005).into()),
        ("CDELT2", 0.0005.into()),
        ("CTYPE3", "FREQ-W2F".into()),
        ("CUNIT3", "Hz".into()),
        ("CRPIX3", 1.0.into()),
        ("CRVAL3", 1_420_405_751.0.into()),
        ("CDELT3", 1e6.into()),
        ("RESTFRQ", 1_420_405_751.0.into()),
        ("PC1_3", 0.1.into()),
    ];
    for (keyword, value) in cards {
        header.set(keyword, value).unwrap();
    }
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

/// A one-dimensional `-TAB` transform allocates nothing either way: its interval is
/// a position and a fraction, and its runs are built with the transform.
#[test]
fn one_dimensional_tabular_transforms_allocate_nothing() {
    let pixel = 12_345.25;
    // The first call builds the cached fixture.
    let world = tabular_forward_at_pixel(pixel);
    assert_eq!(
        allocations(|| tabular_forward_at_pixel(black_box(pixel))),
        0
    );
    assert_eq!(
        allocations(|| tabular_inverse_at_world(black_box(world))),
        0
    );
}

/// The two-dimensional inverse bisects sub-voxels in fixed arrays on the stack, so a
/// fraction that takes 20 halvings to reach allocates nothing, as one that takes one does,
/// and the forward point through the same coupled table allocates nothing either.
#[test]
fn tabular_inverse_allocations_do_not_grow_with_search_depth() {
    let deep = 0.5 + 2.0_f64.powi(-20);
    black_box(tabular_inverse_at_fraction(0.5));
    assert_eq!(
        allocations(|| tabular_inverse_at_fraction(black_box(0.5))),
        0
    );
    assert_eq!(
        allocations(|| tabular_inverse_at_fraction(black_box(deep))),
        0
    );
    assert_eq!(
        allocations(|| tabular_forward_at_fraction(black_box(0.5))),
        0
    );
}
