//! The quad-cube kernels: their faces and the domain of the face cross.

use crate::world_coordinates::projection::tests::*;

#[test]
fn cube_projections_match_wcslib_faces() {
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
            let projected = raw(projection, &[])
                .project(NativeCoordinate::from_degrees(phi, theta))
                .unwrap();
            tolerance::assert_close(
                &[projected.x, projected.y],
                &[x, y],
                Tolerance::abs(
                    1e-12,
                    "the face equations, a few ulps of coordinates to 360°",
                ),
                &format!("{projection:?} face ({phi}, {theta})"),
            );
        }
    }
}

#[test]
fn cube_corner_deprojects_to_its_vertex() {
    for projection in [Projection::Tsc, Projection::Csc, Projection::Qsc] {
        let pv = raw(projection, &[]);
        // (315, 45) is the top-right corner of face 4, which is the cube vertex
        // (1, −1, 1)/√3: φ = −45°, θ = atan(1/√2). The tolerance allows the few
        // roundings of the face equations, which differ by projection.
        let corner = pv.deproject(315.0, 45.0).unwrap();
        let vertex_theta = (1.0 / SQRT_2).atan() * R2D;
        tolerance::assert_close(
            &[corner.phi, corner.theta()],
            &[-45.0, vertex_theta],
            Tolerance::abs(
                1e-12,
                "the face equations' roundings, which differ by projection",
            ),
            &format!("{projection:?} corner"),
        );
    }
}
