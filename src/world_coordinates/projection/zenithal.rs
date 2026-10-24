//! The zenithal projections with no parameters of a perspective, `TAN`, `ARC`, `STG`,
//! `ZEA`, `ZPN` and `AIR` (CG 2002 §5.1): radial in the colatitude ζ about the native
//! pole.

use std::f64::consts::PI;

use crate::fits_error::Result;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::{
    Bracket, NewtonEvaluation, ProjectedCoordinate, Projection, solve_newton,
};

/// A zenithal projection's kernel, with its constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Zenithal {
    Tan,
    Arc,
    Stg,
    Zea,
    Zpn(Zpn),
    Air(Air),
}

/// ZPN's polynomial `R(ζ) = Σ Pₘ ζᵐ` and the branch of it the projection uses.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Zpn {
    pub(super) coefficients: [f64; 21],
    pub(super) branch: ZpnBranch,
}

/// The branch of a ZPN polynomial `R(ζ) = Σ Pₘ ζᵐ` that the projection uses: from the
/// native pole out to the first point where `R′` vanishes, past which the radius folds
/// back and two colatitudes share a radius (wcslib `zpnset`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ZpnBranch {
    /// The highest non-zero coefficient's index.
    degree: usize,
    /// The colatitude (rad) where the branch ends: the first zero of `R′`, or π.
    zeta_max: f64,
    /// `R(zeta_max)`, the largest radius the branch reaches.
    radius_max: f64,
}

/// AIR's `K = ln(cos ξ_b)/tan²ξ_b`, `ξ_b = (90° − θ_b)/2`, from `θ_b = PVi_1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Air {
    pub(super) k: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ZpnEvaluation {
    pub(super) value: f64,
    pub(super) derivative: f64,
}

impl Zenithal {
    /// Project a native point: refused beyond its image, TAN past the equator, which
    /// diverges; STG at the antipode of its pole, which it sends to infinity; ZPN past
    /// its inflection; AIR at its antipode. tan ζ = cos θ / sin θ and
    /// tan(ζ/2) = cos θ / (1 + sin θ).
    pub(super) fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let r = match self {
            Self::Tan if sin_theta <= 0.0 => return Err(projection.world_domain_error()),
            Self::Tan => R2D * cos_theta / sin_theta,
            Self::Stg if 1.0 + sin_theta == 0.0 => return Err(projection.world_domain_error()),
            Self::Stg => 2.0 * R2D * cos_theta / (1.0 + sin_theta),
            Self::Arc => R2D * native.zeta(),
            Self::Zea => 2.0 * R2D * (native.zeta() / 2.0).sin(),
            Self::Zpn(zpn) => {
                let zeta = native.zeta();
                if zeta > zpn.branch.zeta_max {
                    return Err(projection.world_domain_error());
                }
                R2D * zpn.evaluate(zeta).value
            }
            Self::Air(air) => {
                let zeta = native.zeta();
                if zeta == PI {
                    return Err(projection.world_domain_error());
                }
                R2D * air.radius(zeta)
            }
        };
        let p = native.phi * D2R;
        projection.projected_coordinate(r * p.sin(), -r * p.cos())
    }

    /// Deproject `(x, y)` (deg): the colatitude ζ (rad) from the radius, per projection.
    pub(super) fn deproject(
        self,
        projection: Projection,
        x: f64,
        y: f64,
    ) -> Result<NativeCoordinate> {
        let r = x.hypot(y);
        let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
        let u = r / R2D;
        let zeta = match self {
            // tan ζ = u gives sin θ and cos θ with no trigonometry.
            Self::Tan => {
                let norm = u.mul_add(u, 1.0).sqrt();
                return Ok(NativeCoordinate::from_sin_cos(
                    projection.native_longitude(phi)?,
                    1.0 / norm,
                    u / norm,
                ));
            }
            Self::Arc => u,
            Self::Zea => 2.0 * projection.checked_asin(u / 2.0)?,
            Self::Stg => 2.0 * (u / 2.0).atan(),
            Self::Zpn(zpn) => zpn.zeta(projection, u)?,
            Self::Air(air) => air.zeta(u)?,
        };
        projection.native_colatitude(phi, zeta)
    }
}

