//! The cylindrical and pseudo-cylindrical kernels: their poles, digits and the Bonne
//! limit.

use crate::world_coordinates::projection::tests::*;

#[test]
fn bonne_with_zero_theta1_equals_sfl() {
    // §5.5.1: Bonne's projection at θ₁ = 0 is exactly the sinusoidal SFL. A BON
    // header with PV2_1 = 0 must decode identically to an SFL header (and never hit
    // the 1/tan 0 singularity), so it is *decoded* — not flagged unsupported.
    let build = |proj: &str, pv1: Option<f64>| {
        let mut h =
            WcsHeader::celestial(proj, [50.0, 50.0], [45.0, 0.0], [-0.5, 0.5]).image_header();
        if let Some(v) = pv1 {
            h.set_internal("PV2_1", v);
        }
        Wcs::from_header(&h, None).unwrap()
    };
    let bon = build("BON", Some(0.0));
    let sfl = build("SFL", None);
    assert!(bon.celestial.is_some() && bon.description().unsupported_axes.is_empty());
    for pixel in [[20.0, 70.0], [80.0, 30.0], [55.0, 45.0]] {
        assert_eq!(
            to_world(&bon, &pixel).unwrap(),
            to_world(&sfl, &pixel).unwrap(),
            "{pixel:?}"
        );
    }
    // The forward (SFL) path round-trips too. Each world coordinate is below 256°, whose
    // ulp is 128 ε, and a pixel is 0.5°.
    wcs_header::assert_round_trip(
        &bon,
        &[[40.0, 60.0]],
        Tolerance::abs(
            2.0 * 128.0 * f64::EPSILON / 0.5,
            "two ulps of the world coordinate",
        ),
        "BON θ₁ = 0",
    );
}

/// MER's y = ln tan(45° + θ/2) and its inverse 2 atan(eʸ) − 90° cancel near the
/// equator, where y ≈ θ; asinh(tan θ) and atan(sinh y) keep θ to its last bits: at
/// θ = 1e-10°, y = θ (1 + O(θ²)).
#[test]
fn mercator_keeps_its_digits_near_the_equator() {
    let pv = raw(Projection::Mer, &[]);
    for theta in [1e-10, -3e-7, 1e-3] {
        let projected = pv
            .project(NativeCoordinate::from_degrees(10.0, theta))
            .unwrap();
        tolerance::assert_within(
            projected.y,
            theta,
            Tolerance::rel(
                theta * theta * 1e-3 + 2.0 * f64::EPSILON,
                "y = θ (1 + O(θ²)) in degrees, and two roundings",
            ),
            &format!("MER y at θ = {theta}"),
        );
        let back = pv.deproject(10.0, projected.y).unwrap();
        tolerance::assert_within(
            back.theta(),
            theta,
            Tolerance::ulps(4.0, "the forward and the inverse, two roundings each"),
            &format!("MER θ = {theta}"),
        );
    }
}

/// SFL's meridians meet at the poles, where `x / cos y` takes `cos 90° = 0` exactly: the
/// pole itself is φ = 0.
#[test]
fn sfl_poles_have_one_longitude() {
    for projection in [Projection::Sfl, Projection::Bon] {
        let pv = raw(projection, &[]);
        for y in [90.0, -90.0] {
            assert_eq!(
                pv.deproject(0.0, y).unwrap(),
                NativeCoordinate::from_degrees(0.0, y),
                "{projection:?}"
            );
        }
    }
}

/// Next to a MOL pole the plane's y changes with θ as `c^(2/3)` of the colatitude c, so
/// one ulp of y moves θ by far more than one ulp; the round trip keeps to that change, where
/// π sin θ and asin(y/(√2·180/π)) lost 2.3e6 ε at 1e-5° from the pole.
#[test]
fn mollweide_keeps_its_digits_next_to_the_poles() {
    let kernel = raw(Projection::Mol, &[]);
    for colatitude in [10.0, 1e-1, 1e-3, 1e-5, 1e-9] {
        for theta in [90.0 - colatitude, colatitude - 90.0] {
            let native = NativeCoordinate::from_degrees(73.0, theta);
            let projected = kernel.project(native).unwrap();
            let back = kernel.deproject(projected.x, projected.y).unwrap();
            let next = projected.y.abs().next_down().copysign(projected.y);
            let ulp_of_y = chord(back, kernel.deproject(projected.x, next).unwrap());
            tolerance::assert_close(
                &[chord(native, back)],
                &[0.0],
                Tolerance::abs(
                    16.0 * f64::EPSILON + 2.0 * ulp_of_y,
                    "a few roundings, and two ulps of y",
                ),
                &format!("θ = {theta}"),
            );
        }
    }
}

/// MOL and AIT take each native pole to (0, ±√2·180/π), on the edge of the map, where
/// every meridian meets, and back to φ = 0.
#[test]
fn equatorial_poles_are_finite_and_have_canonical_longitude() {
    for projection in [Projection::Mol, Projection::Ait] {
        let pv = raw(projection, &[]);
        for theta in [-90.0, 90.0] {
            let projected = pv
                .project(NativeCoordinate::from_degrees(73.0, theta))
                .unwrap();
            let pole_rounding = Tolerance::abs(1e-12, "the pole's trigonometry, ulps of 81°");
            tolerance::assert_close(
                &[projected.x, projected.y],
                &[0.0, theta.signum() * SQRT_2 * R2D],
                pole_rounding,
                &format!("{projection:?} projected pole"),
            );

            let native = pv.deproject(projected.x, projected.y).unwrap();
            assert_eq!(native.phi, 0.0, "{projection:?}");
            tolerance::assert_within(
                native.theta(),
                theta,
                pole_rounding,
                &format!("{projection:?} pole"),
            );
        }
    }
}
