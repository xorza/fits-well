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
fn unimplemented_projection_codes_fall_back_to_intermediate() {
    use crate::header::Header;
    // Quad-cube and HEALPix codes are recognized but their projection math is not
    // implemented. Rather than fail (which would also lose any other axis), the WCS
    // still builds: the axes are listed in `unsupported_axes` and pixel_to_world
    // returns their intermediate (linear-stage) world coordinate, never silently.
    for code in ["TSC", "CSC", "QSC", "HPX", "XPH"] {
        let mut h = Header::new();
        h.set("NAXIS", 2);
        h.set("CTYPE1", format!("RA---{code}"));
        h.set("CTYPE2", format!("DEC--{code}"));
        h.set("CRPIX1", 1.0).set("CRPIX2", 1.0);
        h.set("CRVAL1", 10.0).set("CRVAL2", 20.0);
        h.set("CDELT1", 2.0).set("CDELT2", 3.0);
        let w = Wcs::from_header(&h, None).unwrap();
        assert_eq!(w.unsupported_axes, vec![0, 1], "{code} axes flagged");
        assert!(w.celestial.is_none(), "{code} not decoded as a projection");
        // Intermediate world at pixel (3,4): CRVAL + CDELT·(pixel − CRPIX).
        assert_eq!(w.pixel_to_world(&[3.0, 4.0]), vec![14.0, 29.0], "{code}");
    }
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
    // §8: PC, CD, and CROTA are mutually exclusive.
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
    // A single convention (CD alone) is accepted.
    let mut cd_only = base();
    cd_only.set("CD1_1", 1.0).set("CD2_2", 1.0);
    assert!(Wcs::from_header(&cd_only, None).is_ok());
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
        let a = w_deg.pixel_to_world(&[px, py]);
        let b = w_asec.pixel_to_world(&[px, py]);
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "deg {a:?} vs arcsec {b:?} at ({px},{py})"
        );
    }
    // The reference pixel maps exactly to CRVAL = (150°, 30°) — proving the arcsec
    // CRVAL was scaled to degrees, not taken literally.
    let r = w_asec.pixel_to_world(&[50.0, 50.0]);
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
        let a = radec.pixel_to_world(&[px, py]);
        let b = helio.pixel_to_world(&[px, py]);
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "RA/DEC {a:?} vs HPLN/HPLT {b:?}"
        );
    }
}

#[test]
fn linear_spectral_resolves_nonlinear_falls_back_to_intermediate() {
    use crate::header::Header;
    // §8.4: a bare spectral type (`FREQ`) is linearly sampled and fully resolves.
    // A non-linear algorithm code (`-LOG`) is not evaluated, so that axis is flagged
    // and returns its intermediate (linear-stage) value — while the celestial pair
    // on the same cube still decodes fully (the whole WCS is no longer lost).
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
    // Bare FREQ: fully linear, nothing flagged. At pixel 3: 1.4e9 + 2·1e6 = 1.402e9.
    let lin = build("FREQ");
    assert!(lin.unsupported_axes.is_empty());
    assert!((lin.pixel_to_world(&[1.0, 1.0, 3.0])[2] - 1.402e9).abs() < 1.0);
    // FREQ-LOG: axis index 2 flagged; it returns the intermediate value, and the
    // RA/DEC pair still decodes (reference pixel → CRVAL exactly).
    let log = build("FREQ-LOG");
    assert_eq!(log.unsupported_axes, vec![2]);
    assert!((log.pixel_to_world(&[1.0, 1.0, 3.0])[2] - 1.402e9).abs() < 1.0);
    let r = log.pixel_to_world(&[1.0, 1.0, 1.0]);
    assert!(
        (r[0] - 45.0).abs() < 1e-9 && (r[1] - 30.0).abs() < 1e-9,
        "{r:?}"
    );
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
        let a = wt.pixel_to_world(&[px, py]);
        let b = wi.pixel_to_world(&[px, py]);
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "pixel-list {a:?} vs image {b:?} at ({px},{py})"
        );
    }
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

    assert_eq!(wt.naxis, 2); // rank inferred from the iCTYP5 keywords
    assert!(wt.celestial.is_some(), "vector-cell pair must be celestial");
    for &(px, py) in &[(256.0, 256.0), (1.0, 1.0), (300.0, 100.0), (50.0, 400.0)] {
        let a = wt.pixel_to_world(&[px, py]);
        let b = wi.pixel_to_world(&[px, py]);
        assert!(
            (a[0] - b[0]).abs() < 1e-12 && (a[1] - b[1]).abs() < 1e-12,
            "vector-cell {a:?} vs image {b:?} at ({px},{py})"
        );
    }
}
