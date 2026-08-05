use crate::header::value::Value;
use crate::reader::FitsReader;
use crate::wcs::*;
use std::f64::consts::SQRT_2;
use std::fs::File;

/// Load the WCS from the primary header of a fixture.
fn open_wcs(name: &str) -> Wcs {
    let r = FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap();
    Wcs::from_header(&r.hdus[0].header, None).unwrap()
}

fn projection_parameters(parameters: &[f64]) -> [f64; 21] {
    let mut pv = [0.0; 21];
    pv[..parameters.len()].copy_from_slice(parameters);
    pv
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

const CEA_GOLDEN: &[(f64, f64, f64, f64)] = &[
    (20.0, 70.0, 46.7406870828, 30.4886140110),
    (80.0, 30.0, 43.2767613377, 29.4887155113),
];

const CROTA_GOLDEN: &[(f64, f64, f64, f64)] = &[
    (128.0, 128.0, 83.6000000000, 22.0000000000),
    (1.0, 1.0, 83.6909943156, 21.9692606492),
    (256.0, 200.0, 83.5210288338, 22.0055606050),
    (64.0, 192.0, 83.6166986376, 22.0425247793),
];

fn assert_astropy_golden(wcs: &Wcs, golden: &[(f64, f64, f64, f64)], context: &str) {
    for &(px, py, ra, dec) in golden {
        let world = wcs.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (world[0] - ra).abs() < 1e-9 && (world[1] - dec).abs() < 1e-9,
            "{context} at ({px},{py}): got {world:?}, want ({ra},{dec})"
        );
    }
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
    use crate::error::FitsError;
    use crate::header::Header;

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
            Err(FitsError::CoordinateCountMismatch { expected: 2, .. })
        ));
    }
    assert!(matches!(
        tan.axis_world(2, &[1.0, 1.0]),
        Err(FitsError::WcsAxisIndexOutOfBounds { axis: 3, len: 2 })
    ));
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
    let axes = [
        WcsAxis {
            ctype: String::new(),
            cunit: String::new(),
            crval: 0.0,
            crpix: 10.0,
            spectral_frame: None,
        },
        WcsAxis {
            ctype: String::new(),
            cunit: String::new(),
            crval: 0.0,
            crpix: -2.0,
            spectral_frame: None,
        },
        WcsAxis {
            ctype: String::new(),
            cunit: String::new(),
            crval: 0.0,
            crpix: 4.0,
            spectral_frame: None,
        },
    ];
    let matrix = [2.0, -1.0, 0.5, 0.0, 3.0, 4.0, -2.0, 1.0, 1.5];
    let pixel = [13.0, 3.0, 2.0];
    assert_eq!(
        matvec_pixel_offset(&matrix, &pixel, &axes),
        [0.0, 7.0, -4.0]
    );
}

