//! [`NativeCoordinate`]: a point on the native sphere, between the rotation and the
//! projection.

use crate::world_coordinates::angle;
use crate::world_coordinates::angle::R2D;

/// A point on the native sphere: the longitude φ in degrees, and the latitude θ as its
/// sine and cosine. The pair holds a point next to the native pole to the precision an
/// `f64` gives its distance from that pole, where θ in degrees rounds to 1.4e-14°.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct NativeCoordinate {
    pub(super) phi: f64,
    sin_theta: f64,
    cos_theta: f64,
}

impl NativeCoordinate {
    /// The point at longitude `phi` and latitude `theta`, both degrees: exact at the
    /// poles and on the equator.
    pub(super) fn from_degrees(phi: f64, theta: f64) -> Self {
        debug_assert!((-90.0..=90.0).contains(&theta), "a latitude: {theta}");
        Self {
            phi,
            sin_theta: angle::sind(theta),
            cos_theta: angle::cosd(theta),
        }
    }

    /// The point at longitude `phi` (degrees) and colatitude `zeta` (radians).
    pub(super) fn from_colatitude(phi: f64, zeta: f64) -> Self {
        let (sin_zeta, cos_zeta) = zeta.sin_cos();
        Self {
            phi,
            sin_theta: cos_zeta,
            cos_theta: sin_zeta,
        }
    }

    /// The point at longitude `phi` (degrees) whose latitude has the sine `sin_theta`
    /// and the cosine `cos_theta`, the components of a unit vector, which need not hold
    /// `sin² + cos² = 1` to the last bit.
    pub(super) fn from_sin_cos(phi: f64, sin_theta: f64, cos_theta: f64) -> Self {
        debug_assert!(
            cos_theta.is_nan() || cos_theta >= 0.0,
            "a latitude in [−90°, 90°] has cos θ ≥ 0: {cos_theta}"
        );
        Self {
            phi,
            sin_theta,
            cos_theta,
        }
    }

    pub(super) const fn sin_theta(self) -> f64 {
        self.sin_theta
    }

    pub(super) const fn cos_theta(self) -> f64 {
        self.cos_theta
    }

    /// The latitude θ in degrees: exactly ±90° where `cos θ` is 0.
    pub(super) fn theta(self) -> f64 {
        if self.cos_theta == 0.0 {
            90.0_f64.copysign(self.sin_theta)
        } else {
            self.sin_theta.atan2(self.cos_theta) * R2D
        }
    }

    /// The colatitude ζ = 90° − θ in radians, well conditioned at both poles.
    pub(super) fn zeta(self) -> f64 {
        self.cos_theta.atan2(self.sin_theta)
    }

    pub(super) const fn is_finite(self) -> bool {
        self.phi.is_finite() && self.sin_theta.is_finite() && self.cos_theta.is_finite()
    }
}
