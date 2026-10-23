use crate::alternate::Alternate;
use crate::header_model::keywords;
use crate::world_coordinates::internals::to_world;

use crate::world_coordinates::Wcs;
use crate::world_coordinates::internals::CROTA_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;
use crate::world_coordinates::internals::celestial_header;
use crate::world_coordinates::linear_transform::internals as linear;
use crate::world_coordinates::linear_transform::*;

/// A transform built straight from a matrix, skipping the header read. The
/// factors are still computed here, so the fixture obeys the same invariant a
/// parsed transform does.
fn from_matrix(matrix: Vec<f64>, naxis: usize) -> LinearTransform {
    let factors = LuFactors::new(&matrix, naxis).expect("non-singular fixture");
    LinearTransform {
        matrix,
        factors,
        naxis,
    }
}

/// `x` with `A·x = b` for the `n×n` matrix `a`.
fn solved(a: &[f64], n: usize, b: &[f64]) -> Vec<f64> {
    let mut x = b.to_vec();
    LuFactors::new(a, n).unwrap().solve(&mut x);
    x
}

fn axis_at(crpix: f64) -> Axis {
    Axis {
        ctype: String::new(),
        cunit: String::new(),
        crval: 0.0,
        crpix,
        spectral_frame: None,
    }
}

/// The solve gives the columns of the inverse, independent of any fixture.
#[test]
fn lu_solve_inverts_the_matrix() {
    // [[2,1],[1,3]], det = 5, inverse = 1/5 [[3,-1],[-1,2]]. The pivot is 2, so
    // L = [[1,0],[0.5,1]] and U = [[2,1],[0,2.5]]: e₁ gives y = (1, −0.5) and
    // x = ((1 + 0.2)/2, −0.5/2.5), and e₂ gives y = (0, 1) and x = ((0 − 0.4)/2, 1/2.5).
    let m = [2.0, 1.0, 1.0, 3.0];
    assert_eq!(solved(&m, 2, &[1.0, 0.0]), [0.6, -0.2]);
    assert_eq!(solved(&m, 2, &[0.0, 1.0]), [-0.2, 0.4]);
    // A zero leading entry needs the row swap: [[0,1],[1,0]] swaps the coordinates.
    assert_eq!(solved(&[0.0, 1.0, 1.0, 0.0], 2, &[3.0, 5.0]), [5.0, 3.0]);
}

#[test]
fn matrix_product_applies_reference_pixel_offsets_while_accumulating() {
    let axes = [axis_at(10.0), axis_at(-2.0), axis_at(4.0)];
    let transform = from_matrix(vec![2.0, -1.0, 0.5, 0.0, 3.0, 4.0, -2.0, 1.0, 1.5], 3);
    let pixel = [13.0, 3.0, 2.0];
    let mut intermediate = [f64::NAN; 3];
    transform.intermediate(&pixel, &axes, &mut intermediate);
    assert_eq!(intermediate, [0.0, 7.0, -4.0]);
    // Every row of the batch form agrees with the single-axis form.
    for (axis, &value) in intermediate.iter().enumerate() {
        assert_eq!(transform.intermediate_axis(axis, &pixel, &axes), value);
    }
    // The inverse direction returns the pixel the intermediate came from, in place.
    transform.pixel(&mut intermediate, &axes);
    for (got, want) in intermediate.iter().zip(&pixel) {
        assert!((got - want).abs() < 1e-12, "{got} vs {want}");
    }
}

#[test]
fn legacy_crota_rotation_matches_astropy() {
    // CDELT + CROTA2 (no PC/CD) — the legacy rotation convention.
    let mut h = celestial_header("TAN", [128.0, 128.0], [83.6, 22.0], [-0.0005, 0.0005]);
    h.set_internal("CROTA2", 25.0);
    let w = Wcs::from_header(&h, None).unwrap();
    assert_astropy_golden(&w, &CROTA_GOLDEN, "CROTA");
}

