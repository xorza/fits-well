//! [`NativeRotation`]: the spherical rotation between native and celestial
//! coordinates (Calabretta & Greisen 2002, §2).

use crate::world_coordinates::angle;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::celestial_coordinate::CelestialCoordinate;
use crate::world_coordinates::celestial_pole::CelestialPole;
use crate::world_coordinates::native_coordinate::NativeCoordinate;

/// The rotation through a [`CelestialPole`] (CG 2002 eqs. 2 and 5), with the trig of
/// the pole's latitude computed once. Each direction takes the latitude by `atan2` from
/// the components of the rotated unit vector: `asin` of its third component alone
/// moves the latitude by ε / cos θ for an error ε, which near a pole is far more than
/// the offset an `f64` holds.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct NativeRotation {
    pub(super) pole: CelestialPole,
    sin_dec: f64,
    cos_dec: f64,
}

impl From<CelestialPole> for NativeRotation {
    fn from(pole: CelestialPole) -> Self {
        Self {
            pole,
            sin_dec: angle::sind(pole.dec),
            cos_dec: angle::cosd(pole.dec),
        }
    }
}

impl NativeRotation {
    /// Native (φ, θ) → celestial (α, δ), in degrees (CG 2002 eq. 2).
    pub(super) fn to_celestial(self, native: NativeCoordinate) -> CelestialCoordinate {
        let (sin_dphi, cos_dphi) = ((native.phi - self.pole.lonpole) * D2R).sin_cos();
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let x = sin_theta * self.cos_dec - cos_theta * self.sin_dec * cos_dphi;
        let y = -cos_theta * sin_dphi;
        let z = sin_theta * self.sin_dec + cos_theta * self.cos_dec * cos_dphi;
        // x and y are components of a unit vector, so √(x² + y²) cannot overflow and
        // needs no `hypot`.
        CelestialCoordinate {
            ra: angle::norm360(self.pole.ra + y.atan2(x) * R2D),
            dec: z.atan2(x.mul_add(x, y * y).sqrt()) * R2D,
        }
    }

    /// Celestial (α, δ) → native (φ, θ), in degrees (CG 2002 eq. 5); `dec` lies in
    /// [−90°, 90°].
    pub(super) fn to_native(self, ra: f64, dec: f64) -> NativeCoordinate {
        debug_assert!((-90.0..=90.0).contains(&dec), "a latitude: {dec}");
        // Δα's sine and cosine from its half angle, which the identity below needs too.
        let (sin_half, cos_half) = ((ra - self.pole.ra) * D2R / 2.0).sin_cos();
        let sin_dalpha = 2.0 * sin_half * cos_half;
        let cos_dalpha = (cos_half - sin_half) * (cos_half + sin_half);
        // A latitude's multiples of 90° are 0 and the poles, where cos δ is exactly 0.
        let (sin_d, cos_d) = if dec.abs() == 90.0 {
            (dec.signum(), 0.0)
        } else {
            (dec * D2R).sin_cos()
        };
        // sin δ cos δp − cos δ sin δp cos Δα as sin(δ − δp) + 2 cos δ sin δp sin²(Δα/2):
        // both terms vanish at the native pole, so a point next to it keeps its digits.
        let x = angle::sind(dec - self.pole.dec) + 2.0 * cos_d * self.sin_dec * sin_half * sin_half;
        let y = -cos_d * sin_dalpha;
        let z = sin_d * self.sin_dec + cos_d * self.cos_dec * cos_dalpha;
        NativeCoordinate::from_sin_cos(
            angle::norm180(self.pole.lonpole + y.atan2(x) * R2D),
            z,
            x.mul_add(x, y * y).sqrt(),
        )
    }
}