#[test]
fn sin_projection_matches_astropy() {
    // RA---SIN/DEC--SIN, CRPIX 100/100, CRVAL 45/30, 3.6″ pixels, no rotation.
    // Golden values from astropy.wcs — validates the SIN formula, not just that
    // our forward and inverse agree.
    let mut h = Header::new();
    h.set_internal("NAXIS", 2);
    h.set_internal("CTYPE1", "RA---SIN")
        .set_internal("CTYPE2", "DEC--SIN");
    h.set_internal("CRPIX1", 100.0)
        .set_internal("CRPIX2", 100.0);
    h.set_internal("CRVAL1", 45.0).set_internal("CRVAL2", 30.0);
    h.set_internal("CDELT1", -1e-3).set_internal("CDELT2", 1e-3);
    let w = Wcs::from_header(&h, None).unwrap();
    let golden: &[(f64, f64, f64, f64)] = &[
        (100.0, 100.0, 45.000000000000, 30.000000000000),
        (50.0, 150.0, 45.057764154844, 30.049987404157),
        (1.0, 1.0, 45.114201616520, 29.900950619091),
        (180.0, 20.0, 44.907698264374, 29.919967754584),
    ];
    for &(px, py, ra, dec) in golden {
        let out = w.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (out[0] - ra).abs() < 1e-9 && (out[1] - dec).abs() < 1e-9,
            "SIN at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
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
fn slant_sin_matches_the_standard_equations() {
    let pv = projection_parameters(&[0.0, 0.2, -0.1]);
    let cases = [
        (30.0, 60.0, 15.859180663294788, -25.577418186492753),
        (-40.0, 45.0, -22.685738719896246, -32.713_858_525_468_76),
    ];
    for (phi, theta, x, y) in cases {
        // Paper II eqs. 61–62 with σ = 1 − sin θ:
        // x/r0 = cos θ sin φ + 0.2σ; y/r0 = −cos θ cos φ − 0.1σ.
        let projected = Projection::Sin.project(phi, theta, &pv).unwrap();
        assert!((projected.x - x).abs() < 1e-12, "x = {}", projected.x);
        assert!((projected.y - y).abs() < 1e-12, "y = {}", projected.y);
        let radial = Projection::Sin
            .project(phi, theta, &projection_parameters(&[]))
            .unwrap();
        assert_ne!([projected.x, projected.y], [radial.x, radial.y]);

        let native = Projection::Sin.deproject(x, y, &pv).unwrap();
        assert!((norm180(native.phi - phi)).abs() < 1e-12);
        assert!((native.theta - theta).abs() < 1e-12);
    }
}

#[test]
fn legacy_crota_rotation_matches_astropy() {
    use crate::header::Header;
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
fn allsky_projections_match_astropy() {
    use crate::header::Header;
    // AIT/MOL, CRPIX 50/50, CRVAL 45/30, CDELT (−0.2, 0.2). astropy golden.
    let golden: &[(&str, f64, f64, f64, f64)] = &[
        ("AIT", 20.0, 70.0, 52.2235197328, 33.8100763254),
        ("AIT", 80.0, 30.0, 38.3347274957, 25.8258310813),
        ("MOL", 20.0, 70.0, 52.9816602799, 33.3699739563),
        ("MOL", 80.0, 30.0, 37.5753525553, 26.1818233270),
    ];
    for &(proj, px, py, ra, dec) in golden {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{proj}"));
        h.set_internal("CTYPE2", format!("DEC--{proj}"));
        h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
        h.set_internal("CRVAL1", 45.0).set_internal("CRVAL2", 30.0);
        h.set_internal("CDELT1", -0.2).set_internal("CDELT2", 0.2);
        let w = Wcs::from_header(&h, None).unwrap();
        let out = w.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (out[0] - ra).abs() < 1e-7 && (out[1] - dec).abs() < 1e-7,
            "{proj} at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
}

#[test]
fn cea_lambda_pv_matches_astropy() {
    use crate::header::Header;
    // CEA with λ = PV2_1 = 0.5. astropy golden.
    let mut h = Header::new();
    h.set_internal("NAXIS", 2);
    h.set_internal("CTYPE1", "RA---CEA")
        .set_internal("CTYPE2", "DEC--CEA");
    h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
    h.set_internal("CRVAL1", 45.0).set_internal("CRVAL2", 30.0);
    h.set_internal("CDELT1", -0.05).set_internal("CDELT2", 0.05);
    h.set_internal("PV2_1", 0.5);
    let w = Wcs::from_header(&h, None).unwrap();
    assert_astropy_golden(&w, CEA_GOLDEN, "CEA λ image");
}

#[test]
fn parameterized_projections_match_astropy() {
    use crate::header::Header;
    // (proj, crval2, cdelt, PVs, [(px,py,ra,dec)…]) golden from astropy.
    struct Case {
        proj: &'static str,
        cv2: f64,
        cd: f64,
        pv: &'static [(usize, f64)],
        pts: &'static [(f64, f64, f64, f64)],
    }
    let cases = [
        Case {
            proj: "ZPN",
            cv2: 30.0,
            cd: 0.2,
            pv: &[(1, 1.0), (3, 0.1)],
            pts: &[
                (20.0, 70.0, 52.208830352, 33.797790311),
                (80.0, 30.0, 38.346539565, 25.839502283),
            ],
        },
        Case {
            proj: "CYP",
            cv2: 0.0,
            cd: 0.5,
            pv: &[(1, 1.0), (2, 0.5)],
            pts: &[
                (20.0, 70.0, 75.0, 13.273646093),
                (80.0, 30.0, 15.0, -13.273646093),
            ],
        },
        Case {
            proj: "PAR",
            cv2: 0.0,
            cd: 0.5,
            pv: &[],
            pts: &[
                (20.0, 70.0, 60.1875, 9.554215610),
                (80.0, 30.0, 29.8125, -9.554215610),
            ],
        },
        Case {
            proj: "COP",
            cv2: 45.0,
            cd: 0.5,
            pv: &[(1, 45.0), (2, 15.0)],
            pts: &[
                (20.0, 70.0, 70.886680135, 52.802260739),
                (80.0, 30.0, 26.716181056, 33.063457476),
            ],
        },
        Case {
            proj: "COE",
            cv2: 45.0,
            cd: 0.5,
            pv: &[(1, 45.0), (2, 15.0)],
            pts: &[
                (20.0, 70.0, 70.744981732, 52.427253763),
                (80.0, 30.0, 26.612080121, 33.642902217),
            ],
        },
        Case {
            proj: "COD",
            cv2: 45.0,
            cd: 0.5,
            pv: &[(1, 45.0), (2, 15.0)],
            pts: &[
                (20.0, 70.0, 70.845584231, 52.615170165),
                (80.0, 30.0, 26.678352755, 33.316436438),
            ],
        },
        Case {
            proj: "COO",
            cv2: 45.0,
            cd: 0.5,
            pv: &[(1, 45.0), (2, 15.0)],
            pts: &[
                (20.0, 70.0, 70.936065152, 52.798760425),
                (80.0, 30.0, 26.752614879, 32.966997552),
            ],
        },
        Case {
            proj: "BON",
            cv2: 30.0,
            cd: 0.5,
            pv: &[(1, 45.0)],
            pts: &[
                (40.0, 60.0, 51.090826613, 34.738247010),
                (70.0, 35.0, 34.224478842, 21.570942288),
            ],
        },
        Case {
            proj: "AIR",
            cv2: 60.0,
            cd: 0.3,
            pv: &[(1, 45.0)],
            pts: &[
                (40.0, 60.0, 51.871584561, 62.956093827),
                (70.0, 35.0, 34.141611622, 54.816671832),
            ],
        },
        Case {
            proj: "AZP",
            cv2: 60.0,
            cd: 0.3,
            pv: &[(1, 2.0), (2, 30.0)],
            pts: &[
                (40.0, 60.0, 51.434150697, 62.429650080),
                (70.0, 35.0, 34.214637058, 55.561587347),
            ],
        },
        Case {
            proj: "PCO",
            cv2: 0.0,
            cd: 0.5,
            pv: &[],
            pts: &[
                (40.0, 60.0, 50.019002131, 4.980985613),
                (70.0, 35.0, 34.915451766, -7.386849830),
                (55.0, 55.0, 42.497621311, 2.497620932),
            ],
        },
        Case {
            proj: "SZP",
            cv2: 60.0,
            cd: 0.3,
            pv: &[(1, 2.0), (2, 180.0), (3, 60.0)],
            pts: &[
                (40.0, 60.0, 51.569468511, 62.792802068),
                (70.0, 35.0, 34.554236543, 54.849394924),
                (55.0, 45.0, 42.132530460, 58.453175902),
            ],
        },
    ];
    for c in &cases {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{}", c.proj));
        h.set_internal("CTYPE2", format!("DEC--{}", c.proj));
        h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
        h.set_internal("CRVAL1", 45.0).set_internal("CRVAL2", c.cv2);
        h.set_internal("CDELT1", -c.cd).set_internal("CDELT2", c.cd);
        for &(m, v) in c.pv {
            h.set_internal(&format!("PV2_{m}"), v);
        }
        let w = Wcs::from_header(&h, None).unwrap();
        for &(px, py, ra, dec) in c.pts {
            let out = w.pixel_to_world(&[px, py]).unwrap();
            assert!(
                (out[0] - ra).abs() < 1e-7 && (out[1] - dec).abs() < 1e-7,
                "{} at ({px},{py}): got {out:?}, want ({ra},{dec})",
                c.proj
            );
        }
    }
}

#[test]
fn projection_parameters_use_standard_defaults() {
    let build = |projection: &str, parameters: &[(usize, f64)]| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{projection}"));
        h.set_internal("CTYPE2", format!("DEC--{projection}"));
        for &(m, value) in parameters {
            h.set_internal(&format!("PV2_{m}"), value);
        }
        Wcs::from_header(&h, None).unwrap()
    };

    assert_eq!(build("AIR", &[]).celestial.unwrap().pv[1], 90.0);

    let cyp = build("CYP", &[]).celestial.unwrap();
    assert_eq!([cyp.pv[1], cyp.pv[2]], [1.0, 1.0]);

    assert_eq!(build("CEA", &[]).celestial.unwrap().pv[1], 1.0);

    let szp = build("SZP", &[(1, 2.0)]).celestial.unwrap();
    assert_eq!([szp.pv[1], szp.pv[2], szp.pv[3]], [2.0, 0.0, 90.0]);

    let hpx = build("HPX", &[]).celestial.unwrap();
    assert_eq!([hpx.pv[1], hpx.pv[2]], [4.0, 3.0]);

    let explicit_zero = build("CYP", &[(1, 0.0), (2, 1.0)]).celestial.unwrap();
    assert_eq!([explicit_zero.pv[1], explicit_zero.pv[2]], [0.0, 1.0]);
}

#[test]
fn degenerate_projection_parameters_are_rejected() {
    use crate::error::FitsError;

    let parse = |projection: &str, parameters: &[(usize, f64)]| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{projection}"));
        h.set_internal("CTYPE2", format!("DEC--{projection}"));
        for &(m, value) in parameters {
            h.set_internal(&format!("PV2_{m}"), value);
        }
        Wcs::from_header(&h, None)
    };

    assert!(matches!(
        parse("CEA", &[(1, 0.0)]),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        parse("CYP", &[(2, 0.0)]),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        parse("CYP", &[(1, -1.0), (2, 1.0)]),
        Err(FitsError::InvalidValue { .. })
    ));
    for parameters in [[(1, 0.0)], [(1, -1.0)], [(2, 0.0)], [(2, -1.0)]] {
        assert!(matches!(
            parse("HPX", &parameters),
            Err(FitsError::InvalidValue { .. })
        ));
    }
}

#[test]
fn unsupported_projection_codes_reject_complete_transforms() {
    use crate::error::FitsError;
    use crate::header::Header;
    // Short codes represent space-padded algorithm names after FITS text trimming.
    for code in ["XPH", "UV", "U"] {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{code}"));
        h.set_internal("CTYPE2", format!("DEC--{code}"));
        h.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
        h.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", 20.0);
        h.set_internal("CDELT1", 2.0).set_internal("CDELT2", 3.0);
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(w.view().unsupported_axes, [0, 1], "{code} axes flagged");
        assert!(w.celestial.is_none(), "{code} not decoded as a projection");
        assert!(matches!(
            w.pixel_to_world(&[3.0, 4.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
        assert!(matches!(
            w.world_to_pixel(&[10.0, 20.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
    }
}

#[test]
fn mismatched_celestial_projections_are_rejected() {
    use crate::error::FitsError;
    use crate::header::Header;
    // §8.2: the longitude and latitude axes must share one projection. Pairing
    // RA---TAN with DEC--SIN is malformed — reject it rather than silently adopt
    // whichever axis is seen first.
    let mut h = Header::new();
    h.set_internal("NAXIS", 2);
    h.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--SIN");
    h.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
    h.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", 20.0);
    h.set_internal("CDELT1", 1.0).set_internal("CDELT2", 1.0);
    assert!(matches!(
        Wcs::from_header(&h, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));

    // A galactic-frame pair sharing TAN builds fine — exercises the one shared
    // classifier on the non-RA/DEC longitude/latitude forms (`GLON`/`GLAT`).
    let mut g = Header::new();
    g.set_internal("NAXIS", 2);
    g.set_internal("CTYPE1", "GLON-TAN")
        .set_internal("CTYPE2", "GLAT-TAN");
    g.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
    g.set_internal("CRVAL1", 30.0).set_internal("CRVAL2", 10.0);
    g.set_internal("CDELT1", -1.0).set_internal("CDELT2", 1.0);
    let w = Wcs::from_header(&g, None).unwrap();
    assert!(
        w.celestial.is_some(),
        "GLON/GLAT TAN pair is a celestial WCS"
    );
}

#[test]
fn degenerate_conic_without_pv1_rejects_complete_transforms() {
    use crate::error::FitsError;
    use crate::header::Header;
    // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be non-zero; absent
    // (→ 0) the cone is degenerate (1/tan 0 → NaN). Rather than return NaN, the WCS
    // flags the celestial axes, so complete transforms fail rather than returning
    // NaN or silently relabeling linear-stage coordinates as sky coordinates.
    for code in ["COP", "COE", "COD", "COO"] {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{code}"));
        h.set_internal("CTYPE2", format!("DEC--{code}"));
        h.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
        h.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", 20.0);
        h.set_internal("CDELT1", 2.0).set_internal("CDELT2", 3.0);
        // No PV2_1 ⇒ θ_a = 0.
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(w.view().unsupported_axes, [0, 1], "{code} axes flagged");
        assert!(w.celestial.is_none(), "{code} degenerate, not deprojected");
        assert!(matches!(
            w.pixel_to_world(&[3.0, 4.0]),
            Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0, 1]
        ));
    }
    // A conic *with* a valid θ_a is still decoded normally (not flagged).
    let mut ok = Header::new();
    ok.set_internal("NAXIS", 2);
    ok.set_internal("CTYPE1", "RA---COP")
        .set_internal("CTYPE2", "DEC--COP");
    ok.set_internal("CRPIX1", 1.0).set_internal("CRPIX2", 1.0);
    ok.set_internal("CRVAL1", 10.0).set_internal("CRVAL2", 45.0);
    ok.set_internal("CDELT1", 0.5)
        .set_internal("CDELT2", 0.5)
        .set_internal("PV2_1", 45.0);
    let w = Wcs::from_header(&ok, None).unwrap();
    assert!(w.view().unsupported_axes.is_empty() && w.celestial.is_some());
}

#[test]
fn bonne_with_zero_theta1_equals_sfl() {
    use crate::header::Header;
    // §5.5.1: Bonne's projection at θ₁ = 0 is exactly the sinusoidal SFL. A BON
    // header with PV2_1 = 0 must decode identically to an SFL header (and never hit
    // the 1/tan 0 singularity), so it is *decoded* — not flagged unsupported.
    let build = |proj: &str, pv1: Option<f64>| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{proj}"));
        h.set_internal("CTYPE2", format!("DEC--{proj}"));
        h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
        h.set_internal("CRVAL1", 45.0).set_internal("CRVAL2", 0.0);
        h.set_internal("CDELT1", -0.5).set_internal("CDELT2", 0.5);
        if let Some(v) = pv1 {
            h.set_internal("PV2_1", v);
        }
        Wcs::from_header(&h, None).unwrap()
    };
    let bon = build("BON", Some(0.0));
    let sfl = build("SFL", None);
    assert!(bon.celestial.is_some() && bon.view().unsupported_axes.is_empty());
    for &(px, py) in &[(20.0, 70.0), (80.0, 30.0), (55.0, 45.0)] {
        let b = bon.pixel_to_world(&[px, py]).unwrap();
        let s = sfl.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (b[0] - s[0]).abs() < 1e-10 && (b[1] - s[1]).abs() < 1e-10,
            "BON θ₁=0 vs SFL at ({px},{py}): {b:?} vs {s:?}"
        );
        assert!(b.iter().all(|v| v.is_finite()));
    }
    // The forward (SFL) path round-trips too.
    let world = bon.pixel_to_world(&[40.0, 60.0]).unwrap();
    let p = bon.world_to_pixel(&world).unwrap();
    assert!(
        (p[0] - 40.0).abs() < 1e-7 && (p[1] - 60.0).abs() < 1e-7,
        "BON θ₁=0 round-trip: {p:?}"
    );
}

#[test]
fn conflicting_linear_keywords_are_rejected() {
    use crate::error::FitsError;
    use crate::header::Header;
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
    assert_eq!(w.matrix, [2.0, 0.0, 0.0, 3.0]);
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

/// Every projection's deprojection inverts its forward projection.
#[test]
fn projections_round_trip() {
    use Projection::*;
    // (projection, PV params) — empty PV for the no-parameter projections.
    let cases: &[(Projection, &[f64])] = &[
        (Tan, &[]),
        (Sin, &[]),
        (Arc, &[]),
        (Stg, &[]),
        (Zea, &[]),
        (Car, &[]),
        (Cea, &[0.0, 1.0]),
        (Mer, &[]),
        (Sfl, &[]),
        (Ait, &[]),
        (Mol, &[]),
        (Zpn, &[0.0, 1.0, 0.0, 0.1]),
        (Cyp, &[0.0, 1.0, 0.5]),
        (Par, &[]),
        (Cop, &[0.0, 45.0, 15.0]),
        (Coe, &[0.0, 45.0, 15.0]),
        (Cod, &[0.0, 45.0, 15.0]),
        (Coo, &[0.0, 45.0, 15.0]),
        (Bon, &[0.0, 45.0]),
        (Air, &[0.0, 45.0]),
        (Azp, &[0.0, 2.0, 30.0]),
        (Pco, &[]),
        (Szp, &[0.0, 2.0, 180.0, 60.0]),
        (Tsc, &[]),
        (Csc, &[]),
        (Qsc, &[]),
        (Hpx, &[0.0, 4.0, 3.0]),
    ];
    for &(proj, pv) in cases {
        let pv = projection_parameters(pv);
        // Positive native latitudes, away from the poles: in-domain for every
        // family (zenithal θ > 0, conics near θ_a, perspective non-divergent).
        for &(phi, theta) in &[(30.0_f64, 70.0_f64), (-40.0, 50.0), (20.0, 55.0)] {
            let projected = proj.project(phi, theta, &pv).unwrap();
            let native = proj.deproject(projected.x, projected.y, &pv).unwrap();
            // CSC's published inverse polynomial is approximate; wcslib's own closure limit is 0.04°.
            let tolerance = if proj == Csc { 4e-2 } else { 1e-7 };
            assert!(
                norm180(native.phi - phi).abs() < tolerance
                    && (native.theta - theta).abs() < tolerance,
                "{proj:?}: ({phi},{theta}) → ({},{}) → ({},{})",
                projected.x,
                projected.y,
                native.phi,
                native.theta
            );
        }
    }
}

#[test]
fn cube_projections_match_wcslib_faces_and_interiors() {
    #[derive(Debug)]
    struct Golden {
        projection: Projection,
        projected: [[f64; 2]; 2],
        deprojected: [[f64; 2]; 2],
    }

    let pv = projection_parameters(&[]);
    let faces = [
        (0.0, 0.0, 0.0, 0.0),
        (90.0, 0.0, 90.0, 0.0),
        (180.0, 0.0, 180.0, 0.0),
        (-90.0, 0.0, 270.0, 0.0),
        (0.0, 90.0, 0.0, 90.0),
        (0.0, -90.0, 0.0, -90.0),
        (45.0, 0.0, 45.0, 0.0),
        (0.0, 45.0, 0.0, 45.0),
        (45.0, 35.264_389_682_754_654, 45.0, 45.0),
    ];
    for projection in [Projection::Tsc, Projection::Csc, Projection::Qsc] {
        for (phi, theta, x, y) in faces {
            let projected = projection.project(phi, theta, &pv).unwrap();
            assert!(
                (projected.x - x).abs() < 1e-12 && (projected.y - y).abs() < 1e-12,
                "{projection:?} face ({phi},{theta}): {projected:?}"
            );
        }
    }

    let cases = [
        Golden {
            projection: Projection::Tsc,
            projected: [
                [25.980_762_113_533_153, 18.912_448_145_754_276],
                [244.019_237_886_466_87, -43.600_895_814_340_646],
            ],
            deprojected: [[30.0, 20.0], [-120.0, -40.0]],
        },
        Golden {
            projection: Projection::Csc,
            projected: [
                [31.303_854_882_717_133, 23.855_872_750_282_288],
                [240.086_374_282_836_9, -44.205_473_363_399_506],
            ],
            deprojected: [
                [30.000_112_252_702_29, 19.999_029_202_196_91],
                [-119.998_951_963_376_24, -39.998_865_884_223_214],
            ],
        },
        Golden {
            projection: Projection::Qsc,
            projected: [
                [31.867_419_121_895_73, 24.348_069_968_897_35],
                [241.782_551_767_309_14, -44.232_092_201_227_864],
            ],
            deprojected: [[30.0, 20.0], [-120.0, -40.0]],
        },
    ];
    let native = [[30.0, 20.0], [-120.0, -40.0]];
    for case in cases {
        for (index, point) in native.iter().enumerate() {
            let projected = case.projection.project(point[0], point[1], &pv).unwrap();
            assert!(
                (projected.x - case.projected[index][0]).abs() < 1e-12
                    && (projected.y - case.projected[index][1]).abs() < 1e-12,
                "{case:?} forward {index}: {projected:?}"
            );
            let deprojected = case
                .projection
                .deproject(projected.x, projected.y, &pv)
                .unwrap();
            assert!(
                norm180(deprojected.phi - case.deprojected[index][0]).abs() < 1e-12
                    && (deprojected.theta - case.deprojected[index][1]).abs() < 1e-12,
                "{case:?} inverse {index}: {deprojected:?}"
            );
        }
    }
}

#[test]
fn cube_projection_domains_reject_points_outside_the_face_cross() {
    use crate::error::FitsError;

    let pv = projection_parameters(&[]);
    for projection in [Projection::Tsc, Projection::Csc, Projection::Qsc] {
        assert!(matches!(
            projection.deproject(100.0, 100.0, &pv),
            Err(FitsError::WcsProjectionDomain { projection: code })
                if code == projection.code()
        ));
        let corner = projection.deproject(315.0, 45.0, &pv).unwrap();
        assert!(corner.phi.is_finite() && corner.theta.is_finite());
    }
}

#[test]
fn hpx_matches_wcslib_defaults_transitions_and_parameters() {
    #[derive(Debug)]
    struct Golden {
        parameters: [f64; 2],
        native: [f64; 2],
        projected: [f64; 2],
    }

    let cases = [
        Golden {
            parameters: [4.0, 3.0],
            native: [30.0, 30.0],
            projected: [30.0, 33.749_999_999_999_99],
        },
        Golden {
            parameters: [4.0, 3.0],
            native: [30.0, 60.0],
            projected: [35.490_381_056_766_58, 61.471_143_170_299_726],
        },
        Golden {
            parameters: [4.0, 3.0],
            native: [-120.0, -60.0],
            projected: [-125.490_381_056_766_58, -61.471_143_170_299_726],
        },
        Golden {
            parameters: [4.0, 3.0],
            native: [10.0, 41.810_314_895_778_596],
            projected: [10.0, 45.0],
        },
        Golden {
            parameters: [3.0, 4.0],
            native: [20.0, 70.0],
            projected: [9.823_024_317_517_834, 120.530_927_047_446_5],
        },
        Golden {
            parameters: [3.0, 4.0],
            native: [20.0, -70.0],
            projected: [40.353_951_364_964_33, -120.530_927_047_446_5],
        },
        Golden {
            parameters: [3.0, 4.0],
            native: [-100.0, -70.0],
            projected: [-79.646_048_635_035_67, -120.530_927_047_446_5],
        },
    ];
    for case in cases {
        let pv = projection_parameters(&[0.0, case.parameters[0], case.parameters[1]]);
        let projected = Projection::Hpx
            .project(case.native[0], case.native[1], &pv)
            .unwrap();
        assert!(
            (projected.x - case.projected[0]).abs() < 1e-12
                && (projected.y - case.projected[1]).abs() < 1e-12,
            "{case:?}: {projected:?}"
        );
        let native = Projection::Hpx
            .deproject(projected.x, projected.y, &pv)
            .unwrap();
        assert!(
            norm180(native.phi - case.native[0]).abs() < 1e-12
                && (native.theta - case.native[1]).abs() < 1e-12,
            "{case:?}: {native:?}"
        );
    }

    let default_pv = projection_parameters(&[0.0, 4.0, 3.0]);
    let north = Projection::Hpx.project(0.0, 90.0, &default_pv).unwrap();
    let south = Projection::Hpx.project(100.0, -90.0, &default_pv).unwrap();
    assert_eq!([north.x, north.y], [45.0, 90.0]);
    assert_eq!([south.x, south.y], [135.0, -90.0]);
    assert!(Projection::Hpx.deproject(45.0, 90.0, &default_pv).is_ok());
    assert!(Projection::Hpx.deproject(0.0, 90.0, &default_pv).is_err());

    let standard = Projection::Hpx.project(20.0, -70.0, &default_pv).unwrap();
    let alternate = Projection::Hpx
        .project(20.0, -70.0, &projection_parameters(&[0.0, 3.0, 4.0]))
        .unwrap();
    assert_ne!([standard.x, standard.y], [alternate.x, alternate.y]);
}

#[test]
fn mollweide_poles_are_finite_and_have_canonical_longitude() {
    let pv = projection_parameters(&[]);
    for theta in [-90.0, 90.0] {
        let projected = Projection::Mol.project(73.0, theta, &pv).unwrap();
        assert!(
            projected.x.abs() < 1e-12,
            "projected pole x = {}",
            projected.x
        );
        assert!((projected.y - theta.signum() * SQRT_2 * R2D).abs() < 1e-12);

        let native = Projection::Mol
            .deproject(projected.x, projected.y, &pv)
            .unwrap();
        assert_eq!(native.phi, 0.0);
        assert!((native.theta - theta).abs() < 1e-12);
        assert!(native.phi.is_finite() && native.theta.is_finite());
    }
}

/// Golden pixel→world for all v2 projections, from `astropy.wcs`. Each header is
/// `<RA|DEC>---<PROJ>`, CRPIX 50/50, CDELT (−0.05, 0.05); zenithal use CRVAL
/// (150, 2.5), cylindrical CRVAL (45, 30) so the full pole computation runs.
#[test]
fn projections_match_astropy() {
    use crate::header::Header;
    let golden: &[(&str, f64, f64, f64, f64, f64, f64)] = &[
        ("STG", 150.0, 2.5, 10.0, 80.0, 152.0043337166, 3.9979316935),
        ("STG", 150.0, 2.5, 90.0, 20.0, 148.0002415773, 0.9990200798),
        ("ZEA", 150.0, 2.5, 10.0, 80.0, 152.0048114944, 3.9982876800),
        ("ZEA", 150.0, 2.5, 30.0, 60.0, 151.0013752965, 2.9996013662),
        ("CAR", 45.0, 30.0, 10.0, 80.0, 47.3445169495, 31.4795416251),
        ("CAR", 45.0, 30.0, 90.0, 20.0, 42.7252855755, 28.4801507052),
        ("CEA", 45.0, 30.0, 10.0, 80.0, 47.3445210618, 31.4797129894),
        ("CEA", 45.0, 30.0, 30.0, 60.0, 46.1605080109, 30.4949427609),
        ("MER", 45.0, 30.0, 10.0, 80.0, 47.3445128393, 31.4793703430),
        ("MER", 45.0, 30.0, 90.0, 20.0, 42.7252817062, 28.4803219894),
        ("SFL", 45.0, 30.0, 10.0, 80.0, 47.3453204029, 31.4795275997),
        ("SFL", 45.0, 30.0, 30.0, 60.0, 46.1605521236, 30.4949360292),
    ];
    for &(proj, cv1, cv2, px, py, ra, dec) in golden {
        let mut h = Header::new();
        h.set_internal("NAXIS", 2);
        h.set_internal("CTYPE1", format!("RA---{proj}"));
        h.set_internal("CTYPE2", format!("DEC--{proj}"));
        h.set_internal("CRPIX1", 50.0).set_internal("CRPIX2", 50.0);
        h.set_internal("CRVAL1", cv1).set_internal("CRVAL2", cv2);
        h.set_internal("CDELT1", -0.05).set_internal("CDELT2", 0.05);
        let w = Wcs::from_header(&h, None).unwrap();
        let out = w.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (out[0] - ra).abs() < 1e-8 && (out[1] - dec).abs() < 1e-8,
            "{proj} at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
        // Full round-trip.
        let back = w.world_to_pixel(&out).unwrap();
        assert!(
            (back[0] - px).abs() < 1e-6 && (back[1] - py).abs() < 1e-6,
            "{proj} round-trip: {back:?}"
        );
    }
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
fn nonlinear_algorithms_are_classified_independently_of_coordinate_type() {
    use crate::error::FitsError;
    use crate::header::Header;
    let build = |t3: &str| {
        let mut h = Header::new();
        h.set_internal("NAXIS", 3);
        h.set_internal("CTYPE1", "RA---TAN")
            .set_internal("CTYPE2", "DEC--TAN")
            .set_internal("CTYPE3", t3);
        h.set_internal("CRPIX1", 1.0)
            .set_internal("CRPIX2", 1.0)
            .set_internal("CRPIX3", 1.0);
        h.set_internal("CRVAL1", 45.0)
            .set_internal("CRVAL2", 30.0)
            .set_internal("CRVAL3", 1.4e9);
        h.set_internal("CDELT1", -1e-3)
            .set_internal("CDELT2", 1e-3)
            .set_internal("CDELT3", 1e6);
        Wcs::from_header(&h, None).unwrap()
    };
    let cases = [
        ("FREQ", false),
        ("FREQ-LOG", false),
        ("FREQ-TAB", true),
        ("TIME", false),
        ("TIME-LOG", false),
        ("TIME-TAB", true),
        ("ABCD", false),
        ("ABCD-LOG", false),
        ("ABCD-TAB", true),
    ];
    for (ctype, unsupported) in cases {
        let wcs = build(ctype);
        let expected: &[usize] = if unsupported { &[2] } else { &[] };
        assert_eq!(wcs.view().unsupported_axes, expected, "{ctype}");
        assert!(wcs.celestial.is_some(), "{ctype} must retain the TAN pair");

        if unsupported {
            assert!(matches!(
                wcs.pixel_to_world(&[1.0, 1.0, 3.0]),
                Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [2]
            ));
            assert!(matches!(
                wcs.world_to_pixel(&[45.0, 30.0, 1.402e9]),
                Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [2]
            ));
        } else {
            let out = wcs.pixel_to_world(&[1.0, 1.0, 3.0]).unwrap();
            let expected = if ctype.ends_with("-LOG") {
                1.4e9 * (2.0e6_f64 / 1.4e9).exp()
            } else {
                1.402e9
            };
            assert!((out[2] - expected).abs() < 1e-6, "{ctype}: {out:?}");
            assert!((out[0] - 45.0).abs() < 1e-9 && (out[1] - 30.0).abs() < 1e-9);
        }
    }
}

fn grism_wcs(ctype: &str, reference: f64) -> Wcs {
    let mut header = Header::new();
    header
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", ctype)
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", reference)
        .set_internal("CDELT1", 1.0e-7)
        .set_internal("PV1_0", 4.5e5)
        .set_internal("PV1_1", 1.0)
        .set_internal("PV1_2", 27.0)
        .set_internal("PV1_3", 1.765)
        .set_internal("PV1_4", -1.077e6)
        .set_internal("PV1_5", 3.0)
        .set_internal("PV1_6", 5.0);
    Wcs::from_header(&header, None).unwrap()
}

#[test]
fn grism_axes_match_wcslib_and_invert() {
    let cases = [
        (
            "WAVE-GRI",
            650.0e-9,
            [
                (4.700_910_602_533_127_5e-7, -1.0),
                (6.499_999_999_999_996e-7, 1.0),
                (9.710_044_628_145_035e-7, 4.0),
            ],
        ),
        (
            "AWAV-GRA",
            724.52e-9,
            [
                (5.431_109_985_443_893e-7, -1.0),
                (7.245_199_999_999_999e-7, 1.0),
                (1.043_049_342_688_692_3e-6, 4.0),
            ],
        ),
    ];
    for (ctype, reference, goldens) in cases {
        let wcs = grism_wcs(ctype, reference);
        assert!(wcs.view().unsupported_axes.is_empty(), "{ctype}");
        for (expected_world, pixel) in goldens {
            let world = wcs.pixel_to_world(&[pixel]).unwrap();
            assert!(
                (world[0] - expected_world).abs() < 1e-18,
                "{ctype} pixel {pixel}: {world:?}"
            );
            let round_trip = wcs.world_to_pixel(&world).unwrap();
            assert!(
                (round_trip[0] - pixel).abs() < 1e-12,
                "{ctype} pixel {pixel}: {round_trip:?}"
            );
        }
    }
}

#[test]
fn grism_axes_reject_incomplete_or_degenerate_detector_metadata() {
    for (parameter, value) in [("PV1_0", None), ("PV1_1", None), ("PV1_0", Some(0.0))] {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", "WAVE-GRI")
            .set_internal("CRVAL1", 650.0e-9);
        if parameter != "PV1_0" {
            header.set_internal("PV1_0", 4.5e5);
        }
        if parameter != "PV1_1" {
            header.set_internal("PV1_1", 1.0);
        }
        if let Some(value) = value {
            header.set_internal(parameter, value);
        }
        assert!(
            matches!(
                Wcs::from_header(&header, None),
                Err(FitsError::InvalidValue { .. })
            ),
            "{parameter}={value:?}"
        );
    }
}

#[derive(Debug)]
struct SpectralGolden {
    ctype: &'static str,
    unit: &'static str,
    reference: f64,
    increment: f64,
    world: f64,
}

#[test]
fn table_26_spectral_algorithms_match_wcslib() {
    let cases = [
        SpectralGolden {
            ctype: "WAVE-F2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_330_354_017_17,
        },
        SpectralGolden {
            ctype: "VELO-F2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 2_000.006_671_265_423_6,
        },
        SpectralGolden {
            ctype: "AWAV-F2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_811_019_260_08,
        },
        SpectralGolden {
            ctype: "FREQ-W2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_408_571.067_528,
        },
        SpectralGolden {
            ctype: "VELO-W2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.993_328_738_271_7,
        },
        SpectralGolden {
            ctype: "AWAV-W2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_621_269_199_03,
        },
        SpectralGolden {
            ctype: "FREQ-V2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_407_161.033_065_3,
        },
        SpectralGolden {
            ctype: "WAVE-V2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_235_504_845_66,
        },
        SpectralGolden {
            ctype: "AWAV-V2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_716_144_208_21,
        },
        SpectralGolden {
            ctype: "FREQ-A2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_408_571.067_528,
        },
        SpectralGolden {
            ctype: "WAVE-A2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_140_655_716_77,
        },
        SpectralGolden {
            ctype: "VELO-A2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.993_328_738_271_7,
        },
    ];
    for case in cases {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", case.ctype)
            .set_internal("CUNIT1", case.unit)
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", case.reference)
            .set_internal("CDELT1", case.increment)
            .set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert!(wcs.view().unsupported_axes.is_empty(), "{}", case.ctype);
        let world = wcs.pixel_to_world(&[3.0]).unwrap()[0];
        let tolerance = case.world.abs() * 2e-14;
        assert!(
            (world - case.world).abs() <= tolerance,
            "{}: got {world:.17e}, wcslib {:.17e}",
            case.ctype,
            case.world
        );
        let pixel = wcs.world_to_pixel(&[case.world]).unwrap()[0];
        assert!(
            (pixel - 3.0).abs() < 2e-10,
            "{} inverse: {pixel:.17e}",
            case.ctype
        );
    }
}

#[test]
fn derived_spectral_types_match_wcslib() {
    let cases = [
        SpectralGolden {
            ctype: "ENER-W2F",
            unit: "J",
            reference: 9.411_715_746_760_2e-25,
            increment: 6.626_075_5e-28,
            world: 9.424_986_583_740_556e-25,
        },
        SpectralGolden {
            ctype: "WAVN-W2F",
            unit: "/m",
            reference: 4.737_963_591_465_666,
            increment: 0.003_335_640_951_981_520_5,
            world: 4.744_644_280_102_364,
        },
        SpectralGolden {
            ctype: "VRAD-W2F",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.986_657_561_148_6,
        },
        SpectralGolden {
            ctype: "VOPT-F2W",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 2_000.013_342_678_547,
        },
        SpectralGolden {
            ctype: "ZOPT-F2W",
            unit: "",
            reference: 0.0,
            increment: 1e-5,
            world: 2.000_040_000_793_568e-5,
        },
        SpectralGolden {
            ctype: "BETA-F2V",
            unit: "",
            reference: 0.0,
            increment: 3.335_640_951_981_520_5e-6,
            world: 6.671_304_156_909_189e-6,
        },
    ];
    for case in cases {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", case.ctype)
            .set_internal("CUNIT1", case.unit)
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", case.reference)
            .set_internal("CDELT1", case.increment)
            .set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let world = wcs.pixel_to_world(&[3.0]).unwrap()[0];
        assert!(
            (world - case.world).abs() <= case.world.abs() * 5e-11,
            "{}: got {world:.17e}, wcslib {:.17e}",
            case.ctype,
            case.world
        );
        let pixel = wcs.world_to_pixel(&[case.world]).unwrap()[0];
        assert!((pixel - 3.0).abs() < 2e-10, "{}", case.ctype);
    }
}

#[derive(Debug)]
struct SpectralUnitCase {
    ctype: &'static str,
    unit: &'static str,
    reference: f64,
    canonical_reference: f64,
}

#[test]
fn spectral_units_are_normalized_to_table_25_defaults() {
    let cases = [
        SpectralUnitCase {
            ctype: "FREQ-W2F",
            unit: "GHz",
            reference: 1.42,
            canonical_reference: 1.42e9,
        },
        SpectralUnitCase {
            ctype: "FREQ-W2F",
            unit: "10**9 Hz",
            reference: 1.42,
            canonical_reference: 1.42e9,
        },
        SpectralUnitCase {
            ctype: "ENER-W2F",
            unit: "keV",
            reference: 1.0,
            canonical_reference: 1.602_176_634e-16,
        },
        SpectralUnitCase {
            ctype: "WAVN-W2F",
            unit: "cm**-1",
            reference: 1.0,
            canonical_reference: 100.0,
        },
        SpectralUnitCase {
            ctype: "VRAD-W2F",
            unit: "km s-1",
            reference: 1.0,
            canonical_reference: 1_000.0,
        },
        SpectralUnitCase {
            ctype: "WAVE-F2W",
            unit: "nm",
            reference: 500.0,
            canonical_reference: 5e-7,
        },
        SpectralUnitCase {
            ctype: "ZOPT-F2W",
            unit: "1",
            reference: 0.1,
            canonical_reference: 0.1,
        },
        SpectralUnitCase {
            ctype: "AWAV-F2A",
            unit: "Angstrom",
            reference: 5_000.0,
            canonical_reference: 5e-7,
        },
    ];
    for case in cases {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", case.ctype)
            .set_internal("CUNIT1", case.unit)
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", case.reference)
            .set_internal("CDELT1", case.reference / 100.0)
            .set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = header.wcs(None).unwrap();
        assert_eq!(wcs.view().axes[0].cunit, case.unit);
        assert!(
            (wcs.view().axes[0].crval - case.canonical_reference).abs()
                <= case.canonical_reference.abs() * f64::EPSILON,
            "{} canonical reference: {:.17e}",
            case.ctype,
            wcs.view().axes[0].crval
        );
        let world = wcs.pixel_to_world(&[1.0]).unwrap()[0];
        assert!(
            (world - case.canonical_reference).abs() <= case.canonical_reference.abs() * 1e-10,
            "{} reference: {world:.17e}",
            case.ctype
        );
    }

    let mut invalid = Header::new();
    invalid
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "WAVE-F2W")
        .set_internal("CUNIT1", "Hz")
        .set_internal("CRVAL1", 1.0);
    assert!(matches!(
        invalid.wcs(None),
        Err(crate::error::FitsError::InvalidValue { card }) if card.contains("CUNIT")
    ));
    invalid.set_internal("CUNIT1", "qHz");
    assert!(matches!(
        invalid.wcs(None),
        Err(crate::error::FitsError::InvalidValue { card }) if card.contains("CUNIT")
    ));
}

#[test]
fn logarithmic_axes_apply_domains_units_and_inverse() {
    use crate::error::FitsError;
    use crate::header::Header;

    let mut generic = Header::new();
    generic
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "TIME-LOG")
        .set_internal("CUNIT1", "d")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 100.0)
        .set_internal("CDELT1", 10.0);
    let generic = generic.wcs(None).unwrap();
    let expected = 100.0 * 0.2_f64.exp();
    assert_eq!(generic.view().axes[0].cunit, "d");
    assert!((generic.pixel_to_world(&[3.0]).unwrap()[0] - expected).abs() < 1e-13);
    assert!((generic.world_to_pixel(&[expected]).unwrap()[0] - 3.0).abs() < 1e-13);
    assert!(matches!(
        generic.world_to_pixel(&[0.0]),
        Err(FitsError::WcsCoordinateDomain {
            axis: 0,
            algorithm: "LOG"
        })
    ));

    let mut frequency = Header::new();
    frequency
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "FREQ-LOG")
        .set_internal("CUNIT1", "GHz")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 1.4)
        .set_internal("CDELT1", 0.001);
    let frequency = frequency.wcs(None).unwrap();
    let expected = 1.4e9 * (2.0e6_f64 / 1.4e9).exp();
    assert_eq!(frequency.view().axes[0].cunit, "GHz");
    assert_eq!(frequency.view().axes[0].crval, 1.4e9);
    assert!((frequency.pixel_to_world(&[3.0]).unwrap()[0] - expected).abs() < 1e-6);
    assert!((frequency.world_to_pixel(&[expected]).unwrap()[0] - 3.0).abs() < 1e-12);

    let mut invalid = Header::new();
    invalid
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "ABCD-LOG")
        .set_internal("CRVAL1", 0.0);
    assert!(matches!(
        invalid.wcs(None),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn spectral_rest_metadata_is_required_resolved_and_table_aware() {
    use crate::error::FitsError;
    use crate::header::Header;

    let velocity_axis = |ctype: &str| {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", ctype)
            .set_internal("CUNIT1", "m/s")
            .set_internal("CRPIX1", 1.0)
            .set_internal("CRVAL1", 0.0)
            .set_internal("CDELT1", 1_000.0);
        header
    };
    assert!(matches!(
        velocity_axis("VELO-F2V").wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ or RESTWAV")
    ));
    assert!(matches!(
        velocity_axis("VRAD-W2F").wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ or RESTWAV")
    ));

    let no_rest = velocity_axis("VRAD-V2F").wcs(None).unwrap();
    let world = no_rest.pixel_to_world(&[3.0]).unwrap()[0];
    let speed_of_light: f64 = 2.997_924_58e8;
    let frequency =
        speed_of_light * ((speed_of_light - 2_000.0) / (speed_of_light + 2_000.0)).sqrt();
    let expected = speed_of_light * (1.0 - frequency / speed_of_light);
    assert!((world - expected).abs() < 1e-8);
    assert!((no_rest.world_to_pixel(&[world]).unwrap()[0] - 3.0).abs() < 1e-10);

    let mut by_frequency = velocity_axis("VELO-F2V");
    by_frequency
        .set_internal("RESTFRQ", 1_420_405_751.0)
        .set_internal("SPECSYS", "BARYCENT");
    let by_frequency = by_frequency.wcs(None).unwrap();
    assert_eq!(
        by_frequency.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Topocentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );
    let mut by_wavelength = velocity_axis("VELO-F2V");
    by_wavelength.set_internal("RESTWAV", 2.997_924_58e8 / 1_420_405_751.0);
    let by_wavelength = by_wavelength.wcs(None).unwrap();
    assert!(
        (by_frequency.pixel_to_world(&[3.0]).unwrap()[0]
            - by_wavelength.pixel_to_world(&[3.0]).unwrap()[0])
            .abs()
            < 1e-12
    );
    assert!(matches!(
        by_frequency.world_to_pixel(&[2.997_924_58e8]),
        Err(FitsError::WcsCoordinateDomain {
            axis: 0,
            algorithm: "F2V"
        })
    ));

    let mut deprecated = velocity_axis("VELO-F2V");
    deprecated.set_internal("RESTFREQ", 1_420_405_751.0);
    assert!(
        (deprecated
            .wcs(None)
            .unwrap()
            .pixel_to_world(&[3.0])
            .unwrap()[0]
            - 2_000.006_671_265_423_6)
            .abs()
            < 1e-9
    );

    let mut invalid = velocity_axis("VELO-F2V");
    invalid.set_internal("RESTFRQ", 0.0);
    assert!(matches!(
        invalid.wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RESTFRQ")
    ));

    let mut pixel_list = Header::new();
    pixel_list
        .set_internal("TCTYP2", "VELO-F2V")
        .set_internal("TCUNI2", "m/s")
        .set_internal("TCRPX2", 1.0)
        .set_internal("TCRVL2", 0.0)
        .set_internal("TCDLT2", 1_000.0)
        .set_internal("RFRQ2", 1_420_405_751.0)
        .set_internal("SPEC2", "BARYCENT")
        .set_internal("SOBS2", "GEOCENTR")
        .set_internal("TCTYP4", "WAVE")
        .set_internal("TCUNI4", "m")
        .set_internal("TCRPX4", 1.0)
        .set_internal("TCRVL4", 5.0e-7)
        .set_internal("TCDLT4", 1.0e-9)
        .set_internal("RWAV4", 5.0e-7)
        .set_internal("SPEC4", "SOURCE")
        .set_internal("SOBS4", "HELIOCEN");
    let pixel_list = pixel_list.wcs_pixel_list(&[2, 4], None).unwrap();
    assert!(
        (pixel_list.pixel_to_world(&[3.0, 1.0]).unwrap()[0] - 2_000.006_671_265_423_6).abs() < 1e-9
    );
    assert_eq!(
        pixel_list.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Barycentric),
            observer: SpectralReferenceFrame::Geocentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );
    assert_eq!(
        pixel_list.view().axes[1].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::Source),
            observer: SpectralReferenceFrame::Heliocentric,
            rest_frequency_hz: None,
            rest_wavelength_m: Some(5.0e-7),
        })
    );

    let mut vector = Header::new();
    vector
        .set_internal("WCAX5A", 1)
        .set_internal("1CTY5A", "VELO-F2V")
        .set_internal("1CUN5A", "m/s")
        .set_internal("1CRP5A", 1.0)
        .set_internal("1CRV5A", 0.0)
        .set_internal("1CDE5A", 1_000.0)
        .set_internal("RWAV5A", 2.997_924_58e8 / 1_420_405_751.0)
        .set_internal("SPEC5A", "LSRK")
        .set_internal("SOBS5A", "TOPOCENT");
    let vector = vector.wcs_array_column(5, Some('A')).unwrap();
    assert!((vector.pixel_to_world(&[3.0]).unwrap()[0] - 2_000.006_671_265_423_6).abs() < 1e-9);
    assert_eq!(
        vector.view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::LsrKinematic),
            observer: SpectralReferenceFrame::Topocentric,
            rest_frequency_hz: None,
            rest_wavelength_m: Some(2.997_924_58e8 / 1_420_405_751.0),
        })
    );

    let mut alternate = Header::new();
    alternate
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "WAVE")
        .set_internal("CTYPE1A", "FREQ")
        .set_internal("SPECSYS", "GEOCENTR")
        .set_internal("SPECSYSA", "CMBDIPOL")
        .set_internal("SSYSOBSA", "BARYCENT")
        .set_internal("RESTFRQA", 1_420_405_751.0);
    assert_eq!(
        alternate.wcs(Some('A')).unwrap().view().axes[0].spectral_frame,
        Some(SpectralFrame {
            coordinate: Some(SpectralReferenceFrame::CmbDipole),
            observer: SpectralReferenceFrame::Barycentric,
            rest_frequency_hz: Some(1_420_405_751.0),
            rest_wavelength_m: None,
        })
    );

    alternate.set_internal("SPECSYSA", "UNKNOWN");
    assert!(matches!(
        alternate.wcs(Some('A')),
        Err(FitsError::InvalidValue { card }) if card.contains("SPECSYS")
    ));

    for (value, expected) in [
        ("TOPOCENT", SpectralReferenceFrame::Topocentric),
        ("GEOCENTR", SpectralReferenceFrame::Geocentric),
        ("BARYCENT", SpectralReferenceFrame::Barycentric),
        ("HELIOCEN", SpectralReferenceFrame::Heliocentric),
        ("LSRK", SpectralReferenceFrame::LsrKinematic),
        ("LSRD", SpectralReferenceFrame::LsrDynamic),
        ("GALACTOC", SpectralReferenceFrame::Galactocentric),
        ("LOCALGRP", SpectralReferenceFrame::LocalGroup),
        ("CMBDIPOL", SpectralReferenceFrame::CmbDipole),
        ("SOURCE", SpectralReferenceFrame::Source),
    ] {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", "WAVE")
            .set_internal("SPECSYS", value);
        assert_eq!(
            header.wcs(None).unwrap().view().axes[0]
                .spectral_frame
                .unwrap()
                .coordinate,
            Some(expected),
            "{value}"
        );
    }
}