impl Zpn {
    /// Evaluate `Σ Pₘ ζᵐ` and its derivative together with extended Horner.
    pub(super) fn evaluate(&self, zeta: f64) -> ZpnEvaluation {
        evaluate_polynomial(zeta, &self.coefficients)
    }

    /// The colatitude ζ (rad) on the branch whose radius is `u = R/(180/π)`: exact for a
    /// linear or quadratic polynomial, the root nearest the pole; above that, the root
    /// in `[0, ζ_max]`, where `R` rises from `P₀` to `R(ζ_max)`.
    fn zeta(&self, projection: Projection, u: f64) -> Result<f64> {
        /// wcslib's `zpnx2s` tolerance on a radius at the ends of the branch.
        const TOLERANCE: f64 = 1e-13;
        let pv = &self.coefficients;
        let branch = self.branch;
        match branch.degree {
            0 => Err(projection.domain_error()),
            1 => Ok((u - pv[0]) / pv[1]),
            2 => {
                let (a, b, c) = (pv[2], pv[1], pv[0] - u);
                let discriminant = b * b - 4.0 * a * c;
                if discriminant < 0.0 {
                    return Err(projection.domain_error());
                }
                // The root of the larger magnitude, then the other as c/q, which does not
                // cancel where b² ≫ 4ac.
                let q = -0.5 * (b + discriminant.sqrt().copysign(b));
                let (z1, z2) = if q == 0.0 { (0.0, 0.0) } else { (q / a, c / q) };
                let mut zeta = z1.min(z2);
                if zeta < -TOLERANCE {
                    zeta = z1.max(z2);
                }
                if !(-TOLERANCE..=PI + TOLERANCE).contains(&zeta) {
                    return Err(projection.domain_error());
                }
                Ok(zeta.clamp(0.0, PI))
            }
            _ => {
                let (radius_low, radius_high) = (pv[0], branch.radius_max);
                if u < radius_low - TOLERANCE || u > radius_high + TOLERANCE {
                    return Err(projection.domain_error());
                }
                if u <= radius_low {
                    return Ok(0.0);
                }
                if u >= radius_high {
                    return Ok(branch.zeta_max);
                }
                solve_newton(
                    projection,
                    Bracket {
                        negative: 0.0,
                        positive: branch.zeta_max,
                        start: (u - radius_low) / (radius_high - radius_low) * branch.zeta_max,
                    },
                    |zeta| {
                        let evaluation = self.evaluate(zeta);
                        NewtonEvaluation {
                            residual: evaluation.value - u,
                            derivative: evaluation.derivative,
                        }
                    },
                )
            }
        }
    }
}

impl ZpnBranch {
    /// The branch the kernel of a projection other than ZPN carries, which nothing reads.
    pub(super) const UNUSED: Self = Self {
        degree: 0,
        zeta_max: PI,
        radius_max: f64::INFINITY,
    };

