//! The native→celestial rotation: the celestial pole and the spherical rotation
//! through it (Calabretta & Greisen 2002, §2).

use crate::fits_error::{FitsError, PoleDefect, Result};
use crate::world_coordinates::angle;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::celestial_coordinate::CelestialCoordinate;

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

/// The fiducial point of a projection: its native coordinates `(φ₀, θ₀)` and the
/// celestial ones `(α₀, δ₀)` it maps to, all degrees.
#[derive(Debug, Clone, Copy)]
pub(super) struct Fiducial {
    pub(super) phi: f64,
    pub(super) theta: f64,
    pub(super) celestial: CelestialCoordinate,
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
    pub(super) fn from_fiducial(fiducial: Fiducial, phip: f64, latpole: f64) -> Result<Self> {
        /// wcslib's `celset` tolerance on the pole latitude and on `|sin δ₀ / z|`.
        const TOLERANCE: f64 = 1e-10;
        let Fiducial {
            phi: phi0,
            theta: theta0,
            celestial: CelestialCoordinate { ra: a0, dec: d0 },
        } = fiducial;
        if theta0 == 90.0 {
            return Ok(Self {
                ra: a0,
                dec: d0,
                lonpole: phip,
            });
        }
        let (sin_theta0, cos_theta0) = (angle::sind(theta0), angle::cosd(theta0));
        let sin_d0 = angle::sind(d0);
        let (sin_dphi, cos_dphi) = if phip == phi0 {
            (0.0, 1.0)
        } else {
            (angle::sind(phip - phi0), angle::cosd(phip - phi0))
        };
        let x = cos_theta0 * cos_dphi;
        let y = sin_theta0;
        let z = x.hypot(y);
        let dp = if z == 0.0 {
            if sin_d0 != 0.0 {
                return Err(FitsError::WcsInvalidPole {
                    defect: PoleDefect::EquatorialFiducial,
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
                            defect: PoleDefect::FiducialLatitude,
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
                    defect: PoleDefect::NoLatitude,
                });
            }
            dp.clamp(-90.0, 90.0)
        };
        let cos_d0 = angle::cosd(d0);
        let z = angle::cosd(dp) * cos_d0;
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
            let x = (sin_theta0 - angle::sind(dp) * sin_d0) / z;
            let y = sin_dphi * cos_theta0 / cos_d0;
            a0 - y.atan2(x) * R2D
        };
        Ok(Self {
            ra: angle::norm360(ap),
            dec: dp,
            lonpole: phip,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::fits_error::{FitsError, PoleDefect};
    use crate::world_coordinates::celestial_coordinate::CelestialCoordinate;
    use crate::world_coordinates::celestial_pole::{CelestialPole, Fiducial};
    use std::assert_matches;

    /// The fiducial point `(φ₀, θ₀) = (0, theta)` at `(α₀, δ₀) = (45°, dec)`.
    fn fiducial(theta: f64, dec: f64) -> Fiducial {
        Fiducial {
            phi: 0.0,
            theta,
            celestial: CelestialCoordinate { ra: 45.0, dec },
        }
    }

    /// A zenithal fiducial point is the native pole. A cylindrical one at (φ₀, θ₀) = (0, 0)
    /// with `φ_p = φ₀` puts the native pole 90° along the meridian from (α₀, δ₀) =
    /// (45°, 30°): δ_p = 0 ± (90° − 30°), LATPOLE 90° picks 60° and α_p = 45° − 180°, and
    /// LATPOLE −90° picks −60° and α_p = 45°.
    #[test]
    fn pole_follows_the_fiducial_point_and_latpole() {
        let pole = |theta0, phip, latpole| {
            CelestialPole::from_fiducial(fiducial(theta0, 30.0), phip, latpole).unwrap()
        };
        assert_eq!(
            pole(90.0, 180.0, 90.0),
            CelestialPole {
                ra: 45.0,
                dec: 30.0,
                lonpole: 180.0
            }
        );
        assert_eq!(
            pole(0.0, 0.0, 90.0),
            CelestialPole {
                ra: 225.0,
                dec: 60.0,
                lonpole: 0.0
            }
        );
        assert_eq!(
            pole(0.0, 0.0, -90.0),
            CelestialPole {
                ra: 45.0,
                dec: -60.0,
                lonpole: 0.0
            }
        );
    }

    /// θ₀ = 0 with φ_p − φ₀ = 90° gives z = 0, where only δ₀ = 0 has a pole, and LATPOLE
    /// is its latitude. With φ_p − φ₀ = 60°, z = cos 60° = 0.5 and sin 45° / z = √2 has no
    /// arccosine. With φ_p − φ₀ = 180°, u = 180° and v = 60°, so both 240° → −120° and
    /// 120° lie past ±90°.
    #[test]
    fn fiducial_point_without_a_pole_is_refused() {
        let pole =
            |d0, phip, latpole| CelestialPole::from_fiducial(fiducial(0.0, d0), phip, latpole);
        assert_matches!(
            pole(30.0, 90.0, 90.0),
            Err(FitsError::WcsInvalidPole {
                defect: PoleDefect::EquatorialFiducial
            })
        );
        assert_eq!(pole(0.0, 90.0, 40.0).unwrap().dec, 40.0);
        assert_matches!(
            pole(45.0, 60.0, 90.0),
            Err(FitsError::WcsInvalidPole {
                defect: PoleDefect::FiducialLatitude
            })
        );
        assert_matches!(
            pole(30.0, 180.0, 90.0),
            Err(FitsError::WcsInvalidPole {
                defect: PoleDefect::NoLatitude
            })
        );
    }
}