#[test]
fn celestial_frame_metadata_resolves_defaults_alternates_and_table_forms() {
    use crate::error::FitsError;
    use crate::header::Header;

    let image = |equinox: Option<f64>, radesys: Option<&str>| {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 2)
            .set_internal("CTYPE1", "RA---TAN")
            .set_internal("CTYPE2", "DEC--TAN");
        if let Some(equinox) = equinox {
            header.set_internal("EQUINOX", equinox);
        }
        if let Some(radesys) = radesys {
            header.set_internal("RADESYS", radesys);
        }
        header
    };
    assert_eq!(
        image(None, None).wcs(None).unwrap().view().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Icrs,
            equinox: None,
        })
    );
    assert_eq!(
        image(Some(1950.0), None)
            .wcs(None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4,
            equinox: Some(1950.0),
        })
    );
    assert_eq!(
        image(Some(2000.0), None)
            .wcs(None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );
    assert_eq!(
        image(Some(1975.0), Some("FK4-NO-E"))
            .wcs(None)
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4NoE,
            equinox: Some(1975.0),
        })
    );

    let mut alternate = image(None, Some("GAPPT"));
    alternate
        .set_internal("CTYPE1A", "RA---TAN")
        .set_internal("CTYPE2A", "DEC--TAN")
        .set_internal("EQUINOXA", 1970.0);
    assert_eq!(
        alternate.wcs(None).unwrap().view().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Gappt,
            equinox: None,
        })
    );
    assert_eq!(
        alternate.wcs(Some('A')).unwrap().view().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk4,
            equinox: Some(1970.0),
        })
    );

    let mut pixel_list = Header::new();
    pixel_list
        .set_internal("TCTY2A", "RA---TAN")
        .set_internal("TCTY3A", "DEC--TAN")
        .set_internal("RADE2A", "FK5")
        .set_internal("RADE3A", "FK5")
        .set_internal("EQUI2A", 2000.0)
        .set_internal("EQUI3A", 2000.0);
    assert_eq!(
        pixel_list
            .wcs_pixel_list(&[2, 3], Some('A'))
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );

    let mut vector = Header::new();
    vector
        .set_internal("1CTY5A", "RA---TAN")
        .set_internal("2CTY5A", "DEC--TAN")
        .set_internal("RADE5A", "ICRS");
    assert_eq!(
        vector
            .wcs_array_column(5, Some('A'))
            .unwrap()
            .view()
            .celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Icrs,
            equinox: None,
        })
    );

    let mut unsupported = Header::new();
    unsupported
        .set_internal("TCTYP2", "RA---XPH")
        .set_internal("TCTYP3", "DEC--XPH")
        .set_internal("RADE2", "FK5")
        .set_internal("RADE3", "FK5")
        .set_internal("EQUI2", 2000.0)
        .set_internal("EQUI3", 2000.0);
    let unsupported = unsupported.wcs_pixel_list(&[2, 3], None).unwrap();
    assert_eq!(
        unsupported.view().celestial_frame,
        Some(CelestialFrame {
            reference_frame: CelestialReferenceFrame::Fk5,
            equinox: Some(2000.0),
        })
    );
    assert_eq!(unsupported.view().unsupported_axes, [0, 1]);

    assert!(matches!(
        image(None, Some("J2000")).wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("RADESYS")
    ));
    assert!(matches!(
        image(Some(-1.0), None).wcs(None),
        Err(FitsError::InvalidValue { card }) if card.contains("EQUINOX")
    ));
    pixel_list.set_internal("RADE3A", "FK4");
    assert!(matches!(
        pixel_list.wcs_pixel_list(&[2, 3], Some('A')),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
}

#[test]
fn table_wcs_resolver_matches_table_22() {
    let primary = TableWcsResolver::new(None);
    let alternate = TableWcsResolver::new(Some('A'));
    let axis_cases = [
        (
            TableAxisKeyword::Type,
            "TCTYP17",
            "TCTY17A",
            "3CTYP17",
            "3CTY17A",
        ),
        (
            TableAxisKeyword::Unit,
            "TCUNI17",
            "TCUN17A",
            "3CUNI17",
            "3CUN17A",
        ),
        (
            TableAxisKeyword::ReferenceValue,
            "TCRVL17",
            "TCRV17A",
            "3CRVL17",
            "3CRV17A",
        ),
        (
            TableAxisKeyword::Increment,
            "TCDLT17",
            "TCDE17A",
            "3CDLT17",
            "3CDE17A",
        ),
        (
            TableAxisKeyword::ReferencePoint,
            "TCRPX17",
            "TCRP17A",
            "3CRPX17",
            "3CRP17A",
        ),
    ];
    for (keyword, primary_pixel, alternate_pixel, primary_vector, alternate_vector) in axis_cases {
        assert_eq!(
            primary.pixel_axis_key(keyword, 17).unwrap().as_str(),
            primary_pixel
        );
        assert_eq!(
            alternate.pixel_axis_key(keyword, 17).unwrap().as_str(),
            alternate_pixel
        );
        assert_eq!(
            primary.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
            primary_vector
        );
        assert_eq!(
            alternate.vector_axis_key(keyword, 3, 17).unwrap().as_str(),
            alternate_vector
        );
    }
    assert_eq!(
        primary
            .pixel_axis_key(TableAxisKeyword::Rotation, 17)
            .unwrap()
            .as_str(),
        "TCROT17"
    );
    assert_eq!(
        primary
            .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
            .unwrap()
            .as_str(),
        "3CROT17"
    );
    assert!(
        alternate
            .pixel_axis_key(TableAxisKeyword::Rotation, 17)
            .is_none()
    );
    assert!(
        alternate
            .vector_axis_key(TableAxisKeyword::Rotation, 3, 17)
            .is_none()
    );

    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, false)
            .as_str(),
        "TPC2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Pc, 2, 3, true)
            .as_str(),
        "TP2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, false)
            .as_str(),
        "TCD2_3A"
    );
    assert_eq!(
        alternate
            .pixel_matrix_key(TableMatrixKeyword::Cd, 2, 3, true)
            .as_str(),
        "TC2_3A"
    );
    assert_eq!(
        alternate
            .vector_matrix_key(TableMatrixKeyword::Pc, 2, 3, 17)
            .as_str(),
        "23PC17A"
    );
    assert_eq!(
        alternate.pixel_parameter_key(2, 1, false).as_str(),
        "TPV2_1A"
    );
    assert_eq!(alternate.pixel_parameter_key(2, 1, true).as_str(), "TV2_1A");
    assert_eq!(
        alternate.vector_parameter_key(2, 17, 1, false).as_str(),
        "2PV17_1A"
    );
    assert_eq!(
        alternate.vector_parameter_key(2, 17, 1, true).as_str(),
        "2V17_1A"
    );
    assert_eq!(alternate.column_key("LONP", 17).as_str(), "LONP17A");
    assert_eq!(alternate.column_key("LATP", 17).as_str(), "LATP17A");
    assert_eq!(alternate.column_key("WCAX", 17).as_str(), "WCAX17A");
}

