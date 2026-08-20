use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::header::Header;
use crate::header::value::Value;
use crate::reader::FitsReader;
use crate::wcs::Wcs;
use crate::wcs::celestial_pole::CelestialPole;
use crate::wcs::internals::TAN_GOLDEN;
use crate::wcs::internals::assert_astropy_golden;
use crate::wcs::projection::Projection;
use crate::wcs::wcs_axis::WcsAxis;
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
    assert_astropy_golden(&w, TAN_GOLDEN, "TAN image");
}

#[test]
fn world_to_pixel_inverts_pixel_to_world() {
    // Round-trip our own full-precision forward output. The transform is accurate
    // to ~1e-9° throughout; near the reference point the 1″/px scale amplifies that
    // to ~1e-6 px, so test at 1e-5 px (≈ 10 nano-arcsec) — far tighter than any
    // real use needs.
    let w = open_wcs("wcs_tan.fits");
    for &(px, py, _, _) in TAN_GOLDEN {
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
        Wcs::from_header(&header, None).unwrap()
    };

    let sin = build("SIN");
    assert!(matches!(
        sin.pixel_to_world(&[2.0, 1.0]),
        Err(FitsError::WcsProjectionDomain { projection: "SIN" })
    ));

    let zpn = build("ZPN");
    assert!(matches!(
        zpn.pixel_to_world(&[2.0, 1.0]),
        Err(FitsError::WcsNoConvergence { algorithm: "ZPN" })
    ));

    let tan = open_wcs("wcs_tan.fits");
    assert!(matches!(
        tan.world_to_pixel(&[150.0, 100.0]),
        Err(FitsError::WcsProjectionDomain { projection: "TAN" })
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
    assert_eq!(celestial.pole, [45.0, 30.0, 180.0]);
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
    // drive the per-axis loops.
    let mut h = Header::new();
    for value in [-1, 0, 1000] {
        h.set_internal("WCSAXES", value);
        assert!(matches!(
            Wcs::from_header(&h, None),
            Err(FitsError::KeywordOutOfRange { name: "WCSAXES" })
        ));
    }

    h.set_internal("WCAX5", -1);
    assert!(matches!(
        Wcs::from_array_column(&h, 5, None),
        Err(FitsError::KeywordOutOfRange { name: "WCAXn" })
    ));
}
