use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::header_model::Header;
use crate::header_model::value::Value;
use crate::reader::FitsReader;
use crate::world_coordinates::R2D;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::internals::TAN_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;
use crate::world_coordinates::projection::Projection;
use crate::world_coordinates::wcs_axis::WcsAxis;
use std::fs::File;

/// Load the WCS from the primary header of a fixture.
fn open_wcs(name: &str) -> Wcs {
    let r = FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap();
    Wcs::from_header(&r.hdus[0].header, None).unwrap()
}

#[test]
fn parses_tan_header() {
    let w = open_wcs("wcs_tan.fits");
    assert_eq!(
        w.view().axes,
        [
            WcsAxis {
                ctype: "RA---TAN".to_string(),
                cunit: "deg".to_string(),
                crval: 150.0,
                crpix: 256.0,
                spectral_frame: None,
            },
            WcsAxis {
                ctype: "DEC--TAN".to_string(),
                cunit: "deg".to_string(),
                crval: 2.5,
                crpix: 256.0,
                spectral_frame: None,
            },
        ]
    );
    // Zenithal pole reduces to (CRVAL, LONPOLE=180).
    let c = w.celestial.expect("celestial");
    assert_eq!(
        c.pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0
        }
    );
}

#[test]
fn pixel_to_world_matches_astropy() {
    let w = open_wcs("wcs_tan.fits");
    assert_astropy_golden(&w, &TAN_GOLDEN, "TAN image");
}

#[test]
fn world_to_pixel_inverts_pixel_to_world() {
    // Round-trip our own full-precision forward output. The transform is accurate
    // to ~1e-9° throughout; near the reference point the 1″/px scale amplifies that
    // to ~1e-6 px, so test at 1e-5 px (≈ 10 nano-arcsec) — far tighter than any
    // real use needs.
    let w = open_wcs("wcs_tan.fits");
    for &(px, py, _, _) in TAN_GOLDEN.points {
        let world = w.pixel_to_world(&[px, py]).unwrap();
        let back = w.world_to_pixel(&world).unwrap();
        assert!(
            (back[0] - px).abs() < 1e-5 && (back[1] - py).abs() < 1e-5,
            "pixel→world→pixel at ({px},{py}): got {back:?}"
        );
    }
}

#[test]
fn reference_pixel_maps_to_crval() {
    let w = open_wcs("wcs_tan.fits");
    let out = w.pixel_to_world(&[256.0, 256.0]).unwrap();
    assert!((out[0] - 150.0).abs() < 1e-12);
    assert!((out[1] - 2.5).abs() < 1e-12);
}

#[test]
fn transform_failures_return_errors() {
    let build = |projection: &str| {
        let mut header = Header::new();
        header.set_internal("NAXIS", 2);
        header
            .set_internal("CTYPE1", format!("RA---{projection}"))
            .set_internal("CTYPE2", format!("DEC--{projection}"));
        header
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRPIX2", 1.0);
        header
            .set_internal("CRVAL1", 0.0)
            .set_internal("CRVAL2", 0.0);
        header
            .set_internal("CDELT1", 100.0)
            .set_internal("CDELT2", 100.0);
        Wcs::from_header(&header, None)
    };

    let sin = build("SIN").unwrap();
    assert!(matches!(
        sin.pixel_to_world(&[2.0, 1.0]),
        Err(FitsError::WcsProjectionDomain { projection: "SIN" })
    ));

    // A ZPN polynomial without coefficients maps every colatitude to the pole.
    assert!(matches!(build("ZPN"), Err(FitsError::InvalidWcs { .. })));

    let tan = open_wcs("wcs_tan.fits");
    assert!(matches!(
        tan.world_to_pixel(&[150.0, 100.0]),
        Err(FitsError::WcsWorldOutOfDomain { projection: "TAN" })
    ));
    for result in [
        tan.pixel_to_world(&[1.0]),
        tan.pixel_to_world(&[1.0, 2.0, 3.0]),
        tan.world_to_pixel(&[150.0]),
        tan.world_to_pixel(&[150.0, 2.5, 1.0]),
    ] {
        assert!(matches!(
            result,
            Err(FitsError::RankMismatch {
                ranked: Ranked::WcsCoordinate,
                expected: 2,
                ..
            })
        ));
    }
    assert!(matches!(
        tan.axis_world(2, &[1.0, 1.0]),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::WcsAxis,
            index: 3,
            len: 2
        })
    ));
}

