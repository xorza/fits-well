//! The perspective kernels: `AZP`'s inverse root.

use crate::world_coordinates::projection::tests::*;

/// The AZP inverse takes each root past the native pole round by −360° and keeps the
/// larger, as wcslib's `azpx2s` does: with μ = 0.03 and γ = −56.46°, the point the forward
/// gives for (φ, θ) = (−179.03°, −54.89°) deprojects back to it, where the root nearest
/// the pole was refused (of a 1° grid, 7280 points failed so).
#[test]
fn azp_inverse_takes_the_root_azpx2s_takes() {
    let mut pv = Projection::Azp.parameter_defaults();
    pv[1] = 0.03;
    pv[2] = -56.46;
    let parameters = Projection::Azp.parameters(pv).unwrap();
    let native = NativeCoordinate::from_degrees(-179.03, -54.89);
    let projected = parameters.project(native).unwrap();
    let back = parameters.deproject(projected.x, projected.y).unwrap();
    tolerance::assert_close(
        &[back.phi, back.theta()],
        &[-179.03, -54.89],
        Tolerance::abs(
            1e-9,
            "a few roundings of the forward and the inverse, in degrees",
        ),
        "AZP round trip",
    );
}
