use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::error::WcsDefect;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::image_model::region::Region;
use crate::internals::cards::{assert_absent, assert_cards};
use crate::internals::fixture::Fixture;
use crate::internals::tolerance::{Tolerance, assert_close};
use crate::internals::wcs_header::{Family, WcsHeader};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::angle;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::axis::Axis;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::internals::TAN_GOLDEN;
use crate::world_coordinates::internals::assert_astropy_golden;

use crate::world_coordinates::internals::{to_pixel, to_world};
use crate::world_coordinates::projection::Projection;

/// Load the WCS from the primary header of a fixture.
#[test]
fn parses_tan_header() {
    let w = Fixture::WcsTan.wcs();
    assert_eq!(
        w.metadata().axes,
        [
            Axis {
                ctype: "RA---TAN".to_string(),
                cunit: "deg".to_string(),
                crval: 150.0,
                crpix: 256.0,
                spectral_frame: None,
            },
            Axis {
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
        c.rotation.pole,
        CelestialPole {
            ra: 150.0,
            dec: 2.5,
            lonpole: 180.0
        }
    );
}

#[test]
fn pixel_to_world_matches_astropy() {
    let w = Fixture::WcsTan.wcs();
    assert_astropy_golden(&w, &TAN_GOLDEN, "TAN image");
}

/// A world coordinate holds α and δ in degrees, each rounded once, so a pixel → world
/// → pixel round trip misses by about that rounding: an ulp of α, scaled by cos δ on the
/// sky, and an ulp of δ, with as much again for the rotation and the projection, which
/// each round too. The sweep covers each
/// zenithal projection at the IRAC header's reference point, at 60°, and next to the
/// celestial pole, at 1.2″ and at 3.6 mas a pixel, out to 20° or 1e5 pixels: there the
/// latitude once came from `asin` of a value next to 1, which lost every offset under a
/// pixel at 3.6 mas.
#[test]
fn world_to_pixel_inverts_pixel_to_world() {
    let ulp = |value: f64| value.abs().next_up() - value.abs();
    let assert_round_trip = |wcs: &Wcs, pixel: [f64; 2], degrees_per_pixel: f64, context: &str| {
        let world = to_world(wcs, &pixel).unwrap();
        let back = to_pixel(wcs, &world).unwrap();
        let rounding = ulp(world[0]) * world[1].to_radians().cos() + ulp(world[1]);
        let miss = (back[0] - pixel[0]).hypot(back[1] - pixel[1]) * degrees_per_pixel;
        assert!(
            miss <= 2.0 * rounding,
            "{context} at {pixel:?}: {back:?} misses by {miss:e}°, the rounding is {rounding:e}°"
        );
    };
    let tan = Fixture::WcsTan.wcs();
    for &(px, py, _, _) in TAN_GOLDEN.points {
        assert_round_trip(&tan, [px, py], 1.0 / 3600.0, "TAN fixture");
    }
    for projection in ["TAN", "SIN", "ARC", "STG", "ZEA"] {
        for crval in [
            [6.155_013_476_190_52, -2.072_307_988_889_38],
            [150.0, 60.0],
            [10.0, 89.95],
        ] {
            for degrees_per_pixel in [3.4e-4, 1e-6] {
                let wcs = WcsHeader::celestial(
                    projection,
                    [0.0, 0.0],
                    crval,
                    [-degrees_per_pixel, degrees_per_pixel],
                )
                .image_header();
                let wcs = Wcs::from_header(&wcs, None).unwrap();
                for offset in [0.5, 37.0, 1e3, 1e5] {
                    if offset * degrees_per_pixel > 20.0 {
                        continue;
                    }
                    for [dx, dy] in [[1.0, 0.0], [0.0, 1.0], [0.7, -0.3], [-1.0, 1.0]] {
                        assert_round_trip(
                            &wcs,
                            [dx * offset, dy * offset],
                            degrees_per_pixel,
                            &format!("{projection} at {crval:?}, {degrees_per_pixel}°/px"),
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn transform_failures_return_errors() {
    let build = |projection: &str| {
        let header =
            WcsHeader::celestial(projection, [1.0, 1.0], [0.0, 0.0], [100.0, 100.0]).image_header();
        Wcs::from_header(&header, None)
    };

    let sin = build("SIN").unwrap();
    assert!(matches!(
        to_world(&sin, &[2.0, 1.0]),
        Err(FitsError::WcsPixelOutOfDomain { projection: "SIN" })
    ));

    // A ZPN polynomial without coefficients maps every colatitude to the pole.
    assert!(matches!(
        build("ZPN"),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: "ZPN" }
        })
    ));

    let tan = Fixture::WcsTan.wcs();
    assert!(matches!(
        to_pixel(&tan, &[150.0, 100.0]),
        Err(FitsError::WcsWorldOutOfDomain { projection: "TAN" })
    ));
    for result in [
        to_world(&tan, &[1.0]),
        to_world(&tan, &[1.0, 2.0, 3.0]),
        to_pixel(&tan, &[150.0]),
        to_pixel(&tan, &[150.0, 2.5, 1.0]),
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
    let mut header =
        WcsHeader::celestial("TAN", [1.0, 1.0], [45.0, 30.0], [-0.1, 0.1]).image_header();
    header
        .set_internal("CUNIT1", "deg")
        .set_internal("CUNIT2", "deg");
    let wcs = Wcs::from_header(&header, None).unwrap();
    let view = wcs.metadata();
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
        h.set_internal(keywords::NAXIS, 2);
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
    for pixel in [[1.0, 1.0], [64.0, 64.0], [100.0, 30.0]] {
        assert_eq!(
            to_world(&helio, &pixel).unwrap(),
            to_world(&radec, &pixel).unwrap(),
            "{pixel:?}"
        );
    }
}

#[test]
fn absent_wcsaxes_uses_the_largest_wcs_index() {
    let build = |keyword: &str, value: Value| {
        let mut h = Header::new();
        h.set_internal(keywords::NAXIS, 2)
            .set_internal(keyword, value);
        h
    };
    let mut cd = Header::new();
    cd.set_internal(keywords::NAXIS, 2)
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
        assert_eq!(Wcs::from_header(h, None).unwrap().metadata().axes.len(), 4);
    }

    for alternate in [
        build("CTYPE4A", Value::Text("LINEAR".to_string())),
        build("PV4_0A", Value::Real(1.0)),
        build("PC4_4A", Value::Real(1.0)),
    ] {
        assert_eq!(
            Wcs::from_header(&alternate, None)
                .unwrap()
                .metadata()
                .axes
                .len(),
            2
        );
        assert_eq!(
            Wcs::from_header(&alternate, 'A')
                .unwrap()
                .metadata()
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
        h.set_internal(keywords::WCSAXES, value);
        assert!(matches!(
            Wcs::from_header(&h, None),
            Err(FitsError::KeywordOutOfRange {
                keyword: keywords::WCSAXES
            })
        ));
    }
    h.set_internal(keywords::WCSAXES, 1);
    assert_eq!(Wcs::from_header(&h, None).unwrap().metadata().axes.len(), 1);
    // The alternate argument selects `WCSAXESA` in each of its forms, and a letter
    // outside A to Z is refused before any keyword is read.
    h.set_internal("WCSAXESA", 2);
    for alt in [Some('A'), None] {
        let axes = if alt.is_some() { 2 } else { 1 };
        assert_eq!(
            Wcs::from_header(&h, alt).unwrap().metadata().axes.len(),
            axes
        );
    }
    assert_eq!(Wcs::from_header(&h, 'A').unwrap().metadata().axes.len(), 2);
    assert!(matches!(
        Wcs::from_header(&h, 'a'),
        Err(FitsError::InvalidAlternate { alternate: 'a' })
    ));
    h.set_internal(keywords::WCSAXES, 999);
    assert_eq!(Wcs::image_axis_count(&h, Alternate::PRIMARY).unwrap(), 999);

    h.set_internal("WCAX5", -1);
    assert!(matches!(
        Wcs::from_array_column(&h, 5, None),
        Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "WCAX5"
    ));
}

/// A celestial header at the default reference pixel, value and increment, then
/// `keywords`.
fn celestial_with(projection: &str, keywords: &[(&str, f64)]) -> Header {
    let mut header = WcsHeader::celestial(projection, [0.0; 2], [0.0; 2], [1.0; 2]).image_header();
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
        let header = celestial_with("CAR", &[("CRVAL2", -30.0), ("LATPOLE", latpole)]);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let dec = wcs.metadata().celestial_projection.unwrap().pole.dec;
        assert!((dec - pole).abs() < 1e-12, "LATPOLE {latpole}: {dec}");
    }
}

/// With θ₀ = 0 and φp − φ₀ = 120°, a fiducial latitude needs |δ₀| ≤ asin|cos 120°| = 30°;
/// δ₀ = 60° admits no pole at all.
#[test]
fn an_impossible_pole_is_refused() {
    let header = celestial_with("CAR", &[("CRVAL2", 60.0), ("LONPOLE", 120.0)]);
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
            .set_internal(keywords::NAXIS, 2)
            .set_internal("CTYPE1", ctypes[0])
            .set_internal("CTYPE2", ctypes[1]);
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert_eq!(wcs.metadata().unsupported_axes, unsupported, "{ctypes:?}");
        assert_eq!(
            wcs.metadata().celestial_projection.is_some(),
            unsupported.is_empty(),
            "{ctypes:?}"
        );
    }
}

/// One axis of a celestial pair evaluates the projection, as the complete transform does.
#[test]
fn axis_world_of_a_celestial_axis_is_the_projected_value() {
    let header = celestial_with("TAN", &[("CRVAL1", 150.0), ("CRVAL2", 60.0)]);
    let wcs = Wcs::from_header(&header, None).unwrap();
    let pixel = [30.0, 20.0];
    let complete = to_world(&wcs, &pixel).unwrap();
    for (axis, &value) in complete.iter().enumerate() {
        let world = wcs.axis_world(axis, &pixel).unwrap();
        assert_eq!(world.value, value, "axis {axis}");
        assert_eq!(world.cunit, "deg", "axis {axis}");
    }
    // Through SIP, which one point's offsets carry for both axes, the same bits.
    let sip = Wcs::from_header(&Fixture::WcsIracSip.header(), None).unwrap();
    let pixel = [200.0, 30.0];
    let complete = to_world(&sip, &pixel).unwrap();
    for (axis, &value) in complete.iter().enumerate() {
        assert_eq!(
            sip.axis_world(axis, &pixel).unwrap().value,
            value,
            "SIP axis {axis}"
        );
    }
}

/// Each axis says the unit its world coordinates come out in: degrees for a celestial
/// axis in radians, the Table-25 default for a spectral algorithm, and the declared unit
/// for a linear axis, spectral or not.
#[test]
fn each_axis_names_its_world_unit() {
    let mut header = celestial_with("CAR", &[]);
    header
        .set_internal("CUNIT1", "rad")
        .set_internal("CUNIT2", "rad")
        .set_internal(keywords::NAXIS, 5)
        .set_internal("CTYPE3", "FREQ-W2F")
        .set_internal("CUNIT3", "MHz")
        .set_internal("CRVAL3", 1420.0)
        .set_internal(keywords::RESTFRQ, 1.42e9)
        .set_internal("CTYPE4", "FREQ")
        .set_internal("CUNIT4", "GHz")
        .set_internal("CTYPE5", "LINEAR")
        .set_internal("CUNIT5", "furlong");
    let wcs = Wcs::from_header(&header, None).unwrap();
    let units: Vec<&str> = (0..5).map(|axis| wcs.world_unit(axis)).collect();
    assert_eq!(units, ["deg", "deg", "Hz", "GHz", "furlong"]);
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
        let header = celestial_with("CAR", &[("CRPIX1", 1.0), ("CRPIX2", 1.0)]);
        let mut header = header;
        header
            .set_internal("CUNIT1", unit)
            .set_internal("CUNIT2", unit);
        let wcs = Wcs::from_header(&header, None).unwrap();
        let world = to_world(&wcs, &[2.0, 1.0]).unwrap();
        // The value passes through the native-to-celestial rotation, whose trigonometry
        // leaves a few 1e-15 rad — order 1e-14 of a degree.
        assert!(
            (world[0] - degrees).abs() < 1e-13 * degrees.max(1.0),
            "{unit:?}: {world:?}"
        );
    }
    for unit in ["furlong", "mdeg", "karcsec", "Hz"] {
        let mut header = celestial_with("CAR", &[]);
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
    let mut header = celestial_with("TAN", &[("CRVAL1", 3600.0), ("CRVAL2", 7200.0)]);
    header
        .set_internal("CUNIT1", "arcsec")
        .set_internal("CUNIT2", "arcsec");
    let wcs = Wcs::from_header(&header, None).unwrap();
    let axes = wcs.metadata().axes;
    assert_eq!((axes[0].crval, axes[0].cunit.as_str()), (3600.0, "arcsec"));
    assert_eq!((axes[1].crval, axes[1].cunit.as_str()), (7200.0, "arcsec"));
    let world = to_world(&wcs, &[0.0, 0.0]).unwrap();
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
    assert_eq!(angle::norm360(-1e-20), 0.0);
    assert_eq!(angle::norm360(-90.0), 270.0);
    assert_eq!(angle::norm360(720.0), 0.0);
    assert_eq!(angle::norm360(359.5), 359.5);
}

/// `[−180, 180)`, exactly. One ulp below −180 is `−180 − 2⁻⁴⁵`, whose exact image is
/// `180 − 2⁻⁴⁵`; rounding `a + 180` through `rem_euclid` would instead land on 180 itself.
/// A tiny angle keeps every bit rather than vanishing into `180 − 180`.
#[test]
fn norm180_is_exact_and_stays_below_a_half_turn() {
    let ulp = 2f64.powi(-45);
    for (angle, normalized) in [
        (-180.0 - ulp, 180.0 - ulp),
        (180.0, -180.0),
        (-180.0, -180.0),
        (190.0, -170.0),
        (-190.0, 170.0),
        (540.0, -180.0),
        (1e-20, 1e-20),
        (-1e-20, -1e-20),
    ] {
        assert_eq!(angle::norm180(angle), normalized, "{angle}");
    }
}

/// `wcs` written by `write_into` into an empty header as its description `alt`, then read
/// back.
fn written_back(wcs: &Wcs, alt: Option<char>) -> Wcs {
    let mut header = Header::new();
    wcs.write_into(&mut header, alt).unwrap();
    Wcs::from_header(&header, alt).unwrap()
}

/// `b` is `a`: the same metadata, and the same world coordinate, bit for bit, or the same
/// refusal, at 25 pixels around the reference pixel.
fn assert_same_transform(a: &Wcs, b: &Wcs, what: &str) {
    let (ma, mb) = (a.metadata(), b.metadata());
    assert_eq!(ma.axes, mb.axes, "{what}");
    assert_eq!(ma.celestial_frame, mb.celestial_frame, "{what}");
    assert_eq!(ma.celestial_projection, mb.celestial_projection, "{what}");
    assert_eq!(ma.unsupported_axes, mb.unsupported_axes, "{what}");
    assert_eq!(ma.distortion, mb.distortion, "{what}");
    for dx in [-20.0, -7.5, 0.0, 3.25, 20.0] {
        for dy in [-20.0, -7.5, 0.0, 3.25, 20.0] {
            let mut pixel: Vec<f64> = ma.axes.iter().map(|axis| axis.crpix).collect();
            pixel[0] += dx;
            if pixel.len() > 1 {
                pixel[1] += dy;
            }
            assert_eq!(
                to_world(a, &pixel).ok(),
                to_world(b, &pixel).ok(),
                "{what} at {pixel:?}"
            );
        }
    }
}

/// `write_into` gives back the same transform for every projection, a spectral cube, an
/// alternate description, SIP and TPV.
#[test]
fn write_into_round_trips_every_description() {
    for projection in Projection::all() {
        let code = projection.code();
        let mut header = WcsHeader::celestial(code, [50.5, 40.25], [30.0, 40.0], [-1e-3, 2e-3])
            .pv(projection.test_parameters())
            .image_header();
        header
            .set_internal("PC1_2", 0.25)
            .set_internal("CUNIT1", "deg")
            .set_internal("RADESYS", "FK5");
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert_same_transform(&wcs, &written_back(&wcs, None), code);
    }

    let mut cube =
        WcsHeader::celestial("TAN", [10.0, 10.0], [150.0, 2.0], [1e-3, 1e-3]).image_header();
    cube.set_internal(keywords::NAXIS, 3)
        .set_internal("CTYPE3", "VRAD")
        .set_internal("CUNIT3", "km/s")
        .set_internal("CRVAL3", 12.5)
        .set_internal("CRPIX3", 3.0)
        .set_internal("CDELT3", 0.5)
        .set_internal("RESTFRQ", 1.420_405_751_8e9)
        .set_internal("SPECSYS", "LSRK");
    let wcs = Wcs::from_header(&cube, None).unwrap();
    assert_same_transform(&wcs, &written_back(&wcs, None), "spectral cube");

    let mut alternate =
        WcsHeader::celestial("TAN", [10.0, 10.0], [150.0, 2.0], [1e-3, 1e-3]).image_header();
    WcsHeader::celestial("SIN", [5.0, 6.0], [10.0, 20.0], [1.0, 1.0])
        .cd([2e-3, 1e-4, 0.0, 3e-3])
        .write_into(&mut alternate, Family::Image, 'A');
    alternate.set_internal("RADESYSA", "ICRS");
    let wcs = Wcs::from_header(&alternate, 'A').unwrap();
    assert_same_transform(&wcs, &written_back(&wcs, Some('A')), "alternate A");

    // The primary-only `RADECSYS` and `RESTFREQ` go back as `RADESYSA` and `RESTFRQA`,
    // as `RADECSYSA` and `RESTFREQA` exceed 8 bytes.
    let mut legacy =
        WcsHeader::celestial("TAN", [10.0, 10.0], [150.0, 2.0], [1e-3, 1e-3]).image_header();
    legacy
        .set_internal("RADECSYS", "FK4")
        .set_internal("RESTFREQ", 1.4e9);
    let wcs = Wcs::from_header(&legacy, None).unwrap();
    let mut header = Header::new();
    wcs.write_into(&mut header, 'A').unwrap();
    assert_cards(
        &header,
        &[("RADESYSA", "FK4".into()), ("RESTFRQA", 1.4e9.into())],
    );
    assert_same_transform(
        &wcs,
        &Wcs::from_header(&header, 'A').unwrap(),
        "legacy spellings",
    );

    for fixture in [Fixture::WcsIracSip, Fixture::WcsTpvOnly] {
        let header = fixture.header();
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert_same_transform(&wcs, &written_back(&wcs, None), &format!("{fixture:?}"));
    }
}

/// The matrix is written in wcslib's form: a `PC` + `CDELT` header gives back its own
/// values, a `CD` one `PC = CD` with `CDELT = 1`, and `CROTA2 = 30°` with
/// `CDELT = (−2e-4, 3e-4)` the `PC` it derives: `cos 30°` on the diagonal,
/// `PC1_2 = −sin 30°·3e-4/(−2e-4) = 0.75`, `PC2_1 = sin 30°·(−2e-4)/3e-4 = −1/3`. An identity
/// term is not written. A write replaces the description's records, so no `CD` stays
/// beside the `PC` written, and leaves another description's records as they were.
#[test]
fn write_into_writes_the_matrix_in_wcslibs_form() {
    let written = |header: &Header| {
        let mut out = Header::new();
        Wcs::from_header(header, None)
            .unwrap()
            .write_into(&mut out, None)
            .unwrap();
        out
    };
    let mut pc = WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [2.0, 3.0]).image_header();
    pc.set_internal("PC1_2", 0.3);
    let out = written(&pc);
    assert_cards(&out, &[("PC1_2", 0.3.into()), ("CDELT1", 2.0.into())]);
    assert_absent(&out, &["PC1_1", "PC2_1"]);

    let mut cd = WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [9.0, 9.0]).image_header();
    cd.set_internal("CD1_1", 2e-3)
        .set_internal("CD1_2", 1e-4)
        .set_internal("CD2_2", 3e-3);
    let out = written(&cd);
    assert_cards(
        &out,
        &[
            ("PC1_1", 2e-3.into()),
            ("PC1_2", 1e-4.into()),
            ("CDELT1", 1.0.into()),
        ],
    );
    assert_eq!(out.get("CD1_1").unwrap(), None);

    let mut crota =
        WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [-2e-4, 3e-4]).image_header();
    crota.set_internal("CROTA2", 30.0);
    let out = written(&crota);
    let cos30 = (30.0f64).to_radians().cos();
    for (name, value) in [
        ("PC1_1", cos30),
        ("PC1_2", 0.75),
        ("PC2_1", -1.0 / 3.0),
        ("PC2_2", cos30),
    ] {
        assert_close(
            &[out.get_real(name).unwrap().unwrap()],
            &[value],
            Tolerance::abs(
                2.0 * f64::EPSILON,
                "two roundings of a unit-scale matrix entry",
            ),
            name,
        );
    }
    assert_eq!(out.get("CROTA2").unwrap(), None);

    let mut target = WcsHeader::celestial("TAN", [7.0, 7.0], [1.0, 1.0], [1.0, 1.0]).image_header();
    target
        .set_internal("CD1_1", 5.0)
        .set_internal("CTYPE1A", "RA---ARC")
        .set_internal("CRVAL1A", 4.5);
    Wcs::from_header(&pc, None)
        .unwrap()
        .write_into(&mut target, None)
        .unwrap();
    assert_eq!(target.get("CD1_1").unwrap(), None);
    assert_cards(
        &target,
        &[
            ("CRPIX1", 1.0.into()),
            ("CTYPE1A", "RA---ARC".into()),
            ("CRVAL1A", 4.5.into()),
        ],
    );
}

/// A `-TAB` axis cannot be written, as its arrays live in another HDU, nor SIP as an
/// alternate description.
#[test]
fn write_into_refuses_what_a_header_cannot_hold() {
    let mut tab = WcsHeader::celestial("TAN", [1.0, 1.0], [0.0, 0.0], [1.0, 1.0]).image_header();
    tab.set_internal(keywords::NAXIS, 3)
        .set_internal("CTYPE3", "WAVE-TAB");
    let wcs = Wcs::from_header(&tab, None).unwrap();
    assert!(matches!(
        wcs.write_into(&mut Header::new(), None),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [2]
    ));
    let sip = Wcs::from_header(&Fixture::WcsIracSip.header(), None).unwrap();
    assert!(matches!(
        sip.write_into(&mut Header::new(), 'A'),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
}

/// A SIP header with a rotated, sheared `PC`, for the crop and bin tests.
fn rotated_sip_header() -> Header {
    WcsHeader::celestial("TAN-SIP", [40.0, 30.0], [120.0, -30.0], [-1e-3, 1e-3])
        .pc([1.0, 0.2, -0.1, 1.0])
        .sip()
        .image_header()
}

/// The world coordinates of a shifted or scaled pixel through a derived WCS, against the
/// parent's: the offsets round, and the projection carries the rounding.
const SHIFT_ROUNDING: Tolerance = Tolerance::abs(1e-12, "the rounding of the shifted offsets");

/// A crop's pixel `p − start` is the parent's pixel `p`, for TAN and for TAN with SIP;
/// `Region::All` gives the same WCS.
#[test]
fn crop_shifts_the_reference_pixel() {
    let ranges = [10..100, 20..200];
    for header in [
        WcsHeader::celestial("TAN", [40.0, 30.0], [120.0, -30.0], [-1e-3, 1e-3]).image_header(),
        rotated_sip_header(),
    ] {
        let parent = Wcs::from_header(&header, None).unwrap();
        let crop = parent.crop(Region::from(&ranges)).unwrap();
        assert_eq!(crop.metadata().distortion, parent.metadata().distortion);
        for p in [[11.0, 21.0], [57.5, 99.25], [99.0, 199.0]] {
            assert_close(
                &to_world(&crop, &[p[0] - 10.0, p[1] - 20.0]).unwrap(),
                &to_world(&parent, &p).unwrap(),
                SHIFT_ROUNDING,
                &format!("{p:?}"),
            );
        }
        assert_same_transform(&parent, &parent.crop(Region::All).unwrap(), "Region::All");
    }
    let parent = Wcs::from_header(&rotated_sip_header(), None).unwrap();
    assert!(matches!(
        parent.crop(Region::Section(std::slice::from_ref(&(0..1)))),
        Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: 2,
            got: 1
        })
    ));
}

/// A bin's pixel `p'` is the parent's pixel `b·(p' − 0.5) + 0.5` on each axis: for a
/// 2 × 2 bin of TAN, and a 2 × 3 bin of a rotated, sheared `PC` with SIP. The SIP terms of
/// the 2 × 3 bin, worked by hand: `A_2_0 = 0.001·2²/2 = 0.002`,
/// `A_1_1 = 0.002·2·3/2 = 0.006`, `B_0_2 = 0.003·3²/3 = 0.009`.
#[test]
fn bin_scales_the_pixel_axes() {
    let tan = Wcs::from_header(
        &WcsHeader::celestial("TAN", [40.0, 30.0], [120.0, -30.0], [-1e-3, 1e-3]).image_header(),
        None,
    )
    .unwrap();
    for (parent, factors) in [
        (tan, [2, 2]),
        (
            Wcs::from_header(&rotated_sip_header(), None).unwrap(),
            [2, 3],
        ),
    ] {
        let binned = parent.bin(&factors).unwrap();
        for p in [[1.0, 1.0], [7.25, 3.5], [30.0, 22.0]] {
            let parent_pixel = [
                factors[0] as f64 * (p[0] - 0.5) + 0.5,
                factors[1] as f64 * (p[1] - 0.5) + 0.5,
            ];
            assert_close(
                &to_world(&binned, &p).unwrap(),
                &to_world(&parent, &parent_pixel).unwrap(),
                SHIFT_ROUNDING,
                &format!("{factors:?} {p:?}"),
            );
        }
    }
    let mut header = Header::new();
    Wcs::from_header(&rotated_sip_header(), None)
        .unwrap()
        .bin(&[2, 3])
        .unwrap()
        .write_into(&mut header, None)
        .unwrap();
    for (name, value) in [("A_2_0", 2e-3), ("A_1_1", 6e-3), ("B_0_2", 9e-3)] {
        assert_close(
            &[header.get_real(name).unwrap().unwrap()],
            &[value],
            Tolerance::rel(
                2.0 * f64::EPSILON,
                "a product and a quotient of the parent's term",
            ),
            name,
        );
    }
    // CRPIX1 = (40 − 0.5)/2 + 0.5 = 20.25, CRPIX2 = (30 − 0.5)/3 + 0.5.
    assert_cards(
        &header,
        &[
            ("CRPIX1", 20.25.into()),
            ("CRPIX2", ((30.0 - 0.5) / 3.0 + 0.5).into()),
        ],
    );
    let parent = Wcs::from_header(&rotated_sip_header(), None).unwrap();
    assert!(matches!(
        parent.bin(&[2]),
        Err(FitsError::RankMismatch {
            ranked: Ranked::BinFactors,
            expected: 2,
            got: 1
        })
    ));
}
