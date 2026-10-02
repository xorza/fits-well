use crate::error::FitsError;
use crate::header_model::Header;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::axis::spectral_kind::{
    Characteristic, conversion_derivative, convert,
};
use crate::world_coordinates::axis::spectral_rest::ResolvedRest;
use crate::world_coordinates::axis::*;
use crate::world_coordinates::internals::axis_header;

/// An in-domain value for each characteristic, all describing roughly the same
/// 1 µm photon, so every conversion below stays well inside its domain.
fn sample(characteristic: Characteristic) -> f64 {
    match characteristic {
        Characteristic::Frequency => 3.0e14,
        Characteristic::Wavelength | Characteristic::AirWavelength => 1.0e-6,
        Characteristic::Velocity => 1.0e7,
    }
}

/// [`convert`] and [`conversion_derivative`] are two parallel matches over the
/// same twelve characteristic pairs. Nothing makes the compiler check that an
/// arm of one agrees with its partner in the other, so a formula edited in one
/// place and not the other is a silent numerical error — a spectral axis that
/// still transforms, just wrongly, in one direction.
///
/// Differentiate `convert` numerically and hold `conversion_derivative` to it.
/// A central difference is second-order accurate, so with a relative step of
/// 1e-6 the two should agree far inside this tolerance; anything that disagrees
/// at 1e-5 is a wrong formula, not roundoff.
#[test]
fn conversion_derivatives_match_the_conversions_they_describe() {
    let rest = ResolvedRest {
        frequency: 3.0e14,
        wavelength: SPEED_OF_LIGHT / 3.0e14,
    };
    let all = [
        Characteristic::Frequency,
        Characteristic::Wavelength,
        Characteristic::AirWavelength,
        Characteristic::Velocity,
    ];
    let mut checked = 0;
    for from in all {
        for to in all {
            if from == to {
                continue;
            }
            let x = sample(from);
            let step = x.abs() * 1e-6;
            let ahead = convert(from, to, x + step, rest).unwrap();
            let behind = convert(from, to, x - step, rest).unwrap();
            let numeric = (ahead - behind) / (2.0 * step);
            let at = convert(from, to, x, rest).unwrap();
            let analytic = conversion_derivative(from, to, x, at, rest).unwrap();
            let relative = (analytic - numeric).abs() / numeric.abs();
            assert!(
                relative < 1e-5,
                "d({to:?})/d({from:?}) at {x:e}: analytic {analytic:e}, \
                 numeric {numeric:e} (relative error {relative:e})"
            );
            checked += 1;
        }
    }
    // Every ordered pair of distinct characteristics is a real algorithm in
    // Table 26; none may be silently absent from the sweep.
    assert_eq!(checked, 12);
}

#[test]
fn nonlinear_algorithms_are_classified_independently_of_coordinate_type() {
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
        // §4.2.1 keeps leading blanks in a string value, so a sloppy writer's
        // `' FREQ-LOG'` still has to classify as the spectral axis it names.
        (" FREQ-LOG", false),
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
    let mut header = axis_header(ctype, "m", 1.0, reference, 1.0e-7);
    header
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
                Err(FitsError::InvalidWcs { .. })
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
    /// The world change for a relative change of 1 in the sampled quantity: the
    /// reference for a world proportional to it, c for a velocity, 1 for a redshift
    /// or β. A rounding in the sampled quantity reaches the world scaled by this — a
    /// velocity is a ratio minus 1, so there it is the digits the subtraction cancels.
    scale: f64,
}

