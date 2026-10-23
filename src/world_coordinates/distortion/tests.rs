use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::header_model::Header;
use crate::reader::internals::fixture_bytes;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::distortion::Distortion;
use crate::world_coordinates::distortion::sip::Sip;
use crate::world_coordinates::distortion::tpv::Tpv;
use crate::world_coordinates::internals::{celestial_header, to_pixel, to_world};
use crate::world_coordinates::projection::Projection;

/// A header that `test_resources` holds as bare records (astropy's
/// `astropy/wcs/tests/data`, BSD-3).
fn fixture_header(name: &str) -> Header {
    Header::parse(&fixture_bytes(name)).unwrap()
}

/// `header` with its distortion removed: plain TAN, no SIP keyword and no `PV`. Its round
/// trip is the projection's own error, which the present TAN pipeline makes larger than
/// a distortion's inverse.
fn without_distortion(header: &Header) -> Wcs {
    let mut header = header.clone();
    header.remove_where(|name| {
        ["A_", "B_", "AP_", "BP_", "PV1_", "PV2_"]
            .iter()
            .any(|prefix| name.starts_with(prefix))
    });
    header
        .set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    Wcs::from_header(&header, None).unwrap()
}

/// How far `pixel` lands from itself through world coordinates and back.
fn round_trip_error(wcs: &Wcs, pixel: &[f64]) -> f64 {
    distance(
        &to_pixel(wcs, &to_world(wcs, pixel).unwrap()).unwrap(),
        pixel,
    )
}

