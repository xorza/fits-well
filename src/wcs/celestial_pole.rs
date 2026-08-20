//! The native→celestial rotation: the celestial pole and the spherical rotation
//! through it (Calabretta & Greisen 2002, §2).

use std::f64::consts::FRAC_PI_2;

use crate::wcs::D2R;
use crate::wcs::R2D;
use crate::wcs::norm180;
use crate::wcs::norm360;
use crate::wcs::projection::NativeCoordinate;

/// The rotation from native to celestial coordinates: the celestial pole
/// `(α_p, δ_p)` and the native longitude of the pole `φ_p` (LONPOLE), all degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct CelestialPole {
    pub(super) ra: f64,
    pub(super) dec: f64,
    pub(super) lonpole: f64,
}

/// A celestial coordinate pair in degrees, as [`CelestialPole::to_celestial`] yields.
#[derive(Debug, Clone, Copy)]
pub(super) struct CelestialCoordinate {
    pub(super) ra: f64,
    pub(super) dec: f64,
}

impl CelestialPole {
    /// Compute the celestial pole `(α_p, δ_p, φ_p)` from the fiducial point
    /// `(φ₀, θ₀) → (α₀, δ₀)`, `φ_p` (LONPOLE), and `θ_p` (LATPOLE) (CG 2002 §2.4).
    /// Zenithal (`θ₀ = 90°`) reduces to `(α₀, δ₀, φ_p)`.
    pub(super) fn from_fiducial(
        phi0: f64,
        theta0: f64,
        a0: f64,
        d0: f64,
        phip: f64,
        thetap: f64,
    ) -> CelestialPole {
        if (theta0 - 90.0).abs() < 1e-12 {
            return CelestialPole {
                ra: a0,
                dec: d0,
                lonpole: phip,
            };
        }
        let (t0, d0r) = (theta0 * D2R, d0 * D2R);
        let dphi = (phip - phi0) * D2R;
        // sinδ0 = sinθ0·sinδ_p + cosθ0·cos(φ_p−φ0)·cosδ_p = R·cos(δ_p − β).
        let a = t0.sin();
        let b = t0.cos() * dphi.cos();
        let rmag = a.hypot(b);
        let beta = a.atan2(b);
        let ac = (d0r.sin() / rmag).clamp(-1.0, 1.0).acos();
        // Two δ_p solutions; pick the one in range nearest LATPOLE.
        let c1 = beta + ac;
        let c2 = beta - ac;
        let in_range = |x: f64| (-FRAC_PI_2..=FRAC_PI_2).contains(&x);
        let dpr = match (in_range(c1), in_range(c2)) {
            (true, true) => {
                if (c1 - thetap * D2R).abs() <= (c2 - thetap * D2R).abs() {
                    c1
                } else {
                    c2
                }
            }
            (true, false) => c1,
            (false, true) => c2,
            (false, false) => c1.clamp(-FRAC_PI_2, FRAC_PI_2),
        };
        let dp = dpr * R2D;
        // α_p from the fiducial constraint (inverting eq. 2 at (φ0, θ0)).
        let fphi = (phi0 - phip) * D2R;
        let y = -t0.cos() * fphi.sin();
        let x = t0.sin() * dpr.cos() - t0.cos() * dpr.sin() * fphi.cos();
        let ap = a0 - y.atan2(x) * R2D;
        CelestialPole {
            ra: norm360(ap),
            dec: dp,
            lonpole: phip,
        }
    }

    /// Native spherical (φ, θ) → celestial (α, δ), all degrees (CG 2002 eq. 2).
    pub(super) fn to_celestial(self, phi: f64, theta: f64) -> CelestialCoordinate {
        let CelestialPole {
            ra: ap,
            dec: dp,
            lonpole: fp,
        } = self;
        let (tr, dpr, dphi) = (theta * D2R, dp * D2R, (phi - fp) * D2R);
        let sin_d = tr.sin() * dpr.sin() + tr.cos() * dpr.cos() * dphi.cos();
        let dec = sin_d.clamp(-1.0, 1.0).asin() * R2D;
        let y = -tr.cos() * dphi.sin();
        let x = tr.sin() * dpr.cos() - tr.cos() * dpr.sin() * dphi.cos();
        CelestialCoordinate {
            ra: norm360(ap + y.atan2(x) * R2D),
            dec,
        }
    }

    /// Celestial (α, δ) → native spherical (φ, θ), all degrees (CG 2002 eq. 5).
    pub(super) fn to_native(self, ra: f64, dec: f64) -> NativeCoordinate {
        let CelestialPole {
            ra: ap,
            dec: dp,
            lonpole: fp,
        } = self;
        let (dr, dpr, dalpha) = (dec * D2R, dp * D2R, (ra - ap) * D2R);
        let sin_t = dr.sin() * dpr.sin() + dr.cos() * dpr.cos() * dalpha.cos();
        let theta = sin_t.clamp(-1.0, 1.0).asin() * R2D;
        let y = -dr.cos() * dalpha.sin();
        let x = dr.sin() * dpr.cos() - dr.cos() * dpr.sin() * dalpha.cos();
        NativeCoordinate {
            phi: norm180(fp + y.atan2(x) * R2D),
            theta,
        }
    }
}