#[test]
fn pixel_list_wcs_matches_the_equivalent_image_wcs() {
    // §8.5: a pixel-list (event) WCS on columns 2,3 must transform identically to
    // an image WCS with the same CTYPE/CRPIX/CRVAL/CDELT and PC rotation.
    let mut tab = Header::new();
    tab.set_internal("TCTYP2", "RA---TAN")
        .set_internal("TCTYP3", "DEC--TAN");
    tab.set_internal("TCRPX2", 256.0)
        .set_internal("TCRPX3", 256.0);
    tab.set_internal("TCRVL2", 150.0)
        .set_internal("TCRVL3", 30.0);
    tab.set_internal("TCDLT2", -1e-3)
        .set_internal("TCDLT3", 1e-3);
    tab.set_internal("TPC2_2", 1.0)
        .set_internal("TPC2_3", -0.05);
    tab.set_internal("TPC3_2", 0.05).set_internal("TPC3_3", 1.0);
    let wt = Wcs::from_pixel_list(&tab, &[2, 3], None).unwrap();

    let mut img = Header::new();
    img.set_internal("NAXIS", 2);
    img.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    img.set_internal("CRPIX1", 256.0)
        .set_internal("CRPIX2", 256.0);
    img.set_internal("CRVAL1", 150.0)
        .set_internal("CRVAL2", 30.0);
    img.set_internal("CDELT1", -1e-3)
        .set_internal("CDELT2", 1e-3);
    img.set_internal("PC1_1", 1.0).set_internal("PC1_2", -0.05);
    img.set_internal("PC2_1", 0.05).set_internal("PC2_2", 1.0);
    let wi = Wcs::from_header(&img, None).unwrap();

    assert!(wt.celestial.is_some(), "pixel-list pair must be celestial");
    for &(px, py) in &[(256.0, 256.0), (1.0, 1.0), (300.0, 100.0), (50.0, 400.0)] {
        let a = wt.pixel_to_world(&[px, py]).unwrap();
        let b = wi.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "pixel-list {a:?} vs image {b:?} at ({px},{py})"
        );
    }

    let mut alternate = Header::new();
    alternate
        .set_internal("TCTY2A", "RA---TAN")
        .set_internal("TCTY3A", "DEC--TAN");
    alternate
        .set_internal("TCRP2A", 256.0)
        .set_internal("TCRP3A", 256.0);
    alternate
        .set_internal("TCRV2A", 150.0)
        .set_internal("TCRV3A", 2.5);
    alternate
        .set_internal("TCDE2A", -0.0002777778)
        .set_internal("TCDE3A", 0.0002777778);
    alternate
        .set_internal("TCUN2A", "deg")
        .set_internal("TCUN3A", "deg");
    alternate
        .set_internal("TP2_2A", 0.96592582628907)
        .set_internal("TP2_3A", -0.25881904510252)
        .set_internal("TP3_2A", 0.25881904510252)
        .set_internal("TP3_3A", 0.96592582628907);
    alternate
        .set_internal("LONP2A", 180.0)
        .set_internal("LATP2A", 2.5);
    let alternate_wcs = Wcs::from_pixel_list(&alternate, &[2, 3], Some('A')).unwrap();
    assert_eq!(
        alternate_wcs.celestial.as_ref().unwrap().pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0,
        }
    );
    assert_astropy_golden(&alternate_wcs, TAN_GOLDEN, "alternate pixel-list TAN");

    tab.set_internal("TCRVL2", "not numeric");
    assert!(matches!(
        Wcs::from_pixel_list(&tab, &[2, 3], None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "TCRVL2"
    ));
}