#[test]
fn public_wcs_metadata_exposes_units_and_celestial_projection_pair() {
    let mut header = Header::new();
    header
        .set_internal("NAXIS", 2)
        .set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN")
        .set_internal("CUNIT1", "deg")
        .set_internal("CUNIT2", "deg")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRPIX2", 1.0)
        .set_internal("CRVAL1", 45.0)
        .set_internal("CRVAL2", 30.0)
        .set_internal("CDELT1", -0.1)
        .set_internal("CDELT2", 0.1);
    let wcs = Wcs::from_header(&header, None).unwrap();
    let view = wcs.view();
    assert_eq!(view.axes[0].cunit, "deg");
    assert_eq!(view.axes[1].cunit, "deg");
    let celestial = view.celestial_projection.unwrap();
    assert_eq!(celestial.longitude_axis, 0);
    assert_eq!(celestial.latitude_axis, 1);
    assert_eq!(celestial.projection, Projection::Tan);
    assert_eq!(
        celestial.pole,
        CelestialPole {
            ra: 45.0,
            dec: 30.0,
            lonpole: 180.0
        }
    );
}

#[test]
fn planetary_solar_lonlat_axes_are_celestial() {
    // §8.2: `yzLN`/`yzLT` (here helioprojective `HPLN`/`HPLT`) are celestial axis
    // types; with the same projection + CRVAL they transform exactly like RA/DEC
    // (the frame label is preserved, never converted — that is out of scope).
    let build = |t1: &str, t2: &str| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", t1).set_internal("CTYPE2", t2);
        h.set_internal("CRPIX1", 64.0).set_internal("CRPIX2", 64.0);
        h.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", -20.0);
        h.set_internal("CDELT1", -1e-3).set_internal("CDELT2", 1e-3);
        Wcs::from_header(&h, None).unwrap()
    };
    let radec = build("RA---TAN", "DEC--TAN");
    let helio = build("HPLN-TAN", "HPLT-TAN");
    assert!(
        helio.celestial.is_some(),
        "HPLN/HPLT must be recognized as a celestial pair"
    );
    for &(px, py) in &[(1.0, 1.0), (64.0, 64.0), (100.0, 30.0)] {
        let a = radec.pixel_to_world(&[px, py]).unwrap();
        let b = helio.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "RA/DEC {a:?} vs HPLN/HPLT {b:?}"
        );
    }
}

#[test]
fn absent_wcsaxes_uses_the_largest_wcs_index() {
    let build = |keyword: &str, value: Value| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2).set_internal(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set_internal("NAXIS", 2)
        .set_internal("CD1_1", 1.0)
        .set_internal("CD2_2", 1.0)
        .set_internal("CD3_3", 1.0)
        .set_internal("CD4_4", 1.0);
    let cases = [
        build("CTYPE4", Value::Text("LINEAR".to_string())),
        build("CUNIT4", Value::Text("m".to_string())),
        build("PV4_0", Value::Real(1.0)),
        build("PC4_4", Value::Real(1.0)),
        cd,
    ];
    for h in &cases {
        assert_eq!(Wcs::from_header(h, None).unwrap().view().axes.len(), 4);
    }

    for alternate in [
        build("CTYPE4A", Value::Text("LINEAR".to_string())),
        build("PV4_0A", Value::Real(1.0)),
        build("PC4_4A", Value::Real(1.0)),
    ] {
        assert_eq!(
            Wcs::from_header(&alternate, None)
                .unwrap()
                .view()
                .axes
                .len(),
            2
        );
        assert_eq!(
            Wcs::from_header(&alternate, Some('A'))
                .unwrap()
                .view()
                .axes
                .len(),
            4
        );
    }
}

