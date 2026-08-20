use crate::error::FitsError;
use crate::header::Header;
use crate::wcs::R2D;
use crate::wcs::Wcs;
use crate::wcs::internals::CEA_GOLDEN;
use crate::wcs::internals::assert_astropy_golden;
use crate::wcs::norm180;
use crate::wcs::projection::Projection;
use crate::wcs::projection::evaluate_zpn;
use std::f64::consts::SQRT_2;

#[test]
fn zpn_extended_horner_evaluates_value_and_derivative() {
    let mut pv = [0.0; 21];
    pv[..4].copy_from_slice(&[2.0, -3.0, 4.0, 5.0]);
    let evaluation = evaluate_zpn(2.0, &pv);
    // P(2) = 2 - 3·2 + 4·2² + 5·2³ = 52; P'(2) = -3 + 8·2 + 15·2² = 73.
    assert_eq!(evaluation.value, 52.0);
    assert_eq!(evaluation.derivative, 73.0);
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
fn allsky_projections_match_astropy() {
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

fn projection_parameters(parameters: &[f64]) -> [f64; 21] {
    let mut pv = [0.0; 21];
    pv[..parameters.len()].copy_from_slice(parameters);
    pv
}
