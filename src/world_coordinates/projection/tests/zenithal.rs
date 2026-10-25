//! The zenithal kernels: the polynomial, the perspective-free forms and their digits
//! next to the pole and the centre.

use crate::world_coordinates::projection::tests::*;

#[test]
fn zpn_extended_horner_evaluates_value_and_derivative() {
    let evaluation = evaluate_polynomial(2.0, &pv(&[2.0, -3.0, 4.0, 5.0]));
    // P(2) = 2 - 3·2 + 4·2² + 5·2³ = 52; P'(2) = -3 + 8·2 + 15·2² = 73.
    assert_eq!(evaluation.value, 52.0);
    assert_eq!(evaluation.derivative, 73.0);
}

/// At x = 1e-9° from the centre σ ≈ 1.5e-22: the root (−b − √d)/2a cancels to 0 there,
/// while c/q keeps it, so cos θ = √(σ(2 − σ)) is the radius x/(180/π) itself.
#[test]
fn sin_keeps_its_digits_next_to_the_centre() {
    let x = 1e-9;
    let native = raw(Projection::Sin, &[]).deproject(x, 0.0).unwrap();
    tolerance::assert_within(
        native.cos_theta(),
        x / R2D,
        Tolerance::ulps(
            2.0,
            "the radius to radians and the cosine kept as its own sine",
        ),
        "SIN next to the centre",
    );
}

/// Next to the native pole the colatitude crosses each projection as an `f64` of its
/// own, so a round trip keeps ζ to an ulp each way, where θ in degrees rounds at
/// 1.4e-14°, 2.5e-16 rad: once all of ζ = 1e-15. The sweep holds each formula that
/// carries ζ: the zenithals from ζ itself, and SIN and SZP from their depth
/// σ = 1 − sin θ.
#[test]
fn next_to_the_pole_the_colatitude_keeps_its_digits() {
    use Projection::*;
    let cases: &[(Projection, &[f64])] = &[
        (Tan, &[]),
        (Sin, &[]),
        (Arc, &[]),
        (Stg, &[]),
        (Zea, &[]),
        (Zpn, &[0.0, 1.0, 0.0, 0.1]),
        (Szp, &[]),
    ];
    for &(projection, pv) in cases {
        let pv = raw(projection, pv);
        for zeta in [1e-15, 1e-9, 1e-6, 1e-3] {
            let native = NativeCoordinate::from_colatitude(30.0, zeta);
            let projected = pv.project(native).unwrap();
            let back = pv.deproject(projected.x, projected.y).unwrap();
            tolerance::assert_within(
                back.zeta(),
                zeta,
                Tolerance::ulps(2.0, "the radius and its inverse, an ulp each"),
                &format!("{projection:?} ζ = {zeta:e}"),
            );
        }
    }
}

/// The ZPN inverse stays on the branch inside the first point of inflection, where it
/// inverts the forward map; `plane_points_without_a_world_point_are_refused` holds the
/// radius past the polynomial's maximum.
#[test]
fn zpn_inverts_only_inside_its_inflection() {
    let cubic = pv(&[0.0, 1.0, 0.0, -0.5]);
    let parameters = Projection::Zpn.parameters(cubic).unwrap();
    for theta in [80.0, 60.0, 45.0] {
        let projected = parameters
            .project(NativeCoordinate::from_degrees(0.0, theta))
            .unwrap();
        let native = parameters.deproject(projected.x, projected.y).unwrap();
        // The radius carries an ulp of rounding, which ζ takes divided by R′: at 45°,
        // R′ = 1 − 1.5ζ² = 0.075 near the peak, and the inverse is good to 9 ulps there.
        let zeta = NativeCoordinate::from_degrees(0.0, theta).zeta();
        let radius = evaluate_polynomial(zeta, &cubic);
        let conditioned = 2.0 * f64::EPSILON * (radius.value / radius.derivative + zeta);
        tolerance::assert_within(
            native.zeta(),
            zeta,
            Tolerance::abs(
                conditioned,
                "two roundings, conditioned by the radius over its slope",
            ),
            &format!("ZPN θ = {theta}"),
        );
    }
    // A quadratic whose ζ² term is 1e-12 of its linear one: the root (−b + √d)/2a
    // cancels to about 1e-4 of ζ there, c/q keeps it to an ulp.
    let quadratic = Projection::Zpn.parameters(pv(&[0.0, 1.0, 1e-12])).unwrap();
    for zeta in [1e-9, 0.5, 2.0] {
        let projected = quadratic
            .project(NativeCoordinate::from_colatitude(0.0, zeta))
            .unwrap();
        let native = quadratic.deproject(projected.x, projected.y).unwrap();
        tolerance::assert_within(
            native.zeta(),
            zeta,
            Tolerance::ulps(2.0, "the quadratic's root and its radius"),
            &format!("ζ = {zeta}"),
        );
    }
    // Degenerate polynomials have no inverse at all.
    assert_matches!(
        Projection::Zpn.parameters([0.0; 21]),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection {
                projection: Projection::Zpn
            }
        })
    );
    let falling = pv(&[0.0, -1.0, 0.5]);
    assert_matches!(
        Projection::Zpn.parameters(falling),
        Err(FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection {
                projection: Projection::Zpn
            }
        })
    );
}