#[test]
fn conflicting_linear_keywords_are_rejected() {
    let base = || {
        let mut h = Header::new();
        h.set_internal(keywords::NAXIS, 2)
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
        let mut h = celestial_header(
            "TAN",
            [50.0, 50.0],
            [150.0 * scale, 30.0 * scale],
            [-5e-4 * scale, 5e-4 * scale],
        );
        if let Some(u) = unit {
            h.set_internal("CUNIT1", u).set_internal("CUNIT2", u);
        }
        Wcs::from_header(&h, None).unwrap()
    };
    let w_deg = build(1.0, None);
    let w_asec = build(3600.0, Some("arcsec"));
    for &(px, py) in &[(1.0, 1.0), (50.0, 50.0), (80.0, 20.0), (33.0, 77.0)] {
        let a = to_world(&w_deg, &[px, py]).unwrap();
        let b = to_world(&w_asec, &[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "deg {a:?} vs arcsec {b:?} at ({px},{py})"
        );
    }
    // The reference pixel maps exactly to CRVAL = (150°, 30°) — proving the arcsec
    // CRVAL was scaled to degrees, not taken literally.
    let r = to_world(&w_asec, &[50.0, 50.0]).unwrap();
    assert!(
        (r[0] - 150.0).abs() < 1e-9 && (r[1] - 30.0).abs() < 1e-9,
        "{r:?}"
    );
}

/// The singular test is relative: a matrix in tiny units factors, and a rank-one
/// matrix does not — also when rounding leaves its second pivot at −1.1e-16 rather
/// than 0, which an absolute threshold took for a value and solved to 1e16s.
#[test]
fn matrix_factoring_detects_singularity_relative_to_the_entries() {
    assert_eq!(
        solved(&[1e-20, 0.0, 0.0, 4e-20], 2, &[1.0, 1.0]),
        [1.0 / 1e-20, 1.0 / 4e-20]
    );
    assert!(LuFactors::new(&[1.0, 2.0, 2.0, 4.0], 2).is_none());
    assert!(LuFactors::new(&[1e-300, 2e-300, 2e-300, 4e-300], 2).is_none());
    assert!(LuFactors::new(&[0.1, 0.7, 0.3, 2.1], 2).is_none());
}

/// Matrix keywords are matched by their exact names: two-digit indices, the selected alternate
/// suffix only, no index past the axis count, and no zero-padded spelling, which names a
/// different keyword. Unset `PC` cells keep the identity, each row scaled by its `CDELT`.
#[test]
fn matrix_keywords_are_matched_by_exact_name() {
    let naxis = 12;
    let cdelt: Vec<f64> = (1..=naxis).map(|i| i as f64).collect();
    let mut header = Header::new();
    header
        .set_internal("PC10_12", 0.5)
        .set_internal("PC2_1", 0.25)
        .set_internal("PC1_1A", 7.0)
        .set_internal("PC13_1", 9.0)
        .set_internal("PC01_02", 3.0);
    let primary =
        LinearMatrix::from_header(&header, AltSuffix::new(None), naxis, &cdelt, None).unwrap();
    let mut expected = vec![0.0; naxis * naxis];
    for i in 0..naxis {
        expected[i * naxis + i] = cdelt[i];
    }
    expected[9 * naxis + 11] = 10.0 * 0.5;
    expected[naxis] = 2.0 * 0.25;
    assert_eq!(primary.values, expected);

    let alternate = LinearMatrix::from_header(
        &header,
        AltSuffix::new(Some(Alternate::new('A'))),
        naxis,
        &cdelt,
        None,
    )
    .unwrap();
    let mut expected = vec![0.0; naxis * naxis];
    for i in 0..naxis {
        expected[i * naxis + i] = cdelt[i];
    }
    expected[0] = 7.0;
    assert_eq!(alternate.values, expected);

    let mut cd = Header::new();
    cd.set_internal("CD3_2", 1.5).set_internal("CD12_12", -2.0);
    let matrix = LinearMatrix::from_header(&cd, AltSuffix::new(None), naxis, &cdelt, None).unwrap();
    let mut expected = vec![0.0; naxis * naxis];
    expected[2 * naxis + 1] = 1.5;
    expected[naxis * naxis - 1] = -2.0;
    assert_eq!(matrix.values, expected);
}
