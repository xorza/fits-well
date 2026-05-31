use super::*;
use crate::reader::FitsReader;
use std::fs::File;

/// Load the WCS from the primary header of a fixture.
fn open_wcs(name: &str) -> Wcs {
    let r = FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap();
    Wcs::from_header(&r.hdu(0).header, None).unwrap()
}

/// Golden pixel→world values from `astropy.wcs` (wcslib) for `wcs_tan.fits`
/// (`RA---TAN`/`DEC--TAN`, CRVAL 150/2.5, CRPIX 256/256, 1″ pixels, 15° rotation).
/// Columns: pixel x, pixel y, RA (deg), Dec (deg).
const TAN_GOLDEN: &[(f64, f64, f64, f64)] = &[
    (1.0, 1.0, 150.050131124369, 2.413246375001),
    (256.0, 256.0, 150.000000000000, 2.500000000000),
    (512.0, 512.0, 149.949665615474, 2.587091911566),
    (100.0, 400.0, 150.052260368590, 2.527420491210),
    (256.5, 256.5, 149.999901697142, 2.500170103464),
    (400.0, 123.0, 149.951756061540, 2.474666292235),
];

#[test]
fn parses_tan_header() {
    let w = open_wcs("wcs_tan.fits");
    assert_eq!(w.naxis, 2);
    assert_eq!(w.ctype, vec!["RA---TAN", "DEC--TAN"]);
    assert_eq!(w.crval, vec![150.0, 2.5]);
    assert_eq!(w.crpix, vec![256.0, 256.0]);
    assert_eq!(w.lonpole, 180.0);
    assert!(w.celestial.is_some());
}

#[test]
fn pixel_to_world_matches_astropy() {
    let w = open_wcs("wcs_tan.fits");
    for &(px, py, ra, dec) in TAN_GOLDEN {
        let out = w.pixel_to_world(&[px, py]);
        assert!(
            (out[0] - ra).abs() < 1e-9,
            "RA at ({px},{py}): got {}, want {ra}",
            out[0]
        );
        assert!(
            (out[1] - dec).abs() < 1e-9,
            "Dec at ({px},{py}): got {}, want {dec}",
            out[1]
        );
    }
}

#[test]
fn world_to_pixel_inverts_pixel_to_world() {
    // Round-trip our own full-precision forward output. The transform is accurate
    // to ~1e-9° throughout; near the reference point the 1″/px scale amplifies that
    // to ~1e-6 px, so test at 1e-5 px (≈ 10 nano-arcsec) — far tighter than any
    // real use needs.
    let w = open_wcs("wcs_tan.fits");
    for &(px, py, _, _) in TAN_GOLDEN {
        let world = w.pixel_to_world(&[px, py]);
        let back = w.world_to_pixel(&world);
        assert!(
            (back[0] - px).abs() < 1e-5 && (back[1] - py).abs() < 1e-5,
            "pixel→world→pixel at ({px},{py}): got {back:?}"
        );
    }
}

#[test]
fn reference_pixel_maps_to_crval() {
    let w = open_wcs("wcs_tan.fits");
    let out = w.pixel_to_world(&[256.0, 256.0]);
    assert!((out[0] - 150.0).abs() < 1e-12);
    assert!((out[1] - 2.5).abs() < 1e-12);
}

/// A matrix inversion sanity check independent of any fixture.
#[test]
fn matrix_inverse_is_correct() {
    let m = vec![2.0, 1.0, 1.0, 3.0]; // [[2,1],[1,3]], det = 5
    let inv = invert(&m, 2).unwrap();
    // inverse = 1/5 [[3,-1],[-1,2]]
    let expect = [0.6, -0.2, -0.2, 0.4];
    for (a, b) in inv.iter().zip(&expect) {
        assert!((a - b).abs() < 1e-12, "{a} vs {b}");
    }
    // m · inv = I
    let prod = matvec(&m, &matvec(&inv, &[1.0, 0.0], 2), 2);
    assert!((prod[0] - 1.0).abs() < 1e-12 && prod[1].abs() < 1e-12);
}

#[test]
fn sin_projection_matches_astropy() {
    // RA---SIN/DEC--SIN, CRPIX 100/100, CRVAL 45/30, 3.6″ pixels, no rotation.
    // Golden values from astropy.wcs — validates the SIN formula, not just that
    // our forward and inverse agree.
    let mut h = Header::new();
    h.set("NAXIS", 2);
    h.set("CTYPE1", "RA---SIN").set("CTYPE2", "DEC--SIN");
    h.set("CRPIX1", 100.0).set("CRPIX2", 100.0);
    h.set("CRVAL1", 45.0).set("CRVAL2", 30.0);
    h.set("CDELT1", -1e-3).set("CDELT2", 1e-3);
    let w = Wcs::from_header(&h, None).unwrap();
    let golden: &[(f64, f64, f64, f64)] = &[
        (100.0, 100.0, 45.000000000000, 30.000000000000),
        (50.0, 150.0, 45.057764154844, 30.049987404157),
        (1.0, 1.0, 45.114201616520, 29.900950619091),
        (180.0, 20.0, 44.907698264374, 29.919967754584),
    ];
    for &(px, py, ra, dec) in golden {
        let out = w.pixel_to_world(&[px, py]);
        assert!(
            (out[0] - ra).abs() < 1e-9 && (out[1] - dec).abs() < 1e-9,
            "SIN at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
}

/// `SIN` and `ARC` deprojections invert their forward projections.
#[test]
fn sin_and_arc_round_trip_through_projection() {
    for proj in [Projection::Sin, Projection::Arc, Projection::Tan] {
        for &(phi, theta) in &[(30.0_f64, 80.0_f64), (-120.0, 85.0), (170.0, 60.0)] {
            let (x, y) = project(proj, phi, theta);
            let (p2, t2) = deproject(proj, x, y);
            assert!(
                (norm180(p2 - phi)).abs() < 1e-9 && (t2 - theta).abs() < 1e-9,
                "{proj:?}: ({phi},{theta}) → ({x},{y}) → ({p2},{t2})"
            );
        }
    }
}
