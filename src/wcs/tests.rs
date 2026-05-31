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
    // Zenithal pole reduces to (CRVAL, LONPOLE=180).
    let c = w.celestial.expect("celestial");
    assert_eq!(c.pole, (150.0, 2.5, 180.0));
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
    let golden: &[(f64, f64, f64, f64)] = &[
        (128.0, 128.0, 83.6000000000, 22.0000000000),
        (1.0, 1.0, 83.6909943156, 21.9692606492),
        (256.0, 200.0, 83.5210288338, 22.0055606050),
        (64.0, 192.0, 83.6166986376, 22.0425247793),
    ];
    for &(px, py, ra, dec) in golden {
        let out = w.pixel_to_world(&[px, py]);
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
        let out = w.pixel_to_world(&[px, py]);
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
    let golden: &[(f64, f64, f64, f64)] = &[
        (20.0, 70.0, 46.7406870828, 30.4886140110),
        (80.0, 30.0, 43.2767613377, 29.4887155113),
    ];
    for &(px, py, ra, dec) in golden {
        let out = w.pixel_to_world(&[px, py]);
        assert!(
            (out[0] - ra).abs() < 1e-8 && (out[1] - dec).abs() < 1e-8,
            "CEA λ at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
    }
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
            let out = w.pixel_to_world(&[px, py]);
            assert!(
                (out[0] - ra).abs() < 1e-7 && (out[1] - dec).abs() < 1e-7,
                "{} at ({px},{py}): got {out:?}, want ({ra},{dec})",
                c.proj
            );
        }
    }
}

#[test]
fn unimplemented_projection_codes_error_cleanly() {
    use crate::error::FitsError;
    use crate::header::Header;
    // The quad-cube and HEALPix codes are recognized but unimplemented; a celestial
    // axis using one must error rather than silently fall back to linear coords.
    for code in ["TSC", "CSC", "QSC", "HPX", "XPH"] {
        let mut h = Header::new();
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{code}"));
        h.set("CTYPE2", format!("DEC--{code}"));
        h.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
        h.set("CRVAL1", 0.0).set("CRVAL2", 0.0);
        h.set("CDELT1", 1.0).set("CDELT2", 1.0);
        assert!(
            matches!(
                Wcs::from_header(&h, None),
                Err(FitsError::UnsupportedProjection { .. })
            ),
            "{code} should be rejected"
        );
    }
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
        (Cea, &[]),
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
    ];
    for &(proj, pv) in cases {
        // Positive native latitudes, away from the poles: in-domain for every
        // family (zenithal θ > 0, conics near θ_a, perspective non-divergent).
        for &(phi, theta) in &[(30.0_f64, 70.0_f64), (-40.0, 50.0), (20.0, 55.0)] {
            let (x, y) = proj.project(phi, theta, pv);
            let (p2, t2) = proj.deproject(x, y, pv);
            assert!(
                norm180(p2 - phi).abs() < 1e-7 && (t2 - theta).abs() < 1e-7,
                "{proj:?}: ({phi},{theta}) → ({x},{y}) → ({p2},{t2})"
            );
        }
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
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{proj}"));
        h.set("CTYPE2", format!("DEC--{proj}"));
        h.set("CRPIX1", 50.0).set("CRPIX2", 50.0);
        h.set("CRVAL1", cv1).set("CRVAL2", cv2);
        h.set("CDELT1", -0.05).set("CDELT2", 0.05);
        let w = Wcs::from_header(&h, None).unwrap();
        let out = w.pixel_to_world(&[px, py]);
        assert!(
            (out[0] - ra).abs() < 1e-8 && (out[1] - dec).abs() < 1e-8,
            "{proj} at ({px},{py}): got {out:?}, want ({ra},{dec})"
        );
        // Full round-trip.
        let back = w.world_to_pixel(&out);
        assert!(
            (back[0] - px).abs() < 1e-6 && (back[1] - py).abs() < 1e-6,
            "{proj} round-trip: {back:?}"
        );
    }
}