#[test]
fn vector_cell_wcs_matches_the_equivalent_image_wcs() {
    // §8 Table 22: an image in a binary-table vector cell (here column 5) uses the
    // axis+column-indexed keyword family (`iCTYPn`, `ijPCn`, …, with leading-digit
    // keyword names); it must transform exactly like the equivalent image WCS.
    let mut tab = Header::new();
    tab.set_internal("1CTYP5", "RA---TAN")
        .set_internal("2CTYP5", "DEC--TAN");
    tab.set_internal("1CRPX5", 256.0)
        .set_internal("2CRPX5", 256.0);
    tab.set_internal("1CRVL5", 150.0)
        .set_internal("2CRVL5", 30.0);
    tab.set_internal("1CDLT5", -1e-3)
        .set_internal("2CDLT5", 1e-3);
    tab.set_internal("11PC5", 1.0).set_internal("12PC5", -0.05);
    tab.set_internal("21PC5", 0.05).set_internal("22PC5", 1.0);
    let wt = Wcs::from_array_column(&tab, 5, None).unwrap();

    let mut img = Header::new();
    img.set_internal("NAXIS", 2);
    img.set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    img.set_internal("CRPIX1", 256.0)
        .set_internal("CRPIX2", 256.0);
    img.set_internal("CRVAL1", 150.0)
        .set_internal("CRVAL2", 30.0);
    img.set_internal("CDELT1", -1e-3)
        .set_internal("CDELT2", 1e-3);
    img.set_internal("PC1_1", 1.0).set_internal("PC1_2", -0.05);
    img.set_internal("PC2_1", 0.05).set_internal("PC2_2", 1.0);
    let wi = Wcs::from_header(&img, None).unwrap();

    assert_eq!(wt.view().axes.len(), 2); // rank inferred from the iCTYP5 keywords
    assert!(wt.celestial.is_some(), "vector-cell pair must be celestial");
    for &(px, py) in &[(256.0, 256.0), (1.0, 1.0), (300.0, 100.0), (50.0, 400.0)] {
        let a = wt.pixel_to_world(&[px, py]).unwrap();
        let b = wi.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "vector-cell {a:?} vs image {b:?} at ({px},{py})"
        );
    }

    let mut alternate = Header::new();
    alternate
        .set_internal("1CTY5A", "RA---TAN")
        .set_internal("2CTY5A", "DEC--TAN");
    alternate
        .set_internal("1CRP5A", 256.0)
        .set_internal("2CRP5A", 256.0);
    alternate
        .set_internal("1CRV5A", 150.0)
        .set_internal("2CRV5A", 2.5);
    alternate
        .set_internal("1CDE5A", -0.0002777778)
        .set_internal("2CDE5A", 0.0002777778);
    alternate
        .set_internal("1CUN5A", "deg")
        .set_internal("2CUN5A", "deg");
    alternate
        .set_internal("11PC5A", 0.96592582628907)
        .set_internal("12PC5A", -0.25881904510252)
        .set_internal("21PC5A", 0.25881904510252)
        .set_internal("22PC5A", 0.96592582628907);
    alternate
        .set_internal("LONP5A", 180.0)
        .set_internal("LATP5A", 2.5);
    let inferred = Wcs::from_array_column(&alternate, 5, Some('A')).unwrap();
    assert_eq!(inferred.view().axes.len(), 2);
    assert_eq!(
        inferred.celestial.as_ref().unwrap().pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0,
        }
    );
    assert_astropy_golden(&inferred, TAN_GOLDEN, "alternate vector-cell TAN");

    alternate.set_internal("WCAX5A", 2);
    let explicit = Wcs::from_array_column(&alternate, 5, Some('A')).unwrap();
    assert_astropy_golden(&explicit, TAN_GOLDEN, "ranked alternate vector-cell TAN");

    tab.set_internal("1CRVL5", "not numeric");
    assert!(matches!(
        Wcs::from_array_column(&tab, 5, None),
        Err(FitsError::TypeMismatch { name, .. }) if name == "1CRVL5"
    ));
}

