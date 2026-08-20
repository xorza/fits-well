use crate::error::FitsError;
use crate::header::Header;
use crate::wcs::Wcs;
use crate::wcs::internals::CROTA_GOLDEN;
use crate::wcs::linear_transform::internals as linear;
use crate::wcs::linear_transform::*;
use crate::wcs::wcs_axis::WcsAxis;

/// A transform built straight from a matrix, skipping the header read. The
/// inverse is still computed here, so the fixture obeys the same invariant a
/// parsed transform does.
fn from_matrix(matrix: Vec<f64>, naxis: usize) -> LinearTransform {
    let inverse = invert(&matrix, naxis).expect("non-singular fixture");
    LinearTransform {
        matrix,
        inverse,
        naxis,
    }
}

fn axis_at(crpix: f64) -> WcsAxis {
    WcsAxis {
        ctype: String::new(),
        cunit: String::new(),
        crval: 0.0,
        crpix,
        spectral_frame: None,
    }
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
    let prod = [m[0] * inv[0] + m[1] * inv[2], m[2] * inv[0] + m[3] * inv[2]];
    assert!((prod[0] - 1.0).abs() < 1e-12 && prod[1].abs() < 1e-12);
}

#[test]
fn matrix_product_applies_reference_pixel_offsets_while_accumulating() {
    let axes = [axis_at(10.0), axis_at(-2.0), axis_at(4.0)];
    let transform = from_matrix(vec![2.0, -1.0, 0.5, 0.0, 3.0, 4.0, -2.0, 1.0, 1.5], 3);
    let pixel = [13.0, 3.0, 2.0];
    assert_eq!(transform.intermediate(&pixel, &axes), [0.0, 7.0, -4.0]);
    // Every row of the batch form agrees with the single-axis form.
    for axis in 0..3 {
        assert_eq!(
            transform.intermediate_axis(axis, &pixel, &axes),
            transform.intermediate(&pixel, &axes)[axis]
        );
    }
    // The inverse direction returns the pixel the intermediate came from.
    let round_trip = transform.pixel(&[0.0, 7.0, -4.0], &axes);
    for (got, want) in round_trip.iter().zip(&pixel) {
        assert!((got - want).abs() < 1e-12, "{got} vs {want}");
    }
}

#[test]
fn legacy_crota_rotation_matches_astropy() {
    // CDELT + CROTA2 (no PC/CD) — the legacy rotation convention.
    let mut h = Header::new();
    h.set_internal("NAXIS", 2);
    h.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    h.set_internal("CRPIX1", 128.0)
        .set_internal("CRPIX2", 128.0);
    h.set_internal("CRVAL1", 83.6).set_internal("CRVAL2", 22.0);
    h.set_internal("CDELT1", -0.0005)
        .set_internal("CDELT2", 0.0005);
    h.set_internal("CROTA2", 25.0);
    let w = Wcs::from_header(&h, None).unwrap();
    for &(px, py, ra, dec) in CROTA_GOLDEN {
        let out = w.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (out[0] - ra).abs() < 1e-8 && (out[1] - dec).abs() < 1e-8,
            "CROTA at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
}

#[test]
fn conflicting_linear_keywords_are_rejected() {
    let base = || {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2)
            .set_internal("CTYPE1", "RA---TAN")
            .set_internal("CTYPE2", "DEC--TAN")
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRPIX2", 1.0)
            .set_internal("CRVAL1", 0.0)
            .set_internal("CRVAL2", 0.0)
            .set_internal("CDELT1", 1.0)
            .set_internal("CDELT2", 1.0);
        h
    };
    // PC and CD are mutually exclusive; CROTA must not accompany PC.
    let mut pc_cd = base();
    pc_cd.set_internal("PC1_1", 1.0).set_internal("CD1_1", 1.0);
    assert!(matches!(
        Wcs::from_header(&pc_cd, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
    let mut crota_pc = base();
    crota_pc
        .set_internal("PC1_1", 1.0)
        .set_internal("CROTA2", 30.0);
    assert!(matches!(
        Wcs::from_header(&crota_pc, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
    let mut crota_cd = base();
    crota_cd
        .set_internal("CDELT1", 11.0)
        .set_internal("CDELT2", 13.0)
        .set_internal("CD1_1", 2.0)
        .set_internal("CD2_2", 3.0)
        .set_internal("CROTA2", 30.0);
    let w = Wcs::from_header(&crota_cd, None).unwrap();
    assert_eq!(linear::matrix(&w.linear), [2.0, 0.0, 0.0, 3.0]);
    // A single convention (CD alone) is accepted.
    let mut cd_only = base();
    cd_only
        .set_internal("CD1_1", 1.0)
        .set_internal("CD2_2", 1.0);
    assert!(Wcs::from_header(&cd_only, None).is_ok());

    let mut malformed = base();
    malformed.set_internal("CRVAL1", "not numeric");
    assert!(matches!(
        Wcs::from_header(&malformed, None),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "CRVAL1" && expected == "real"
    ));
}

#[test]
fn cunit_scales_celestial_axes_to_degrees() {
    // §8.2: CRVAL/CDELT are in CUNITia units. The same physical TAN WCS expressed
    // in degrees and in arcseconds must yield identical world coordinates.
    let build = |scale: f64, unit: Option<&str>| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", "RA---TAN")
            .set_internal("CTYPE2", "DEC--TAN");
        h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
        h.set_internal("CRVAL1", 150.0 * scale)
            .set_internal("CRVAL2", 30.0 * scale);
        h.set_internal("CDELT1", -5e-4 * scale)
            .set_internal("CDELT2", 5e-4 * scale);
        if let Some(u) = unit {
            h.set_internal("CUNIT1", u).set_internal("CUNIT2", u);
        }
        Wcs::from_header(&h, None).unwrap()
    };
    let w_deg = build(1.0, None);
    let w_asec = build(3600.0, Some("arcsec"));
    for &(px, py) in &[(1.0, 1.0), (50.0, 50.0), (80.0, 20.0), (33.0, 77.0)] {
        let a = w_deg.pixel_to_world(&[px, py]).unwrap();
        let b = w_asec.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "deg {a:?} vs arcsec {b:?} at ({px},{py})"
        );
    }
    // The reference pixel maps exactly to CRVAL = (150°, 30°) — proving the arcsec
    // CRVAL was scaled to degrees, not taken literally.
    let r = w_asec.pixel_to_world(&[50.0, 50.0]).unwrap();
    assert!(
        (r[0] - 150.0).abs() < 1e-9 && (r[1] - 30.0).abs() < 1e-9,
        "{r:?}"
    );
}
