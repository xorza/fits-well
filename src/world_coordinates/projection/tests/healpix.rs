//! The HEALPix kernel: its poles and parameters.

use crate::world_coordinates::projection::tests::*;

#[test]
fn hpx_poles_and_parameters() {
    let default_pv = raw(Projection::Hpx, &[0.0, 4.0, 3.0]);
    let north = default_pv
        .project(NativeCoordinate::from_degrees(0.0, 90.0))
        .unwrap();
    let south = default_pv
        .project(NativeCoordinate::from_degrees(100.0, -90.0))
        .unwrap();
    assert_eq!([north.x, north.y], [45.0, 90.0]);
    assert_eq!([south.x, south.y], [135.0, -90.0]);
    assert_eq!(default_pv.deproject(45.0, 90.0).unwrap().theta(), 90.0);

    // In the polar cap 1 − sin θ = σ²/K. At y = 90° − 45°·2⁻³⁰, exact in f64, σ = 2⁻³⁰,
    // and sin θ = 1 − 2⁻⁶⁰/3 rounds to 1, which once gave θ = 90°; cos θ =
    // √(σ²/K·(2 − σ²/K)) = 2⁻³⁰·√(2/3) to 1e-19 keeps the point.
    let sigma = 2f64.powi(-30);
    let cap = default_pv.deproject(45.0, 90.0 - 45.0 * sigma).unwrap();
    let expected = sigma * (2.0f64 / 3.0).sqrt();
    tolerance::assert_within(
        cap.cos_theta(),
        expected,
        Tolerance::ulps(2.0, "the cap's square root and product"),
        "polar cap",
    );

    let standard = default_pv
        .project(NativeCoordinate::from_degrees(20.0, -70.0))
        .unwrap();
    let alternate = raw(Projection::Hpx, &[0.0, 3.0, 4.0])
        .project(NativeCoordinate::from_degrees(20.0, -70.0))
        .unwrap();
    assert_ne!([standard.x, standard.y], [alternate.x, alternate.y]);
}