/// Table 26's algorithms and the types derived from them (ENER, WAVN, VRAD, VOPT,
/// ZOPT, BETA), each evaluated at pixel 3 by wcslib with RESTFRQ = 1420405751 Hz.
#[test]
fn spectral_algorithms_match_wcslib() {
    // A conversion chain is a few products, quotients and a square root, each
    // rounding by at most half an ulp of the sampled quantity.
    const ROUNDING: f64 = 4.0 * f64::EPSILON;
    let cases = [
        SpectralGolden {
            ctype: "WAVE-F2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_330_354_017_17,
            scale: 0.211_061_140_655_716_77,
        },
        SpectralGolden {
            ctype: "VELO-F2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 2_000.006_671_265_423_6,
            scale: SPEED_OF_LIGHT,
        },
        SpectralGolden {
            ctype: "AWAV-F2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_811_019_260_08,
            scale: 0.211_003_621_269_199_05,
        },
        SpectralGolden {
            ctype: "FREQ-W2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_408_571.067_528,
            scale: 1_420_405_751.0,
        },
        SpectralGolden {
            ctype: "VELO-W2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.993_328_738_271_7,
            scale: SPEED_OF_LIGHT,
        },
        SpectralGolden {
            ctype: "AWAV-W2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_621_269_199_03,
            scale: 0.211_003_621_269_199_05,
        },
        SpectralGolden {
            ctype: "FREQ-V2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_407_161.033_065_3,
            scale: 1_420_405_751.0,
        },
        SpectralGolden {
            ctype: "WAVE-V2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_235_504_845_66,
            scale: 0.211_061_140_655_716_77,
        },
        SpectralGolden {
            ctype: "AWAV-V2A",
            unit: "m",
            reference: 0.211_003_621_269_199_05,
            increment: 1e-4,
            world: 0.211_203_716_144_208_21,
            scale: 0.211_003_621_269_199_05,
        },
        SpectralGolden {
            ctype: "FREQ-A2F",
            unit: "Hz",
            reference: 1_420_405_751.0,
            increment: 1e6,
            world: 1_422_408_571.067_528,
            scale: 1_420_405_751.0,
        },
        SpectralGolden {
            ctype: "WAVE-A2W",
            unit: "m",
            reference: 0.211_061_140_655_716_77,
            increment: 1e-4,
            world: 0.211_261_140_655_716_77,
            scale: 0.211_061_140_655_716_77,
        },
        SpectralGolden {
            ctype: "VELO-A2V",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.993_328_738_271_7,
            scale: SPEED_OF_LIGHT,
        },
        SpectralGolden {
            ctype: "ENER-W2F",
            unit: "J",
            reference: 9.411_715_746_760_2e-25,
            increment: 6.626_075_5e-28,
            world: 9.424_986_583_740_556e-25,
            scale: 9.411_715_746_760_2e-25,
        },
        SpectralGolden {
            ctype: "WAVN-W2F",
            unit: "/m",
            reference: 4.737_963_591_465_666,
            increment: 0.003_335_640_951_981_520_5,
            world: 4.744_644_280_102_364,
            scale: 4.737_963_591_465_666,
        },
        SpectralGolden {
            ctype: "VRAD-W2F",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 1_999.986_657_561_148_6,
            scale: SPEED_OF_LIGHT,
        },
        SpectralGolden {
            ctype: "VOPT-F2W",
            unit: "m/s",
            reference: 0.0,
            increment: 1e3,
            world: 2_000.013_342_678_547,
            scale: SPEED_OF_LIGHT,
        },
        SpectralGolden {
            ctype: "ZOPT-F2W",
            unit: "",
            reference: 0.0,
            increment: 1e-5,
            world: 2.000_040_000_793_568e-5,
            scale: 1.0,
        },
        SpectralGolden {
            ctype: "BETA-F2V",
            unit: "",
            reference: 0.0,
            increment: 3.335_640_951_981_520_5e-6,
            world: 6.671_304_156_909_189e-6,
            scale: 1.0,
        },
    ];
    for case in cases {
        let mut header = axis_header(case.ctype, case.unit, 1.0, case.reference, case.increment);
        header.set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert!(wcs.view().unsupported_axes.is_empty(), "{}", case.ctype);
        let world = wcs.pixel_to_world(&[3.0]).unwrap()[0];
        assert!(
            (world - case.world).abs() <= ROUNDING * case.scale,
            "{}: got {world:.17e}, wcslib {:.17e}",
            case.ctype,
            case.world
        );
        let pixel = wcs.world_to_pixel(&[case.world]).unwrap()[0];
        assert!(
            (pixel - 3.0).abs() <= ROUNDING * case.scale / case.increment.abs(),
            "{} inverse: {pixel:.17e}",
            case.ctype
        );
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
        let mut header = axis_header(
            case.ctype,
            case.unit,
            1.0,
            case.reference,
            case.reference / 100.0,
        );
        header.set_internal("RESTFRQ", 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        // The view reports the axis as declared; the world coordinate is in the default unit.
        assert_eq!(wcs.view().axes[0].cunit, case.unit);
        assert_eq!(wcs.view().axes[0].crval, case.reference, "{}", case.ctype);
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
        Wcs::from_header(&invalid, None),
        Err(crate::error::FitsError::InvalidUnit { unit, expected: "a length unit" }) if unit == "Hz"
    ));
    invalid.set_internal("CUNIT1", "qHz");
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(crate::error::FitsError::InvalidUnit { unit, .. }) if unit == "qHz"
    ));
}

