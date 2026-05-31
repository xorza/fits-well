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

#[test]
fn frame_transforms_match_astropy() {
    use super::frame::Frame;
    // Crab nebula in ICRS.
    let (ra, dec) = (83.633212, 22.014460);

    // ICRS → FK5(J2000): the IAU-2000 frame bias (~25 mas), matched to astropy.
    let (r, d) = Frame::Icrs
        .transform(ra, dec, Frame::Fk5 { equinox: 2000.0 })
        .unwrap();
    assert!((r - 83.6332196247).abs() < 1e-8 && (d - 22.0144547866).abs() < 1e-8);

    // ICRS → FK5(J1975): frame bias + IAU-1976 precession. Golden is bias·pmat76
    // (our precession matrix is bit-identical to `erfa.pmat76`). astropy uses the
    // IAU-2006 model for FK5 and so differs by ~68 mas over these 25 years.
    let (r, d) = Frame::Icrs
        .transform(ra, dec, Frame::Fk5 { equinox: 1975.0 })
        .unwrap();
    assert!(
        (r - 83.2570463443).abs() < 1e-8 && (d - 21.9985649535).abs() < 1e-8,
        "FK5(J1975): got ({r},{d})"
    );

    // ICRS → Galactic: the exact astropy matrix.
    let (l, b) = Frame::Icrs.transform(ra, dec, Frame::Galactic).unwrap();
    assert!(
        (l - 184.5575560202).abs() < 1e-8 && (b - (-5.7842773615)).abs() < 1e-8,
        "Galactic: got ({l},{b})"
    );

    // ICRS → FK4(B1950): precession + E-terms. astropy golden (~few mas accuracy).
    let (r, d) = Frame::Icrs
        .transform(ra, dec, Frame::Fk4 { equinox: 1950.0 })
        .unwrap();
    assert!(
        (r - 82.8809530677).abs() < 1e-4 && (d - 21.9817695456).abs() < 1e-4,
        "FK4(B1950): got ({r},{d})"
    );
    // FK4 at a non-B1950 equinox needs Newcomb pre-precession — not yet supported.
    assert!(matches!(
        Frame::Icrs.transform(ra, dec, Frame::Fk4 { equinox: 1975.0 }),
        Err(crate::error::FitsError::UnsupportedFrame)
    ));
}

#[test]
fn frame_round_trips() {
    use super::frame::Frame;
    let (ra, dec) = (200.0, -45.0);
    for to in [Frame::Fk5 { equinox: 1975.0 }, Frame::Galactic] {
        let (x, y) = Frame::Icrs.transform(ra, dec, to).unwrap();
        let (r, d) = to.transform(x, y, Frame::Icrs).unwrap();
        assert!((r - ra).abs() < 1e-9 && (d - dec).abs() < 1e-9, "{to:?}");
    }
}

#[test]
fn frame_parses_from_header() {
    use super::frame::Frame;
    use crate::header::Header;
    let mut h = Header::new();
    h.set("RADESYS", "FK5").set("EQUINOX", 2000.0);
    assert_eq!(Frame::from_header(&h, None), Frame::Fk5 { equinox: 2000.0 });
    // Legacy: EQUINOX < 1984 with no RADESYS ⇒ FK4.
    let mut h2 = Header::new();
    h2.set("EQUINOX", 1950.0);
    assert_eq!(
        Frame::from_header(&h2, None),
        Frame::Fk4 { equinox: 1950.0 }
    );
    // Nothing ⇒ ICRS.
    assert_eq!(Frame::from_header(&Header::new(), None), Frame::Icrs);
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

/// Every projection's deprojection inverts its forward projection.
#[test]
fn projections_round_trip() {
    use Projection::*;
    for proj in [Tan, Sin, Arc, Stg, Zea, Car, Cea, Mer, Sfl, Ait, Mol] {
        // Positive native latitudes, away from the poles: in-domain for both the
        // zenithal (θ > 0 only) and cylindrical families.
        for &(phi, theta) in &[(30.0_f64, 70.0_f64), (-60.0, 40.0), (20.0, 25.0)] {
            let (x, y) = proj.project(phi, theta);
            let (p2, t2) = proj.deproject(x, y);
            assert!(
                norm180(p2 - phi).abs() < 1e-9 && (t2 - theta).abs() < 1e-9,
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