    /// The branch of `pv`, or `None` when the polynomial is zero, or of degree two or more
    /// with `P₁ ≤ 0` — falling from the pole, so no branch exists.
    pub(super) fn new(pv: &[f64; 21]) -> Option<Self> {
        let degree = pv.iter().rposition(|&coefficient| coefficient != 0.0)?;
        if degree < 2 {
            return Some(Self {
                degree,
                zeta_max: PI,
                radius_max: evaluate_polynomial(PI, pv).value,
            });
        }
        if pv[1] <= 0.0 {
            return None;
        }
        // The first sign change of R′ on a one-degree grid, then bisected to the
        // precision of the colatitude itself; no change within 180° leaves the whole
        // sphere on the branch.
        let derivative = |zeta: f64| evaluate_polynomial(zeta, pv).derivative;
        let zeta_max = (1..=180)
            .map(|degree| f64::from(degree) * D2R)
            .find(|&zeta| derivative(zeta) <= 0.0)
            .map_or(PI, |above| {
                let (mut low, mut high) = (above - D2R, above);
                while high - low > f64::EPSILON * high {
                    let middle = f64::midpoint(low, high);
                    if derivative(middle) > 0.0 {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                low
            });
        Some(Self {
            degree,
            zeta_max,
            radius_max: evaluate_polynomial(zeta_max, pv).value,
        })
    }
}

impl Air {
    /// The constant of `θ_b` (deg). With `t_b = tan ξ_b`, `ln cos ξ_b = −ln(1 + t_b²)/2`,
    /// so `K = −ln_1p(t_b²)/(2t_b²)`, which keeps its precision as `t_b → 0` and has the
    /// limit `−1/2` there.
    pub(super) fn new(theta_b: f64) -> Self {
        let t_squared = ((90.0 - theta_b) * D2R / 2.0).tan().powi(2);
        let k = if t_squared == 0.0 {
            -0.5
        } else {
            -t_squared.ln_1p() / (2.0 * t_squared)
        };
        Self { k }
    }

    /// The radius `R/(180/π)` at colatitude `ζ` (rad): `−2[ln(cos ξ)/tan ξ + K tan ξ]`,
    /// `ξ = ζ/2`, in `t = tan ξ`.
    pub(super) fn radius(self, zeta: f64) -> f64 {
        self.radius_t((zeta / 2.0).tan())
    }

    /// The radius in `t = tan ξ`: `ln(1 + t²)/t − 2Kt`, with `ln cos ξ = −ln(1 + t²)/2`.
    /// Defined and increasing for every `t ≥ 0`, with slope `1 − 2K` at 0.
    fn radius_t(self, t: f64) -> f64 {
        if t == 0.0 {
            return 0.0;
        }
        (t * t).ln_1p() / t - 2.0 * self.k * t
    }

    /// `d/dt` of [`Self::radius_t`]: `(2t²/(1 + t²) − ln(1 + t²))/t² − 2K`.
    fn radius_t_derivative(self, t: f64) -> f64 {
        if t == 0.0 {
            return 1.0 - 2.0 * self.k;
        }
        let t_squared = t * t;
        (2.0 * t_squared / (1.0 + t_squared) - t_squared.ln_1p()) / t_squared - 2.0 * self.k
    }

    /// The colatitude ζ (rad) of `u = R/(180/π)`, solved in `t = tan(ζ/2)`, where the
    /// radius is close to linear over the whole domain. As `0 ≤ ln(1 + t²)/t ≤ t` and
    /// `K < 0`, `−2Kt ≤ R(t) ≤ (1 − 2K)t`, so `t` lies in `[u/(1 − 2K), u/(−2K)]`, and the
    /// solve starts at the small-angle inverse, the lower end.
    pub(super) fn zeta(self, u: f64) -> Result<f64> {
        let low = u / (1.0 - 2.0 * self.k);
        let bracket = Bracket {
            negative: low,
            positive: u / (-2.0 * self.k),
            start: low,
        };
        let t = solve_newton(Projection::Air, bracket, |t| NewtonEvaluation {
            residual: self.radius_t(t) - u,
            derivative: self.radius_t_derivative(t),
        })?;
        Ok(2.0 * t.atan())
    }
}

/// Evaluate `Σ Pₘ ζᵐ` of `pv` and its derivative together with extended Horner.
pub(super) fn evaluate_polynomial(zeta: f64, pv: &[f64; 21]) -> ZpnEvaluation {
    let mut value = pv[20];
    let mut derivative = 0.0;
    for &coefficient in pv[..20].iter().rev() {
        derivative = derivative * zeta + value;
        value = value * zeta + coefficient;
    }
    ZpnEvaluation { value, derivative }
}
