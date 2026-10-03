//! The native→celestial rotation: the celestial pole and the spherical rotation
//! through it (Calabretta & Greisen 2002, §2).

use crate::error::{FitsError, Result};
use crate::world_coordinates::D2R;
use crate::world_coordinates::R2D;
use crate::world_coordinates::norm180;
use crate::world_coordinates::norm360;
use crate::world_coordinates::projection::NativeCoordinate;
use crate::world_coordinates::{cosd, sind};

/// The rotation from native to celestial coordinates: the celestial pole
/// `(α_p, δ_p)` and the native longitude of the pole `φ_p` (LONPOLE), all degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CelestialPole {
    /// Celestial longitude of the native pole, `α_p` (right ascension in an
    /// equatorial frame).
    pub ra: f64,
    /// Celestial latitude of the native pole, `δ_p`.
    pub dec: f64,
    /// Native longitude of the celestial pole, `φ_p` (`LONPOLE`).
    pub lonpole: f64,
}

/// A celestial coordinate pair in degrees, as [`CelestialPole::to_celestial`] yields.
#[derive(Debug, Clone, Copy)]
pub(super) struct CelestialCoordinate {
    pub(super) ra: f64,
    pub(super) dec: f64,
}

impl CelestialPole {
    /// Compute the celestial pole `(α_p, δ_p, φ_p)` from the fiducial point
    /// `(φ₀, θ₀) → (α₀, δ₀)`, `φ_p` (LONPOLE), and `θ_p` (LATPOLE), as wcslib's `celset`
    /// does (CG 2002 §2.4). Zenithal (`θ₀ = 90°`) reduces to `(α₀, δ₀, φ_p)`.
    ///
    /// Away from the native pole, `sin δ₀ = sin θ₀ sin δ_p + cos θ₀ cos(φ_p − φ₀) cos δ_p`
    /// has the solutions `δ_p = u ± v`, `u = atan2(sin θ₀, cos θ₀ cos(φ_p − φ₀))`,
    /// `v = acos(sin δ₀ / z)`, `z = √(sin²θ₀ + cos²θ₀ cos²(φ_p − φ₀))`. Each is wrapped to
    /// `[−180°, 180°]`; LATPOLE picks the nearer of those within ±90°, and with none
    /// there is no pole. When `z = 0` LATPOLE alone fixes `δ_p`.
    pub(super) fn from_fiducial(
        phi0: f64,
        theta0: f64,
        a0: f64,
        d0: f64,
        phip: f64,
        latpole: f64,
    ) -> Result<CelestialPole> {
        /// wcslib's `celset` tolerance on the pole latitude and on `|sin δ₀ / z|`.
        const TOLERANCE: f64 = 1e-10;
        if theta0 == 90.0 {
            return Ok(CelestialPole {
                ra: a0,
                dec: d0,
                lonpole: phip,
            });
        }
        let (sin_theta0, cos_theta0) = (sind(theta0), cosd(theta0));
        let sin_d0 = sind(d0);
        let (sin_dphi, cos_dphi) = if phip == phi0 {
            (0.0, 1.0)
        } else {
            (sind(phip - phi0), cosd(phip - phi0))
        };
        let x = cos_theta0 * cos_dphi;
        let y = sin_theta0;
        let z = x.hypot(y);
        let dp = if z == 0.0 {
            if sin_d0 != 0.0 {
                return Err(FitsError::WcsInvalidPole {
                    detail: "|φp − φ0| = 90° with θ0 = 0 needs δ0 = 0",
                });
            }
            latpole.clamp(-90.0, 90.0)
        } else {
            let (u, v) = if phip == phi0 {
                (theta0, 90.0 - d0)
            } else {
                let mut ratio = sin_d0 / z;
                if ratio.abs() > 1.0 {
                    if ratio.abs() - 1.0 >= TOLERANCE {
                        return Err(FitsError::WcsInvalidPole {
                            detail: "|δ0| is too large for this φp, φ0 and θ0",
                        });
                    }
                    ratio = ratio.signum();
                }
                (y.atan2(x) * R2D, ratio.acos() * R2D)
            };
            let wrap = |angle: f64| {
                if angle > 180.0 {
                    angle - 360.0
                } else if angle < -180.0 {
                    angle + 360.0
                } else {
                    angle
                }
            };
            let (candidate1, candidate2) = (wrap(u + v), wrap(u - v));
            let valid = |angle: f64| angle.abs() < 90.0 + TOLERANCE;
            let dp = if (latpole - candidate1).abs() < (latpole - candidate2).abs() {
                if valid(candidate1) {
                    candidate1
                } else {
                    candidate2
                }
            } else if valid(candidate2) {
                candidate2
            } else {
                candidate1
            };
            if !valid(dp) {
                return Err(FitsError::WcsInvalidPole {
                    detail: "no pole latitude within ±90° fits φp, φ0 and θ0",
                });
            }
            dp.clamp(-90.0, 90.0)
        };
        let cos_d0 = cosd(d0);
        let z = cosd(dp) * cos_d0;
        let ap = if z.abs() < TOLERANCE {
            if cos_d0.abs() < TOLERANCE {
                // The celestial pole is the fiducial point.
                a0
            } else if dp > 0.0 {
                // The celestial north pole is the native pole.
                a0 + phip - phi0 - 180.0
            } else {
                // The celestial south pole is the native pole.
                a0 - phip + phi0
            }
        } else {
            let x = (sin_theta0 - sind(dp) * sin_d0) / z;
            let y = sin_dphi * cos_theta0 / cos_d0;
            a0 - y.atan2(x) * R2D
        };
        Ok(CelestialPole {
            ra: norm360(ap),
            dec: dp,
            lonpole: phip,
        })
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