/// The largest difference between two points, each axis on its own.
fn distance(a: &[f64], b: &[f64]) -> f64 {
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

/// A 12-character `CTYPEia` carries a distortion code after any projection; `TPV` is
/// TAN with the TPV polynomial; an unknown code is no projection this crate evaluates.
#[test]
fn ctype_distortion_codes() {
    let projection = |ctype: &str| Ctype::parse(ctype).projection();
    assert_eq!(
        projection("RA---TAN-SIP"),
        Some((Projection::Tan, Some(Distortion::Sip)))
    );
    assert_eq!(
        projection("DEC--SIN-SIP"),
        Some((Projection::Sin, Some(Distortion::Sip)))
    );
    assert_eq!(
        projection("RA---TPV"),
        Some((Projection::Tan, Some(Distortion::Tpv)))
    );
    assert_eq!(projection("GLON-CAR"), Some((Projection::Car, None)));
    assert_eq!(projection("FREQ"), None);
    assert_eq!(projection("RA---TAN-XYZ"), None);
    assert_eq!(projection("RA---TPV-SIP"), None);
    assert_eq!(Ctype::parse("RA---TAN-SIP").head, "RA");

    let mut header = celestial_header("TAN-XYZ", [1.0, 1.0], [0.0, 0.0], [1e-3, 1e-3]);
    let wcs = Wcs::from_header(&header, None).unwrap();
    assert_eq!(wcs.metadata().unsupported_axes, [0, 1]);
    header.set_internal("CTYPE1", "RA---TAN");
    assert!(matches!(
        Wcs::from_header(&header, None),
        Err(FitsError::ConflictingWcsKeywords { .. })
    ));
}

/// An order-2 SIP worked by hand: `A_2_0 = 0.001`, `A_1_1 = 0.002`, `B_0_2 = 0.003`
/// at `(u, v) = (3, −2)` give `f = 0.001·9 + 0.002·(−6) = −0.003` and
/// `g = 0.003·4 = 0.012`, so `(U, V) = (2.997, −1.988)`. The full transform equals a
/// plain TAN at the pixel those offsets name.
#[test]
fn sip_is_a_prior_distortion_worked_by_hand() {
    let mut header = celestial_header("TAN-SIP", [10.0, 20.0], [30.0, 40.0], [1e-3, 1e-3]);
    header
        .set_internal("A_ORDER", 2)
        .set_internal("A_2_0", 1e-3)
        .set_internal("A_1_1", 2e-3)
        .set_internal("B_ORDER", 2)
        .set_internal("B_0_2", 3e-3);
    let sip = Sip::from_header(&header).unwrap().unwrap();
    let [u, v] = sip.forward(3.0, -2.0);
    // Within the rounding of a three-term sum.
    assert!((u - 2.997).abs() <= 4.0 * f64::EPSILON * 3.0, "{u}");
    assert!((v + 1.988).abs() <= 4.0 * f64::EPSILON * 2.0, "{v}");

    let wcs = Wcs::from_header(&header, None).unwrap();
    assert_eq!(wcs.metadata().distortion, Some(Distortion::Sip));
    let plain = Wcs::from_header(
        &celestial_header("TAN", [10.0, 20.0], [30.0, 40.0], [1e-3, 1e-3]),
        None,
    )
    .unwrap();
    let distorted = to_world(&wcs, &[13.0, 18.0]).unwrap();
    let shifted = to_world(&plain, &[10.0 + 2.997, 20.0 - 1.988]).unwrap();
    // 1e-12° bounds the rounding of the offsets and of the projection.
    assert!(
        distance(&distorted, &shifted) <= 1e-12,
        "{distorted:?} {shifted:?}"
    );
}

/// astropy's IRAC header: its `test_sip` gives pixel (200, 200) at origin 0 the focal
/// offsets (72, 72) within a relative 1e-3, which is `72 + (A_0_2 + A_1_1 + A_2_0)·72²`.
/// SIP adds no more than 1e-10 pixel to the round trip of each corner of the 256² image,
/// whether the inverse starts from `AP`/`BP` or not, and applies with no `-SIP` code too.
#[test]
fn sip_on_astropys_irac_header() {
    let header = fixture_header("wcs_irac_sip.hdr");
    let [u, v] = Sip::from_header(&header)
        .unwrap()
        .unwrap()
        .forward(72.0, 72.0);
    let expected_u = 72.0 + (6.666e-6 + 1.801e-5 - 2.353e-5) * 72.0 * 72.0;
    assert!((u / 72.0 - 1.0).abs() <= 1e-3 && (v / 72.0 - 1.0).abs() <= 1e-3);
    assert!((u - expected_u).abs() <= 1e-12, "{u} {expected_u}");

    let wcs = Wcs::from_header(&header, None).unwrap();
    assert_eq!(wcs.metadata().distortion, Some(Distortion::Sip));
    let mut without_approximation = header.clone();
    without_approximation.remove_where(|name| name.starts_with("AP_") || name.starts_with("BP_"));
    let plain_start = Wcs::from_header(&without_approximation, None).unwrap();
    let undistorted = without_distortion(&header);
    for corner in [[1.0, 1.0], [256.0, 1.0], [1.0, 256.0], [256.0, 256.0]] {
        let bound = round_trip_error(&undistorted, &corner) + 1e-10;
        assert!(round_trip_error(&wcs, &corner) <= bound, "{corner:?}");
        assert!(
            round_trip_error(&plain_start, &corner) <= bound,
            "{corner:?}"
        );
    }

    let mut no_code = header.clone();
    no_code
        .set_internal("CTYPE1", "RA---TAN")
        .set_internal("CTYPE2", "DEC--TAN");
    let uncoded = Wcs::from_header(&no_code, None).unwrap();
    assert_eq!(uncoded.metadata().distortion, Some(Distortion::Sip));
    assert_eq!(
        to_world(&uncoded, &[17.0, 230.0]).unwrap(),
        to_world(&wcs, &[17.0, 230.0]).unwrap()
    );

    let mut no_keywords = header.clone();
    no_keywords.remove_where(|name| name.starts_with("A_") || name.starts_with("B_"));
    assert!(matches!(
        Wcs::from_header(&no_keywords, None),
        Err(FitsError::MissingKeyword { keyword }) if keyword == "A_ORDER"
    ));
    let mut too_high = header.clone();
    too_high.set_internal("A_ORDER", 10);
    assert!(matches!(
        Wcs::from_header(&too_high, None),
        Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "A_ORDER"
    ));
    let mut b_alone = header;
    b_alone.remove_where(|name| name.starts_with("A_"));
    assert!(matches!(
        Wcs::from_header(&b_alone, None),
        Err(FitsError::MissingKeyword { keyword }) if keyword == "A_ORDER"
    ));
}

/// SIP keywords carry no alternate letter, so an alternate description takes no SIP.
#[test]
fn an_alternate_description_takes_no_sip() {
    let mut header = celestial_header("TAN-SIP", [10.0, 20.0], [30.0, 40.0], [1e-3, 1e-3]);
    header
        .set_internal("A_ORDER", 2)
        .set_internal("A_2_0", 1e-3)
        .set_internal("B_ORDER", 2);
    for (name, value) in [("CTYPE1A", "RA---TAN-SIP"), ("CTYPE2A", "DEC--TAN-SIP")] {
        header.set_internal(name, value);
    }
    for (name, value) in [
        ("CRPIX1A", 10.0),
        ("CRPIX2A", 20.0),
        ("CRVAL1A", 30.0),
        ("CRVAL2A", 40.0),
        ("CDELT1A", 1e-3),
        ("CDELT2A", 1e-3),
    ] {
        header.set_internal(name, value);
    }
    let alternate = Wcs::from_header(&header, 'A').unwrap();
    assert_eq!(alternate.metadata().distortion, None);
    assert_eq!(
        Wcs::from_header(&header, None)
            .unwrap()
            .metadata()
            .distortion,
        Some(Distortion::Sip)
    );
}

/// Pixel offsets of 1e5 have a spacing of 1.5e-11, which an absolute 1e-13 can never
/// meet; the relative stop rule lets the inverse converge, and SIP adds no more than
/// 1e-9 pixel to the whole round trip.
#[test]
fn the_sip_inverse_converges_far_from_crpix() {
    let mut header = celestial_header("TAN-SIP", [1.0, 1.0], [0.0, 0.0], [1e-6, 1e-6]);
    header
        .set_internal("A_ORDER", 2)
        .set_internal("A_2_0", 1e-12)
        .set_internal("B_ORDER", 2)
        .set_internal("B_0_2", 1e-12);
    let sip = Sip::from_header(&header).unwrap().unwrap();
    let [u, v] = sip.forward(1e5, 1.0);
    let [back_u, back_v] = sip.inverse(u, v).unwrap();
    assert!(
        (back_u - 1e5).abs() <= 1e-8 && (back_v - 1.0).abs() <= 1e-13,
        "{back_u} {back_v}"
    );
    let wcs = Wcs::from_header(&header, None).unwrap();
    let pixel = [1e5 + 1.0, 2.0];
    let bound = round_trip_error(&without_distortion(&header), &pixel) + 1e-9;
    assert!(round_trip_error(&wcs, &pixel) <= bound);
}

/// astropy's PTF header in its two forms, SIP and TPV, give one world coordinate at
/// `CRPIX` to 6 decimals, as astropy's `test_sip_tpv_agreement` asserts there. Over 25
/// pixels across the 2048 × 4096 image the forms differ by 3.6e-15° measured, one unit
/// in the last place of the declination: the headers are an exact conversion of each
/// other, so the two implementations check each other to rounding, bounded here at
/// 1e-13°, two units in the last place of the right ascension near 275°. Both headers
/// declare the deprecated `RADECSYS`.
#[test]
fn sip_and_tpv_forms_of_one_header_agree() {
    let sip = Wcs::from_header(&fixture_header("wcs_siponly.hdr"), None).unwrap();
    let tpv = Wcs::from_header(&fixture_header("wcs_tpvonly.hdr"), None).unwrap();
    assert_eq!(sip.metadata().distortion, Some(Distortion::Sip));
    assert_eq!(tpv.metadata().distortion, Some(Distortion::Tpv));
    let crpix = [-3925.16, 4360.23];
    assert!(
        distance(
            &to_world(&sip, &crpix).unwrap(),
            &to_world(&tpv, &crpix).unwrap()
        ) < 5e-7
    );
    let undistorted = without_distortion(&fixture_header("wcs_tpvonly.hdr"));
    let mut largest = 0.0f64;
    for x in [1.0, 512.0, 1024.0, 1536.0, 2048.0] {
        for y in [1.0, 1024.0, 2048.0, 3072.0, 4096.0] {
            let (a, b) = (
                to_world(&sip, &[x, y]).unwrap(),
                to_world(&tpv, &[x, y]).unwrap(),
            );
            largest = largest.max(distance(&a, &b));
            let bound = round_trip_error(&undistorted, &[x, y]) + 1e-9;
            assert!(
                round_trip_error(&tpv, &[x, y]) <= bound,
                "TPV round trip at {x}, {y}"
            );
        }
    }
    assert!(largest <= 1e-13, "{largest}");
}

/// A TPV with no `PV` is plain TAN, bit for bit; one term worked by hand:
/// `PV1_4 = 0.5` (x²) and `PV2_3 = 0.25` (r) give, at `(x, y) = (0.3, 0.4)`,
/// `ξ = 0.3 + 0.5·0.09 = 0.345` and `η = 0.4 + 0.25·0.5 = 0.525`.
#[test]
fn tpv_is_a_sequent_distortion_worked_by_hand() {
    let tan = Wcs::from_header(
        &celestial_header("TAN", [5.0, 6.0], [20.0, 30.0], [1e-3, 1e-3]),
        None,
    )
    .unwrap();
    let mut header = celestial_header("TPV", [5.0, 6.0], [20.0, 30.0], [1e-3, 1e-3]);
    let identity = Wcs::from_header(&header, None).unwrap();
    for pixel in [[1.0, 1.0], [300.0, -70.0]] {
        let world = to_world(&tan, &pixel).unwrap();
        assert_eq!(to_world(&identity, &pixel).unwrap(), world);
        assert_eq!(
            to_pixel(&identity, &world).unwrap(),
            to_pixel(&tan, &world).unwrap()
        );
    }
    header
        .set_internal("PV1_4", 0.5)
        .set_internal("PV2_3", 0.25);
    let tpv = Tpv::from_header(&header, Alternate::PRIMARY, 0, 1).unwrap();
    let [xi, eta] = tpv.forward(0.3, 0.4);
    // Within the rounding of a two-term sum.
    assert!((xi - 0.345).abs() <= 2.0 * f64::EPSILON, "{xi}");
    assert!((eta - 0.525).abs() <= 2.0 * f64::EPSILON, "{eta}");
    // The stop rule leaves residuals of at most 1e-13, and the Jacobian's diagonal,
    // `1 + x = 1.3` and `1 + 0.25·y/r = 1.2`, is above 1, so each error is smaller.
    let [x, y] = tpv.inverse(xi, eta).unwrap();
    assert!(
        (x - 0.3).abs() <= 1e-13 && (y - 0.4).abs() <= 1e-13,
        "{x} {y}"
    );
}