#[test]
fn rejects_absurd_wcsaxes() {
    // Axis counts are untrusted; reject both bounds before they size a matrix or
    // drive the per-axis loops. 1 and 999 are the bounds §8.2 allows.
    let mut h = Header::new();
    for value in [-1, 0, 1000] {
        h.set_internal("WCSAXES", value);
        assert!(matches!(
            Wcs::from_header(&h, None),
            Err(FitsError::KeywordOutOfRange { name: "WCSAXES" })
        ));
    }
    h.set_internal("WCSAXES", 1);
    assert_eq!(Wcs::from_header(&h, None).unwrap().view().axes.len(), 1);
    h.set_internal("WCSAXES", 999);
    assert_eq!(Wcs::image_axis_count(&h, None).unwrap(), 999);

    h.set_internal("WCAX5", -1);
    assert!(matches!(
        Wcs::from_array_column(&h, 5, None),
        Err(FitsError::KeywordOutOfRange { name: "WCAXn" })
    ));
}

fn celestial_header(projection: &str, keywords: &[(&str, f64)]) -> Header {
    let mut header = Header::new();
    header
        .set_internal("NAXIS", 2)
        .set_internal("CTYPE1", format!("RA---{projection}"))
        .set_internal("CTYPE2", format!("DEC--{projection}"));
    for &(keyword, value) in keywords {
        header.set_internal(keyword, value);
    }
    header
}

/// CAR with δ₀ = −30° has two poles, δp = u ± v with u = 180°, v = acos(−½) = 120°:
/// 300° ≡ −60° and 60°. LATPOLE picks between them — the candidate `5π/3` is valid once
/// wrapped, as wcslib's `celset` wraps it.
#[test]
fn latpole_chooses_between_both_valid_poles() {
    for (latpole, pole) in [(90.0, 60.0), (-90.0, -60.0)] {
        let header = celestial_header("CAR", &[("CRVAL2", -30.0), ("LATPOLE", latpole)]);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let dec = wcs.view().celestial_projection.unwrap().pole.dec;
        assert!((dec - pole).abs() < 1e-12, "LATPOLE {latpole}: {dec}");
    }
}

/// With θ₀ = 0 and φp − φ₀ = 120°, a fiducial latitude needs |δ₀| ≤ asin|cos 120°| = 30°;
/// δ₀ = 60° admits no pole at all.
#[test]
fn an_impossible_pole_is_refused() {
    let header = celestial_header("CAR", &[("CRVAL2", 60.0), ("LONPOLE", 120.0)]);
    assert!(matches!(
        Wcs::from_header(&header, None),
        Err(FitsError::WcsInvalidPole { .. })
    ));
}

/// A celestial axis needs a partner of its own system; a lone one, or a pair of two
/// systems, is not evaluated.
#[test]
fn unmatched_celestial_axes_are_unsupported() {
    for (ctypes, unsupported) in [
        (["RA---TAN", "FREQ"], vec![0]),
        (["FREQ", "DEC--TAN"], vec![1]),
        (["RA---TAN", "GLAT-TAN"], vec![0, 1]),
        (["GLON-TAN", "GLAT-TAN"], vec![]),
        (["ELON-CAR", "ELAT-CAR"], vec![]),
        (["HPLN-TAN", "HPLT-TAN"], vec![]),
        (["HPLN-TAN", "SOLT-TAN"], vec![0, 1]),
    ] {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 2)
            .set_internal("CTYPE1", ctypes[0])
            .set_internal("CTYPE2", ctypes[1]);
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert_eq!(wcs.view().unsupported_axes, unsupported, "{ctypes:?}");
        assert_eq!(
            wcs.view().celestial_projection.is_some(),
            unsupported.is_empty(),
            "{ctypes:?}"
        );
    }
}

