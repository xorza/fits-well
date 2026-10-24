use crate::fits_error::GrismDefect;
use crate::fits_error::UnitKind;
use crate::header_model::{Header, keywords};
use crate::internals::tolerance::{Tolerance, assert_close};
use crate::internals::wcs_header::{WcsHeader, axis_header};
use crate::world_coordinates::internals::{to_pixel, to_world};

use crate::world_coordinates::Wcs;
use crate::world_coordinates::axis_transform::spectral_kind::{
    Characteristic, SPEED_OF_LIGHT, conversion_derivative, convert,
};
use crate::world_coordinates::axis_transform::spectral_rest::ResolvedRest;
use crate::world_coordinates::axis_transform::*;

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

/// One ulp δ from the rest value a velocity is c·δ/ν₀ (or c·δ/λ₀) to a part in 1e16.
/// The forms 1 − ν/ν₀ and ν₀² − ν² round the quotient or the squares first, which
/// leaves δ at most a few ulps of 1 and moves the velocity by a third of itself; the
/// differences ν₀ − ν and λ − λ₀ are exact there.
#[test]
fn velocities_next_to_the_rest_value_keep_their_digits() {
    use crate::world_coordinates::axis_transform::spectral_kind::SpectralKind;
    let rest = ResolvedRest {
        frequency: 1_420_405_751.0,
        wavelength: SPEED_OF_LIGHT / 1_420_405_751.0,
    };
    let assert_near = |got: Option<f64>, want: f64, context: &str| {
        let got = got.unwrap();
        assert!(
            ((got - want) / want).abs() <= 4.0 * f64::EPSILON,
            "{context}: {got:e}, want {want:e}"
        );
    };
    let frequency = rest.frequency.next_down();
    let frequency_offset = SPEED_OF_LIGHT * (rest.frequency - frequency) / rest.frequency;
    let wavelength = rest.wavelength.next_up();
    let wavelength_offset = (wavelength - rest.wavelength) / rest.wavelength;
    assert_near(
        SpectralKind::RadioVelocity.world_from_characteristic(frequency, rest),
        frequency_offset,
        "VRAD",
    );
    assert_near(
        convert(
            Characteristic::Frequency,
            Characteristic::Velocity,
            frequency,
            rest,
        ),
        frequency_offset,
        "F2V",
    );
    assert_near(
        SpectralKind::OpticalVelocity.world_from_characteristic(wavelength, rest),
        SPEED_OF_LIGHT * wavelength_offset,
        "VOPT",
    );
    assert_near(
        SpectralKind::Redshift.world_from_characteristic(wavelength, rest),
        wavelength_offset,
        "ZOPT",
    );
    assert_near(
        convert(
            Characteristic::Wavelength,
            Characteristic::Velocity,
            wavelength,
            rest,
        ),
        SPEED_OF_LIGHT * wavelength_offset,
        "W2V",
    );
}