#[test]
fn logarithmic_axes_apply_domains_units_and_inverse() {
    let generic = axis_header("TIME-LOG", "d", 1.0, 100.0, 10.0);
    let generic = Wcs::from_header(&generic, None).unwrap();
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

    let frequency = axis_header("FREQ-LOG", "GHz", 1.0, 1.4, 0.001);
    let frequency = Wcs::from_header(&frequency, None).unwrap();
    let expected = 1.4e9 * (2.0e6_f64 / 1.4e9).exp();
    assert_eq!(frequency.view().axes[0].cunit, "GHz");
    assert_eq!(frequency.view().axes[0].crval, 1.4);
    assert!((frequency.pixel_to_world(&[3.0]).unwrap()[0] - expected).abs() < 1e-6);
    assert!((frequency.world_to_pixel(&[expected]).unwrap()[0] - 3.0).abs() < 1e-12);

    let mut invalid = Header::new();
    invalid
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "ABCD-LOG")
        .set_internal("CRVAL1", 0.0);
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidWcs { .. })
    ));
}

/// Every code parses to the algorithm that names it back, and an `X2P` code must
/// express the coordinate type's own characteristic and convert between two.
#[test]
fn spectral_algorithm_codes_round_trip_and_check_their_variables() {
    use crate::world_coordinates::axis::spectral_algorithm::SpectralAlgorithm;

    let mut names = Vec::new();
    for sampled in Characteristic::ALL {
        for expressed in Characteristic::ALL {
            let code = format!(
                "{}2{}",
                sampled.letter() as char,
                expressed.letter() as char
            );
            let algorithm = SpectralAlgorithm::parse(&code).unwrap();
            assert_eq!(algorithm, SpectralAlgorithm::Pair { sampled, expressed });
            assert_eq!(algorithm.name(), code);
            names.push(code);
        }
    }
    assert_eq!(names.len(), 16);
    for code in ["GRI", "GRA"] {
        assert_eq!(SpectralAlgorithm::parse(code).unwrap().name(), code);
    }
    assert_eq!(SpectralAlgorithm::parse("TAB"), None);
    assert_eq!(SpectralAlgorithm::parse("F3W"), None);

    for ctype in ["FREQ-F2F", "FREQ-W2V"] {
        let mut header = Header::new();
        header
            .set_internal("NAXIS", 1)
            .set_internal("CTYPE1", ctype)
            .set_internal("CRVAL1", 1.4e9)
            .set_internal("RESTFRQ", 1.4e9);
        assert!(
            matches!(
                Wcs::from_header(&header, None),
                Err(FitsError::InvalidWcs { detail }) if detail.contains("inconsistent variables")
            ),
            "{ctype}"
        );
    }
}

/// wcslib's `restreq`: a velocity-type coordinate needs a rest value, a conversion
/// to or from velocity needs one, and when both do they cancel. `VRAD` is a
/// frequency-based type and `ZOPT` a wavelength-based one (Paper III, Table 1).
#[test]
fn rest_need_follows_restreq() {
    use crate::world_coordinates::axis::spectral_kind::SpectralKind;
    use crate::world_coordinates::axis::spectral_rest::RestNeed;

    let cases = [
        (
            SpectralKind::Frequency,
            Characteristic::Wavelength,
            RestNeed::Unused,
        ),
        (
            SpectralKind::Frequency,
            Characteristic::Velocity,
            RestNeed::Required,
        ),
        (
            SpectralKind::RadioVelocity,
            Characteristic::Frequency,
            RestNeed::Required,
        ),
        (
            SpectralKind::RadioVelocity,
            Characteristic::Velocity,
            RestNeed::Cancels,
        ),
        (
            SpectralKind::RelativisticVelocity,
            Characteristic::Frequency,
            RestNeed::Required,
        ),
        (
            SpectralKind::Redshift,
            Characteristic::Wavelength,
            RestNeed::Required,
        ),
    ];
    for (kind, sampled, need) in cases {
        assert_eq!(
            RestNeed::of(kind, sampled),
            need,
            "{kind:?} sampled {sampled:?}"
        );
    }
}