#[test]
fn table_wcs_parameter_aliases_match_astropy() {
    let pixel = |parameter: &str, alternate: bool| {
        let mut header = Header::new();
        if alternate {
            header
                .set_internal("TCTY2A", "RA---CEA")
                .set_internal("TCTY3A", "DEC--CEA");
            header
                .set_internal("TCRP2A", 50.0)
                .set_internal("TCRP3A", 50.0);
            header
                .set_internal("TCRV2A", 45.0)
                .set_internal("TCRV3A", 30.0);
            header
                .set_internal("TCDE2A", -0.05)
                .set_internal("TCDE3A", 0.05);
        } else {
            header
                .set_internal("TCTYP2", "RA---CEA")
                .set_internal("TCTYP3", "DEC--CEA");
            header
                .set_internal("TCRPX2", 50.0)
                .set_internal("TCRPX3", 50.0);
            header
                .set_internal("TCRVL2", 45.0)
                .set_internal("TCRVL3", 30.0);
            header
                .set_internal("TCDLT2", -0.05)
                .set_internal("TCDLT3", 0.05);
        }
        header.set_internal(parameter, 0.5);
        Wcs::from_pixel_list(&header, &[2, 3], alternate.then_some('A')).unwrap()
    };
    for parameter in ["TPV3_1", "TV3_1"] {
        let wcs = pixel(parameter, false);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }
    for parameter in ["TPV3_1A", "TV3_1A"] {
        let wcs = pixel(parameter, true);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }

    let vector = |parameter: &str, alternate: bool| {
        let mut header = Header::new();
        if alternate {
            header.set_internal("WCAX5A", 2);
            header
                .set_internal("1CTY5A", "RA---CEA")
                .set_internal("2CTY5A", "DEC--CEA");
            header
                .set_internal("1CRP5A", 50.0)
                .set_internal("2CRP5A", 50.0);
            header
                .set_internal("1CRV5A", 45.0)
                .set_internal("2CRV5A", 30.0);
            header
                .set_internal("1CDE5A", -0.05)
                .set_internal("2CDE5A", 0.05);
        } else {
            header.set_internal("WCAX5", 2);
            header
                .set_internal("1CTYP5", "RA---CEA")
                .set_internal("2CTYP5", "DEC--CEA");
            header
                .set_internal("1CRPX5", 50.0)
                .set_internal("2CRPX5", 50.0);
            header
                .set_internal("1CRVL5", 45.0)
                .set_internal("2CRVL5", 30.0);
            header
                .set_internal("1CDLT5", -0.05)
                .set_internal("2CDLT5", 0.05);
        }
        header.set_internal(parameter, 0.5);
        Wcs::from_array_column(&header, 5, alternate.then_some('A')).unwrap()
    };
    for parameter in ["2PV5_1", "2V5_1"] {
        let wcs = vector(parameter, false);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }
    for parameter in ["2PV5_1A", "2V5_1A"] {
        let wcs = vector(parameter, true);
        assert_astropy_golden(&wcs, CEA_GOLDEN, parameter);
    }
}

