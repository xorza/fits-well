use crate::header::value::Value;
use crate::reader::FitsReader;
use crate::wcs::*;
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
            },
            WcsAxis {
                ctype: "DEC--TAN".to_string(),
                cunit: "deg".to_string(),
                crval: 2.5,
                crpix: 256.0,
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
        header.set("NAXIS", 2);
        header
            .set("CTYPE1", format!("RA---{projection}"))
            .set("CTYPE2", format!("DEC--{projection}"));
        header.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
        header.set("CRVAL1", 0.0).set("CRVAL2", 0.0);
        header.set("CDELT1", 100.0).set("CDELT2", 100.0);
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
        Err(FitsError::WcsNoConvergence { projection: "ZPN" })
    ));

    let tan = open_wcs("wcs_tan.fits");
    assert!(matches!(
        tan.world_to_pixel(&[150.0, 100.0]),
        Err(FitsError::WcsProjectionDomain { projection: "TAN" })
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
        let out = w.pixel_to_world(&[px, py]).unwrap();
        assert!(
            (out[0] - ra).abs() < 1e-9 && (out[1] - dec).abs() < 1e-9,
            "SIN at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
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
    h.set("NAXIS", 2);
    h.set("CTYPE1", "RA---TAN").set("CTYPE2", "DEC--TAN");
    h.set("CRPIX1", 128.0).set("CRPIX2", 128.0);
    h.set("CRVAL1", 83.6).set("CRVAL2", 22.0);
    h.set("CDELT1", -0.0005).set("CDELT2", 0.0005);
    h.set("CROTA2", 25.0);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{proj}"));
        h.set("CTYPE2", format!("DEC--{proj}"));
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", 45.0).set("CRVAL2", 30.0);
        h.set("CDELT1", -0.2).set("CDELT2", 0.2);
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
    h.set("NAXIS", 2);
    h.set("CTYPE1", "RA---CEA").set("CTYPE2", "DEC--CEA");
    h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
    h.set("CRVAL1", 45.0).set("CRVAL2", 30.0);
    h.set("CDELT1", -0.05).set("CDELT2", 0.05);
    h.set("PV2_1", 0.5);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{}", c.proj));
        h.set("CTYPE2", format!("DEC--{}", c.proj));
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", 45.0).set("CRVAL2", c.cv2);
        h.set("CDELT1", -c.cd).set("CDELT2", c.cd);
        for &(m, v) in c.pv {
            h.set(&format!("PV2_{m}"), v);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{projection}"));
        h.set("CTYPE2", format!("DEC--{projection}"));
        for &(m, value) in parameters {
            h.set(&format!("PV2_{m}"), value);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{projection}"));
        h.set("CTYPE2", format!("DEC--{projection}"));
        for &(m, value) in parameters {
            h.set(&format!("PV2_{m}"), value);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{code}"));
        h.set("CTYPE2", format!("DEC--{code}"));
        h.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
        h.set("CRVAL1", 10.0).set("CRVAL2", 20.0);
        h.set("CDELT1", 2.0).set("CDELT2", 3.0);
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
    h.set("NAXIS", 2);
    h.set("CTYPE1", "RA---TAN").set("CTYPE2", "DEC--SIN");
    h.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
    h.set("CRVAL1", 10.0).set("CRVAL2", 20.0);
    h.set("CDELT1", 1.0).set("CDELT2", 1.0);
    assert!(matches!(
        Wcs::from_header(&h, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));

    // A galactic-frame pair sharing TAN builds fine — exercises the one shared
    // classifier on the non-RA/DEC longitude/latitude forms (`GLON`/`GLAT`).
    let mut g = Header::new();
    g.set("NAXIS", 2);
    g.set("CTYPE1", "GLON-TAN").set("CTYPE2", "GLAT-TAN");
    g.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
    g.set("CRVAL1", 30.0).set("CRVAL2", 10.0);
    g.set("CDELT1", -1.0).set("CDELT2", 1.0);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{code}"));
        h.set("CTYPE2", format!("DEC--{code}"));
        h.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
        h.set("CRVAL1", 10.0).set("CRVAL2", 20.0);
        h.set("CDELT1", 2.0).set("CDELT2", 3.0);
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
    ok.set("NAXIS", 2);
    ok.set("CTYPE1", "RA---COP").set("CTYPE2", "DEC--COP");
    ok.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
    ok.set("CRVAL1", 10.0).set("CRVAL2", 45.0);
    ok.set("CDELT1", 0.5).set("CDELT2", 0.5).set("PV2_1", 45.0);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{proj}"));
        h.set("CTYPE2", format!("DEC--{proj}"));
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", 45.0).set("CRVAL2", 0.0);
        h.set("CDELT1", -0.5).set("CDELT2", 0.5);
        if let Some(v) = pv1 {
            h.set("PV2_1", v);
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
        h.set("NAXIS", 2)
            .set("CTYPE1", "RA---TAN")
            .set("CTYPE2", "DEC--TAN")
            .set("CRPIX1", 1.0)
            .set("CRPIX2", 1.0)
            .set("CRVAL1", 0.0)
            .set("CRVAL2", 0.0)
            .set("CDELT1", 1.0)
            .set("CDELT2", 1.0);
        h
    };
    // PC and CD are mutually exclusive; CROTA must not accompany PC.
    let mut pc_cd = base();
    pc_cd.set("PC1_1", 1.0).set("CD1_1", 1.0);
    assert!(matches!(
        Wcs::from_header(&pc_cd, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
    let mut crota_pc = base();
    crota_pc.set("PC1_1", 1.0).set("CROTA2", 30.0);
    assert!(matches!(
        Wcs::from_header(&crota_pc, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
    let mut crota_cd = base();
    crota_cd
        .set("CDELT1", 11.0)
        .set("CDELT2", 13.0)
        .set("CD1_1", 2.0)
        .set("CD2_2", 3.0)
        .set("CROTA2", 30.0);
    let w = Wcs::from_header(&crota_cd, None).unwrap();
    assert_eq!(w.matrix, [2.0, 0.0, 0.0, 3.0]);
    // A single convention (CD alone) is accepted.
    let mut cd_only = base();
    cd_only.set("CD1_1", 1.0).set("CD2_2", 1.0);
    assert!(Wcs::from_header(&cd_only, None).is_ok());

    let mut malformed = base();
    malformed.set("CRVAL1", "not numeric");
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

#[test]
fn zpn_extended_horner_evaluates_value_and_derivative() {
    let mut pv = [0.0; 21];
    pv[..4].copy_from_slice(&[2.0, -3.0, 4.0, 5.0]);
    let evaluation = evaluate_zpn(2.0, &pv);
    // P(2) = 2 - 3·2 + 4·2² + 5·2³ = 52; P'(2) = -3 + 8·2 + 15·2² = 73.
    assert_eq!(evaluation.value, 52.0);
    assert_eq!(evaluation.derivative, 73.0);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{proj}"));
        h.set("CTYPE2", format!("DEC--{proj}"));
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", cv1).set("CRVAL2", cv2);
        h.set("CDELT1", -0.05).set("CDELT2", 0.05);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", "RA---TAN").set("CTYPE2", "DEC--TAN");
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", 150.0 * scale).set("CRVAL2", 30.0 * scale);
        h.set("CDELT1", -5e-4 * scale).set("CDELT2", 5e-4 * scale);
        if let Some(u) = unit {
            h.set("CUNIT1", u).set("CUNIT2", u);
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", t1).set("CTYPE2", t2);
        h.set("CRPIX1", 64.0).set("CRPIX2", 64.0);
        h.set("CRVAL1", 10.0).set("CRVAL2", -20.0);
        h.set("CDELT1", -1e-3).set("CDELT2", 1e-3);
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
        h.set("NAXIS", 3);
        h.set("CTYPE1", "RA---TAN")
            .set("CTYPE2", "DEC--TAN")
            .set("CTYPE3", t3);
        h.set("CRPIX1", 1.0).set("CRPIX2", 1.0).set("CRPIX3", 1.0);
        h.set("CRVAL1", 45.0)
            .set("CRVAL2", 30.0)
            .set("CRVAL3", 1.4e9);
        h.set("CDELT1", -1e-3)
            .set("CDELT2", 1e-3)
            .set("CDELT3", 1e6);
        Wcs::from_header(&h, None).unwrap()
    };
    let cases = [
        ("FREQ", false),
        ("FREQ-LOG", true),
        ("FREQ-TAB", true),
        ("TIME", false),
        ("TIME-LOG", true),
        ("TIME-TAB", true),
        ("ABCD", false),
        ("ABCD-LOG", true),
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
            assert_eq!(out[2], 1.402e9, "{ctype}");
            assert!((out[0] - 45.0).abs() < 1e-9 && (out[1] - 30.0).abs() < 1e-9);
        }
    }
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
    tab.set("TCTYP2", "RA---TAN").set("TCTYP3", "DEC--TAN");
    tab.set("TCRPX2", 256.0).set("TCRPX3", 256.0);
    tab.set("TCRVL2", 150.0).set("TCRVL3", 30.0);
    tab.set("TCDLT2", -1e-3).set("TCDLT3", 1e-3);
    tab.set("TPC2_2", 1.0).set("TPC2_3", -0.05);
    tab.set("TPC3_2", 0.05).set("TPC3_3", 1.0);
    let wt = Wcs::from_pixel_list(&tab, &[2, 3], None).unwrap();

    let mut img = Header::new();
    img.set("NAXIS", 2);
    img.set("CTYPE1", "RA---TAN").set("CTYPE2", "DEC--TAN");
    img.set("CRPIX1", 256.0).set("CRPIX2", 256.0);
    img.set("CRVAL1", 150.0).set("CRVAL2", 30.0);
    img.set("CDELT1", -1e-3).set("CDELT2", 1e-3);
    img.set("PC1_1", 1.0).set("PC1_2", -0.05);
    img.set("PC2_1", 0.05).set("PC2_2", 1.0);
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
        .set("TCTY2A", "RA---TAN")
        .set("TCTY3A", "DEC--TAN");
    alternate.set("TCRP2A", 256.0).set("TCRP3A", 256.0);
    alternate.set("TCRV2A", 150.0).set("TCRV3A", 2.5);
    alternate
        .set("TCDE2A", -0.0002777778)
        .set("TCDE3A", 0.0002777778);
    alternate.set("TCUN2A", "deg").set("TCUN3A", "deg");
    alternate
        .set("TP2_2A", 0.96592582628907)
        .set("TP2_3A", -0.25881904510252)
        .set("TP3_2A", 0.25881904510252)
        .set("TP3_3A", 0.96592582628907);
    alternate.set("LONP2A", 180.0).set("LATP2A", 2.5);
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

    tab.set("TCRVL2", "not numeric");
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
    tab.set("1CTYP5", "RA---TAN").set("2CTYP5", "DEC--TAN");
    tab.set("1CRPX5", 256.0).set("2CRPX5", 256.0);
    tab.set("1CRVL5", 150.0).set("2CRVL5", 30.0);
    tab.set("1CDLT5", -1e-3).set("2CDLT5", 1e-3);
    tab.set("11PC5", 1.0).set("12PC5", -0.05);
    tab.set("21PC5", 0.05).set("22PC5", 1.0);
    let wt = Wcs::from_array_column(&tab, 5, None).unwrap();

    let mut img = Header::new();
    img.set("NAXIS", 2);
    img.set("CTYPE1", "RA---TAN").set("CTYPE2", "DEC--TAN");
    img.set("CRPIX1", 256.0).set("CRPIX2", 256.0);
    img.set("CRVAL1", 150.0).set("CRVAL2", 30.0);
    img.set("CDELT1", -1e-3).set("CDELT2", 1e-3);
    img.set("PC1_1", 1.0).set("PC1_2", -0.05);
    img.set("PC2_1", 0.05).set("PC2_2", 1.0);
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
        .set("1CTY5A", "RA---TAN")
        .set("2CTY5A", "DEC--TAN");
    alternate.set("1CRP5A", 256.0).set("2CRP5A", 256.0);
    alternate.set("1CRV5A", 150.0).set("2CRV5A", 2.5);
    alternate
        .set("1CDE5A", -0.0002777778)
        .set("2CDE5A", 0.0002777778);
    alternate.set("1CUN5A", "deg").set("2CUN5A", "deg");
    alternate
        .set("11PC5A", 0.96592582628907)
        .set("12PC5A", -0.25881904510252)
        .set("21PC5A", 0.25881904510252)
        .set("22PC5A", 0.96592582628907);
    alternate.set("LONP5A", 180.0).set("LATP5A", 2.5);
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

    alternate.set("WCAX5A", 2);
    let explicit = Wcs::from_array_column(&alternate, 5, Some('A')).unwrap();
    assert_astropy_golden(&explicit, TAN_GOLDEN, "ranked alternate vector-cell TAN");

    tab.set("1CRVL5", "not numeric");
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
            header.set("TCTY2A", "RA---CEA").set("TCTY3A", "DEC--CEA");
            header.set("TCRP2A", 50.0).set("TCRP3A", 50.0);
            header.set("TCRV2A", 45.0).set("TCRV3A", 30.0);
            header.set("TCDE2A", -0.05).set("TCDE3A", 0.05);
        } else {
            header.set("TCTYP2", "RA---CEA").set("TCTYP3", "DEC--CEA");
            header.set("TCRPX2", 50.0).set("TCRPX3", 50.0);
            header.set("TCRVL2", 45.0).set("TCRVL3", 30.0);
            header.set("TCDLT2", -0.05).set("TCDLT3", 0.05);
        }
        header.set(parameter, 0.5);
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
            header.set("WCAX5A", 2);
            header.set("1CTY5A", "RA---CEA").set("2CTY5A", "DEC--CEA");
            header.set("1CRP5A", 50.0).set("2CRP5A", 50.0);
            header.set("1CRV5A", 45.0).set("2CRV5A", 30.0);
            header.set("1CDE5A", -0.05).set("2CDE5A", 0.05);
        } else {
            header.set("WCAX5", 2);
            header.set("1CTYP5", "RA---CEA").set("2CTYP5", "DEC--CEA");
            header.set("1CRPX5", 50.0).set("2CRPX5", 50.0);
            header.set("1CRVL5", 45.0).set("2CRVL5", 30.0);
            header.set("1CDLT5", -0.05).set("2CDLT5", 0.05);
        }
        header.set(parameter, 0.5);
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
            header.set("TCTY2A", "LINEAR").set("TCTY3A", "LINEAR");
        } else {
            header.set("TCTYP2", "LINEAR").set("TCTYP3", "LINEAR");
        }
        header
            .set(&format!("{root}2_2{suffix}"), expected[0])
            .set(&format!("{root}2_3{suffix}"), expected[1])
            .set(&format!("{root}3_2{suffix}"), expected[2])
            .set(&format!("{root}3_3{suffix}"), expected[3]);
        let wcs = Wcs::from_pixel_list(&header, &[2, 3], alternate.then_some('A')).unwrap();
        assert_eq!(wcs.matrix, expected, "{root}, alternate={alternate}");
    }

    for (root, alternate) in [("PC", false), ("CD", false), ("PC", true), ("CD", true)] {
        let suffix = if alternate { "A" } else { "" };
        let mut header = Header::new();
        header.set(&format!("WCAX5{suffix}"), 2);
        header
            .set(&format!("11{root}5{suffix}"), expected[0])
            .set(&format!("12{root}5{suffix}"), expected[1])
            .set(&format!("21{root}5{suffix}"), expected[2])
            .set(&format!("22{root}5{suffix}"), expected[3]);
        let wcs = Wcs::from_array_column(&header, 5, alternate.then_some('A')).unwrap();
        assert_eq!(wcs.matrix, expected, "{root}, alternate={alternate}");
    }
}

#[test]
fn primary_table_wcs_rotation_matches_astropy() {
    let mut pixel = Header::new();
    pixel.set("TCTYP2", "RA---TAN").set("TCTYP3", "DEC--TAN");
    pixel.set("TCUNI2", "deg").set("TCUNI3", "deg");
    pixel.set("TCRPX2", 128.0).set("TCRPX3", 128.0);
    pixel.set("TCRVL2", 83.6).set("TCRVL3", 22.0);
    pixel.set("TCDLT2", -0.0005).set("TCDLT3", 0.0005);
    pixel.set("TCROT3", 25.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], None).unwrap();
    assert_astropy_golden(&pixel_wcs, CROTA_GOLDEN, "primary pixel-list CROTA");

    let mut vector = Header::new();
    vector.set("WCAX5", 2);
    vector.set("1CTYP5", "RA---TAN").set("2CTYP5", "DEC--TAN");
    vector.set("1CUNI5", "deg").set("2CUNI5", "deg");
    vector.set("1CRPX5", 128.0).set("2CRPX5", 128.0);
    vector.set("1CRVL5", 83.6).set("2CRVL5", 22.0);
    vector.set("1CDLT5", -0.0005).set("2CDLT5", 0.0005);
    vector.set("2CROT5", 25.0);
    let vector_wcs = Wcs::from_array_column(&vector, 5, None).unwrap();
    assert_astropy_golden(&vector_wcs, CROTA_GOLDEN, "primary vector-cell CROTA");
}

#[test]
fn table_wcs_column_poles_match_the_equivalent_image_wcs() {
    let mut image = Header::new();
    image.set("NAXIS", 2);
    image.set("CTYPE1", "RA---CEA").set("CTYPE2", "DEC--CEA");
    image.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
    image.set("CRVAL1", 45.0).set("CRVAL2", 30.0);
    image.set("CDELT1", -0.05).set("CDELT2", 0.05);
    image.set("PV2_1", 0.5);
    image.set("LONPOLE", 0.0).set("LATPOLE", -90.0);
    let image_wcs = Wcs::from_header(&image, None).unwrap();
    let image_pole = image_wcs.celestial.as_ref().unwrap().pole;
    assert_eq!(image_pole.ra, 45.0);
    assert!((image_pole.dec + 60.0).abs() < 1e-12, "{image_pole:?}");
    assert_eq!(image_pole.lonpole, 0.0);

    let mut pixel = Header::new();
    pixel.set("TCTY2A", "RA---CEA").set("TCTY3A", "DEC--CEA");
    pixel.set("TCRP2A", 50.0).set("TCRP3A", 50.0);
    pixel.set("TCRV2A", 45.0).set("TCRV3A", 30.0);
    pixel.set("TCDE2A", -0.05).set("TCDE3A", 0.05);
    pixel.set("TV3_1A", 0.5);
    pixel.set("LONP2A", 0.0).set("LATP2A", -90.0);
    let pixel_wcs = Wcs::from_pixel_list(&pixel, &[2, 3], Some('A')).unwrap();

    let mut vector = Header::new();
    vector.set("WCAX5A", 2);
    vector.set("1CTY5A", "RA---CEA").set("2CTY5A", "DEC--CEA");
    vector.set("1CRP5A", 50.0).set("2CRP5A", 50.0);
    vector.set("1CRV5A", 45.0).set("2CRV5A", 30.0);
    vector.set("1CDE5A", -0.05).set("2CDE5A", 0.05);
    vector.set("2V5_1A", 0.5);
    vector.set("LONP5A", 0.0).set("LATP5A", -90.0);
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
        h.set("NAXIS", 2).set(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set("NAXIS", 2)
        .set("CD1_1", 1.0)
        .set("CD2_2", 1.0)
        .set("CD3_3", 1.0)
        .set("CD4_4", 1.0);
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
        h.set(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set("11CD5", 1.0).set("22CD5", 1.0).set("33CD5", 1.0);
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
        .set("11CD5A", 1.0)
        .set("22CD5A", 1.0)
        .set("33CD5A", 1.0);
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
}

#[test]
fn rejects_absurd_wcsaxes() {
    // Axis counts are untrusted; reject both bounds before they size a matrix or
    // drive the per-axis loops.
    let mut h = Header::new();
    for value in [-1, 0, 1000] {
        h.set("WCSAXES", value);
        assert!(matches!(
            Wcs::from_header(&h, None),
            Err(FitsError::KeywordOutOfRange { name: "WCSAXES" })
        ));
    }

    h.set("WCAX5", -1);
    assert!(matches!(
        Wcs::from_array_column(&h, 5, None),
        Err(FitsError::KeywordOutOfRange { name: "WCAXn" })
    ));
}