#[test]
fn nonlinear_algorithms_are_classified_independently_of_coordinate_type() {
    let build = |t3: &str| {
        let mut h =
            WcsHeader::celestial("TAN", [1.0, 1.0], [45.0, 30.0], [-1e-3, 1e-3]).image_header();
        h.set_internal(keywords::NAXIS, 3)
            .set_internal("CTYPE3", t3)
            .set_internal("CRPIX3", 1.0)
            .set_internal("CRVAL3", 1.4e9)
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
        assert_eq!(wcs.description().unsupported_axes, expected, "{ctype}");
        assert!(wcs.celestial.is_some(), "{ctype} must retain the TAN pair");

        if unsupported {
            assert!(matches!(
                to_world(&wcs, &[1.0, 1.0, 3.0]),
                Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [2]
            ));
            assert!(matches!(
                to_pixel(&wcs, &[45.0, 30.0, 1.402e9]),
                Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [2]
            ));
        } else {
            let out = to_world(&wcs, &[1.0, 1.0, 3.0]).unwrap();
            let expected = if ctype.ends_with("-LOG") {
                1.4e9 * (2.0e6_f64 / 1.4e9).exp()
            } else {
                1.402e9
            };
            assert!((out[2] - expected).abs() < 1e-6, "{ctype}: {out:?}");
            // The reference pixel is the native pole, which the rotation takes to CRVAL
            // through the sine and cosine of δ₀.
            assert_close(
                &out[..2],
                &[45.0, 30.0],
                Tolerance::ulps(1.0, "the rotation's roundings of δ₀"),
                ctype,
            );
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

/// wcslib 8.5 evaluates the KPNO MARS grism of [`grism_wcs`] at pixels −1, 1 and 4.
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
        assert!(wcs.description().unsupported_axes.is_empty(), "{ctype}");
        for (expected_world, pixel) in goldens {
            let world = to_world(&wcs, &[pixel]).unwrap();
            assert!(
                (world[0] - expected_world).abs() < 1e-18,
                "{ctype} pixel {pixel}: {world:?}"
            );
            let round_trip = to_pixel(&wcs, &world).unwrap();
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
            .set_internal(keywords::NAXIS, 1)
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
        // PV1_0 is the groove density and PV1_1 the order.
        let expected = usize::from(parameter == "PV1_1");
        assert!(
            matches!(
                Wcs::from_header(&header, None),
                Err(FitsError::InvalidWcs {
                    defect: WcsDefect::Grism {
                        algorithm: "GRI",
                        defect: GrismDefect::MissingParameter { parameter }
                    }
                }) if parameter == expected
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

/// The world change of an axis of `ctype` for a relative change of 1 in the quantity it
/// samples: `reference` for a world proportional to it, c for a velocity, 1 for a
/// redshift or β. A rounding of the sampled quantity reaches the world scaled by this.
fn rounding_scale(ctype: &str, reference: f64) -> f64 {
    match &ctype[..4] {
        "VELO" | "VRAD" | "VOPT" => SPEED_OF_LIGHT,
        "ZOPT" | "BETA" => 1.0,
        _ => reference.abs(),
    }
}

/// Table 26's algorithms and the types derived from them (ENER, WAVN, VRAD, VOPT,
/// ZOPT, BETA), each evaluated at pixel 3 by wcslib with RESTFRQ = 1420405751 Hz.
/// wcslib's velocities lose digits to their differences of squares, VELO-F2V's
/// 1.3e-8 m/s of the exact 2000.0066712779978 that `spectral_frame`'s tests hold this
/// evaluation to; the bound of c·4ε = 2.7e-7 m/s covers both.
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
        let mut header = axis_header(case.ctype, case.unit, 1.0, case.reference, case.increment);
        header.set_internal(keywords::RESTFRQ, 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        assert!(
            wcs.description().unsupported_axes.is_empty(),
            "{}",
            case.ctype
        );
        let world = to_world(&wcs, &[3.0]).unwrap()[0];
        assert!(
            (world - case.world).abs() <= ROUNDING * rounding_scale(case.ctype, case.reference),
            "{}: got {world:.17e}, wcslib {:.17e}",
            case.ctype,
            case.world
        );
        let pixel = to_pixel(&wcs, &[case.world]).unwrap()[0];
        assert!(
            (pixel - 3.0).abs()
                <= ROUNDING * rounding_scale(case.ctype, case.reference) / case.increment.abs(),
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
        header.set_internal(keywords::RESTFRQ, 1_420_405_751.0);
        let wcs = Wcs::from_header(&header, None).unwrap();
        // The view reports the axis as declared; the world coordinate is in the default unit.
        assert_eq!(wcs.description().axes[0].cunit, case.unit);
        assert_eq!(
            wcs.description().axes[0].crval,
            case.reference,
            "{}",
            case.ctype
        );
        // The unit's factor and its product with the reference round, and the world
        // passes through the quantity the axis samples.
        let world = to_world(&wcs, &[1.0]).unwrap()[0];
        let bound = 2.0 * f64::EPSILON * rounding_scale(case.ctype, case.canonical_reference);
        assert!(
            (world - case.canonical_reference).abs() <= bound,
            "{} reference: {world:.17e}",
            case.ctype
        );
    }

    let mut invalid = Header::new();
    invalid
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "WAVE-F2W")
        .set_internal("CUNIT1", "Hz")
        .set_internal("CRVAL1", 1.0);
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidUnit { unit, expected: UnitKind::Length }) if unit == "Hz"
    ));
    invalid.set_internal("CUNIT1", "qHz");
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidUnit { unit, .. }) if unit == "qHz"
    ));
}

#[test]
fn logarithmic_axes_apply_domains_units_and_inverse() {
    let generic = axis_header("TIME-LOG", "d", 1.0, 100.0, 10.0);
    let generic = Wcs::from_header(&generic, None).unwrap();
    let expected = 100.0 * 0.2_f64.exp();
    assert_eq!(generic.description().axes[0].cunit, "d");
    assert!((to_world(&generic, &[3.0]).unwrap()[0] - expected).abs() < 1e-13);
    assert!((to_pixel(&generic, &[expected]).unwrap()[0] - 3.0).abs() < 1e-13);
    assert!(matches!(
        to_pixel(&generic, &[0.0]),
        Err(FitsError::WcsCoordinateOutOfDomain {
            axis: 0,
            algorithm: "LOG"
        })
    ));

    let frequency = axis_header("FREQ-LOG", "GHz", 1.0, 1.4, 0.001);
    let frequency = Wcs::from_header(&frequency, None).unwrap();
    let expected = 1.4e9 * (2.0e6_f64 / 1.4e9).exp();
    assert_eq!(frequency.description().axes[0].cunit, "GHz");
    assert_eq!(frequency.description().axes[0].crval, 1.4);
    assert!((to_world(&frequency, &[3.0]).unwrap()[0] - expected).abs() < 1e-6);
    assert!((to_pixel(&frequency, &[expected]).unwrap()[0] - 3.0).abs() < 1e-12);

    let mut invalid = Header::new();
    invalid
        .set_internal(keywords::NAXIS, 1)
        .set_internal("CTYPE1", "ABCD-LOG")
        .set_internal("CRVAL1", 0.0);
    assert!(matches!(
        Wcs::from_header(&invalid, None),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::LogarithmicReference { ctype }
        }) if ctype == "ABCD-LOG"
    ));
}

/// Every code parses to the algorithm that names it back, and an `X2P` code must
/// express the coordinate type's own characteristic and convert between two.
#[test]
fn spectral_algorithm_codes_round_trip_and_check_their_variables() {
    use crate::world_coordinates::axis_transform::spectral_algorithm::SpectralAlgorithm;

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
            .set_internal(keywords::NAXIS, 1)
            .set_internal("CTYPE1", ctype)
            .set_internal("CRVAL1", 1.4e9)
            .set_internal(keywords::RESTFRQ, 1.4e9);
        assert!(
            matches!(
                Wcs::from_header(&header, None),
                Err(FitsError::InvalidWcs {
                    defect: WcsDefect::InconsistentSpectralAlgorithm { ctype: got }
                }) if got == ctype
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
    use crate::world_coordinates::axis_transform::spectral_kind::SpectralKind;
    use crate::world_coordinates::axis_transform::spectral_rest::RestNeed;

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