#[test]
fn table_wcs_matrix_aliases_resolve_exactly() {
    let expected = [2.0, 0.5, -0.25, 3.0];
    for (root, alternate) in [
        ("TPC", false),
        ("TP", false),
        ("TCD", false),
        ("TC", false),
        ("TPC", true),
        ("TP", true),
        ("TCD", true),
        ("TC", true),
    ] {
        let suffix = if alternate { "A" } else { "" };
        let mut header = Header::new();
        if alternate {
            header
                .set_internal("TCTY2A", "LINEAR")
                .set_internal("TCTY3A", "LINEAR");
        } else {
            header
                .set_internal("TCTYP2", "LINEAR")
                .set_internal("TCTYP3", "LINEAR");
        }
        header
            .set_internal(&format!("{root}2_2{suffix}"), expected[0])
            .set_internal(&format!("{root}2_3{suffix}"), expected[1])
            .set_internal(&format!("{root}3_2{suffix}"), expected[2])
            .set_internal(&format!("{root}3_3{suffix}"), expected[3]);
        let wcs = Wcs::from_pixel_list(&header, &[2, 3], alternate.then_some('A')).unwrap();
        assert_eq!(wcs.matrix, expected, "{root}, alternate={alternate}");
    }

    for (root, alternate) in [("PC", false), ("CD", false), ("PC", true), ("CD", true)] {
        let suffix = if alternate { "A" } else { "" };
        let mut header = Header::new();
        header.set_internal(&format!("WCAX5{suffix}"), 2);
        header
            .set_internal(&format!("11{root}5{suffix}"), expected[0])
            .set_internal(&format!("12{root}5{suffix}"), expected[1])
            .set_internal(&format!("21{root}5{suffix}"), expected[2])
            .set_internal(&format!("22{root}5{suffix}"), expected[3]);
        let wcs = Wcs::from_array_column(&header, 5, alternate.then_some('A')).unwrap();
        assert_eq!(wcs.matrix, expected, "{root}, alternate={alternate}");
    }
}