/// One axis of a celestial pair evaluates the projection, as the complete transform does.
#[test]
fn axis_world_of_a_celestial_axis_is_the_projected_value() {
    let header = celestial_header("TAN", &[("CRVAL1", 150.0), ("CRVAL2", 60.0)]);
    let wcs = Wcs::from_header(&header, None).unwrap();
    let pixel = [30.0, 20.0];
    let complete = wcs.pixel_to_world(&pixel).unwrap();
    for (axis, &value) in complete.iter().enumerate() {
        let world = wcs.axis_world(axis, &pixel).unwrap();
        assert_eq!(world.value, value, "axis {axis}");
        assert_eq!(world.cunit, "deg", "axis {axis}");
    }
}

/// Angle units resolve through one table — SI-prefixed radians, the unprefixed sexagesimal
/// units, numeric multipliers — and anything else is refused, never read as degrees.
/// 1 mrad = 0.0572957795°; CDELT 1 then puts pixel 2 that far from CRVAL.
#[test]
fn celestial_units_resolve_or_are_refused() {
    for (unit, degrees) in [
        ("deg", 1.0),
        ("", 1.0),
        ("arcmin", 1.0 / 60.0),
        ("arcsec", 1.0 / 3600.0),
        ("mas", 1.0 / 3_600_000.0),
        ("rad", R2D),
        ("mrad", 1e-3 * R2D),
        ("urad", 1e-6 * R2D),
        ("10**-3 deg", 1e-3),
    ] {
        let header = celestial_header("CAR", &[("CRPIX1", 1.0), ("CRPIX2", 1.0)]);
        let mut header = header;
        header
            .set_internal("CUNIT1", unit)
            .set_internal("CUNIT2", unit);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let world = wcs.pixel_to_world(&[2.0, 1.0]).unwrap();
        // The value passes through the native-to-celestial rotation, whose trigonometry
        // leaves a few 1e-15 rad — order 1e-14 of a degree.
        assert!(
            (world[0] - degrees).abs() < 1e-13 * degrees.max(1.0),
            "{unit:?}: {world:?}"
        );
    }
    for unit in ["furlong", "mdeg", "karcsec", "Hz"] {
        let mut header = celestial_header("CAR", &[]);
        header
            .set_internal("CUNIT1", unit)
            .set_internal("CUNIT2", "deg");
        assert!(
            matches!(Wcs::from_header(&header, None), Err(FitsError::InvalidUnit { unit: found, .. }) if found == unit),
            "{unit:?}"
        );
    }
}

/// The view reports the axes as the header declares them; the transform's own unit is
/// what `axis_world` and the world coordinates use.
#[test]
fn the_view_keeps_crval_in_its_declared_unit() {
    let mut header = celestial_header("TAN", &[("CRVAL1", 3600.0), ("CRVAL2", 7200.0)]);
    header
        .set_internal("CUNIT1", "arcsec")
        .set_internal("CUNIT2", "arcsec");
    let wcs = Wcs::from_header(&header, None).unwrap();
    let axes = wcs.view().axes;
    assert_eq!((axes[0].crval, axes[0].cunit.as_str()), (3600.0, "arcsec"));
    assert_eq!((axes[1].crval, axes[1].cunit.as_str()), (7200.0, "arcsec"));
    let world = wcs.pixel_to_world(&[0.0, 0.0]).unwrap();
    // The reference pixel deprojects to the native pole and rotates back to CRVAL, in
    // degrees, through a few ULP of trigonometry.
    assert!(
        (world[0] - 1.0).abs() < 1e-12 && (world[1] - 2.0).abs() < 1e-12,
        "{world:?}"
    );
}

/// `[0, 360)`: a tiny negative angle's remainder rounds to 360 itself, which is 0.
#[test]
fn norm360_stays_below_a_full_turn() {
    use crate::world_coordinates::norm360;

    assert_eq!(norm360(-1e-20), 0.0);
    assert_eq!(norm360(-90.0), 270.0);
    assert_eq!(norm360(720.0), 0.0);
    assert_eq!(norm360(359.5), 359.5);
}