#[test]
fn primary_table_wcs_rotation_matches_astropy() {
    let mut pixel = Header::new();
    pixel
        .set_internal("TCTYP2", "RA---TAN")
        .set_internal("TCTYP3", "DEC--TAN");
    pixel
        .set_internal("TCUNI2", "deg")
        .set_internal("TCUNI3", "deg");
    pixel
        .set_internal("TCRPX2", 128.0)
        .set_internal("TCRPX3", 128.0);
    pixel
        .set_internal("TCRVL2", 83.6)
        .set_internal("TCRVL3", 22.0);
    pixel
        .set_internal("TCDLT2", -0.0005)
        .set_internal("TCDLT3", 0.0005);
    pixel.set_internal("TCROT3", 25.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], None).unwrap();
    assert_astropy_golden(&pixel_wcs, CROTA_GOLDEN, "primary pixel-list CROTA");

    let mut vector = Header::new();
    vector.set_internal("WCAX5", 2);
    vector
        .set_internal("1CTYP5", "RA---TAN")
        .set_internal("2CTYP5", "DEC--TAN");
    vector
        .set_internal("1CUNI5", "deg")
        .set_internal("2CUNI5", "deg");
    vector
        .set_internal("1CRPX5", 128.0)
        .set_internal("2CRPX5", 128.0);
    vector
        .set_internal("1CRVL5", 83.6)
        .set_internal("2CRVL5", 22.0);
    vector
        .set_internal("1CDLT5", -0.0005)
        .set_internal("2CDLT5", 0.0005);
    vector.set_internal("2CROT5", 25.0);
    let vector_wcs = Wcs::from_array_column(&vector, 5, None).unwrap();
    assert_astropy_golden(&vector_wcs, CROTA_GOLDEN, "primary vector-cell CROTA");
}

#[test]
fn table_wcs_column_poles_match_the_equivalent_image_wcs() {
    let mut image = Header::new();
    image.set_internal("NAXIS", 2);
    image
        .set_internal("CTYPE1", "RA---CEA")
        .set_internal("CTYPE2", "DEC--CEA");
    image
        .set_internal("CRPIX1", 50.0)
        .set_internal("CRPIX2", 50.0);
    image
        .set_internal("CRVAL1", 45.0)
        .set_internal("CRVAL2", 30.0);
    image
        .set_internal("CDELT1", -0.05)
        .set_internal("CDELT2", 0.05);
    image.set_internal("PV2_1", 0.5);
    image
        .set_internal("LONPOLE", 0.0)
        .set_internal("LATPOLE", -90.0);
    let image_wcs = Wcs::from_header(&image, None).unwrap();
    let image_pole = image_wcs.celestial.as_ref().unwrap().pole;
    assert_eq!(image_pole.ra, 45.0);
    assert!((image_pole.dec + 60.0).abs() < 1e-12, "{image_pole:?}");
    assert_eq!(image_pole.lonpole, 0.0);

    let mut pixel = Header::new();
    pixel
        .set_internal("TCTY2A", "RA---CEA")
        .set_internal("TCTY3A", "DEC--CEA");
    pixel
        .set_internal("TCRP2A", 50.0)
        .set_internal("TCRP3A", 50.0);
    pixel
        .set_internal("TCRV2A", 45.0)
        .set_internal("TCRV3A", 30.0);
    pixel
        .set_internal("TCDE2A", -0.05)
        .set_internal("TCDE3A", 0.05);
    pixel.set_internal("TV3_1A", 0.5);
    pixel
        .set_internal("LONP2A", 0.0)
        .set_internal("LATP2A", -90.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], Some('A')).unwrap();

    let mut vector = Header::new();
    vector.set_internal("WCAX5A", 2);
    vector
        .set_internal("1CTY5A", "RA---CEA")
        .set_internal("2CTY5A", "DEC--CEA");
    vector
        .set_internal("1CRP5A", 50.0)
        .set_internal("2CRP5A", 50.0);
    vector
        .set_internal("1CRV5A", 45.0)
        .set_internal("2CRV5A", 30.0);
    vector
        .set_internal("1CDE5A", -0.05)
        .set_internal("2CDE5A", 0.05);
    vector.set_internal("2V5_1A", 0.5);
    vector
        .set_internal("LONP5A", 0.0)
        .set_internal("LATP5A", -90.0);
    let vector_wcs = Wcs::from_array_column(&vector, 5, Some('A')).unwrap();

    for table_wcs in [&pixel_wcs, &vector_wcs] {
        assert_eq!(
            table_wcs.celestial.as_ref().unwrap().pole,
            image_wcs.celestial.as_ref().unwrap().pole
        );
        for pixel in [[50.0, 50.0], [20.0, 70.0], [80.0, 30.0]] {
            let table_world = table_wcs.pixel_to_world(&pixel).unwrap();
            let image_world = image_wcs.pixel_to_world(&pixel).unwrap();
            assert!(
                (table_world[0] - image_world[0]).abs() < 1e-12
                    && (table_world[1] - image_world[1]).abs() < 1e-12,
                "table {table_world:?} vs image {image_world:?} at {pixel:?}"
            );
        }
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
fn vector_cell_rank_uses_every_supported_keyword_family() {
    let build = |keyword: &str, value: Value| {
        let mut h = Header::new();
        h.set_internal(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set_internal("11CD5", 1.0)
        .set_internal("22CD5", 1.0)
        .set_internal("33CD5", 1.0);
    let cases = [
        build("3CTYP5", Value::Text("LINEAR".to_string())),
        build("3CUNI5", Value::Text("m".to_string())),
        build("3CRPX5", Value::Real(10.0)),
        build("3CRVL5", Value::Real(10.0)),
        build("3CDLT5", Value::Real(2.0)),
        build("3CROT5", Value::Real(10.0)),
        build("3PV5_1", Value::Real(2.0)),
        build("3V5_1", Value::Real(2.0)),
        build("3PS5_1", Value::Text("value".to_string())),
        build("3S5_1", Value::Text("value".to_string())),
        build("13PC5", Value::Real(0.25)),
        cd,
    ];
    for h in &cases {
        assert_eq!(
            Wcs::from_array_column(h, 5, None)
                .unwrap()
                .view()
                .axes
                .len(),
            3
        );
    }

    let mut alternate_cd = Header::new();
    alternate_cd
        .set_internal("11CD5A", 1.0)
        .set_internal("22CD5A", 1.0)
        .set_internal("33CD5A", 1.0);
    let alternate_cases = [
        build("3CTY5A", Value::Text("LINEAR".to_string())),
        build("3CUN5A", Value::Text("m".to_string())),
        build("3CRP5A", Value::Real(10.0)),
        build("3CRV5A", Value::Real(10.0)),
        build("3CDE5A", Value::Real(2.0)),
        build("3PV5_1A", Value::Real(2.0)),
        build("3V5_1A", Value::Real(2.0)),
        build("3PS5_1A", Value::Text("value".to_string())),
        build("3S5_1A", Value::Text("value".to_string())),
        build("13PC5A", Value::Real(0.25)),
        alternate_cd,
    ];
    for header in &alternate_cases {
        assert_eq!(
            Wcs::from_array_column(header, 5, Some('A'))
                .unwrap()
                .view()
                .axes
                .len(),
            3
        );
    }

    let invalid_long_alternate = build("3CTYP5A", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&invalid_long_alternate, 5, Some('A')),
        Err(FitsError::MissingKeyword { name: "iCTYPn" })
    ));

    // A two-digit array axis: the whole leading digit run is the axis, where in
    // `13PC5` above the run is a *pair* of single-digit axes and the rank comes from
    // the second. Rank inference has to admit both readings of a leading run.
    let two_digit = build("12CTYP5", Value::Text("LINEAR".to_string()));
    assert_eq!(
        Wcs::from_array_column(&two_digit, 5, None)
            .unwrap()
            .view()
            .axes
            .len(),
        12
    );

    // A leading zero is not an index (§4.1.2 indices are unpadded), so `03CTYP5`
    // names no axis and the column has no vector WCS at all.
    let padded_index = build("03CTYP5", Value::Text("LINEAR".to_string()));
    assert!(matches!(
        Wcs::from_array_column(&padded_index, 5, None),
        Err(FitsError::MissingKeyword { name: "iCTYPn" })
    ));
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
