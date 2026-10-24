//! The projections whose fiducial point lies on the native equator, `θ₀ = 0`: the
//! cylindrical `CAR`, `CEA`, `MER` and `CYP`, the pseudo-cylindrical `SFL`, `PAR`, `AIT`
//! and `MOL`, the polyconic `PCO` and Bonne's `BON` (CG 2002 §5.2, §5.3, §5.5, §5.6).

use crate::world_coordinates::angle;
use std::f64::consts::{FRAC_1_SQRT_2, FRAC_PI_2, FRAC_PI_4, PI, SQRT_2};

use crate::fits_error::Result;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::DEGREE_TOLERANCE;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::angle::UNIT_TOLERANCE;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::{
    Bracket, NewtonEvaluation, ProjectedCoordinate, Projection, solve_newton,
};

/// An equatorial projection's kernel, with its constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Equatorial {
    Car,
    /// `λ = PVi_1`.
    Cea {
        lambda: f64,
    },
    Mer,
    Sfl,
    Ait,
    Mol,
    /// `μ = PVi_1`, `λ = PVi_2`.
    Cyp {
        mu: f64,
        lambda: f64,
    },
    Par,
    Bon(Bon),
    Pco,
}

/// Bonne's `θ₁ = PVi_1`, which at 0 makes it SFL, and the apex offset
/// `Y₀ = θ₁ + cot θ₁` (rad) of its cone.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Bon {
    theta1: f64,
    y0: f64,
}

impl Bon {
    pub(super) fn new(theta1_degrees: f64) -> Self {
        let theta1 = theta1_degrees * D2R;
        Self {
            theta1: theta1_degrees,
            y0: theta1 + 1.0 / theta1.tan(),
        }
    }
}

impl Equatorial {
    pub(super) fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let phi = native.phi;
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let [x, y] = match self {
            Self::Car => [phi, native.theta()],
            Self::Cea { lambda } => [phi, R2D * sin_theta / lambda],
            // The poles are at infinity.
            Self::Mer if cos_theta == 0.0 => return Err(projection.world_domain_error()),
            // ln tan(45° + θ/2) as asinh(tan θ), which does not cancel near the equator.
            Self::Mer => [phi, R2D * (sin_theta / cos_theta).asinh()],
            Self::Sfl => [phi * cos_theta, native.theta()],
            Self::Ait => {
                let pr = phi * D2R;
                let gamma = R2D * (2.0 / (1.0 + cos_theta * (pr / 2.0).cos())).sqrt();
                [
                    2.0 * gamma * cos_theta * (pr / 2.0).sin(),
                    gamma * sin_theta,
                ]
            }
            Self::Mol => {
                let gamma = mollweide_auxiliary(native)?;
                [
                    (2.0 * SQRT_2 / PI) * phi * gamma.cos,
                    SQRT_2 * R2D * gamma.sin,
                ]
            }
            // The latitude whose rays run parallel to the cylinder.
            Self::Cyp { mu, .. } if mu + cos_theta == 0.0 => {
                return Err(projection.world_domain_error());
            }
            Self::Cyp { mu, lambda } => [
                lambda * phi,
                R2D * (mu + lambda) * sin_theta / (mu + cos_theta),
            ],
            Self::Par => {
                let t = native.theta() * D2R;
                [
                    phi * (2.0 * (2.0 * t / 3.0).cos() - 1.0),
                    180.0 * (t / 3.0).sin(),
                ]
            }
            // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0.
            Self::Bon(bon) if bon.theta1 == 0.0 => [phi * cos_theta, native.theta()],
            Self::Bon(bon) => {
                let r = bon.y0 - native.theta() * D2R;
                let aphi = phi * D2R * cos_theta / r;
                [R2D * r * aphi.sin(), R2D * (bon.y0 - r * aphi.cos())]
            }
            Self::Pco if sin_theta == 0.0 => [phi, 0.0],
            Self::Pco => {
                // 1 − cos ω as 2·sin²(ω/2), which keeps its precision as θ → 0, where
                // cot θ grows and the difference would cancel.
                let omega = phi * D2R * sin_theta;
                let cot = cos_theta / sin_theta;
                [
                    R2D * cot * omega.sin(),
                    native.theta() + R2D * cot * 2.0 * (omega / 2.0).sin().powi(2),
                ]
            }
        };
        projection.projected_coordinate(x, y)
    }

    pub(super) fn deproject(
        self,
        projection: Projection,
        x: f64,
        y: f64,
    ) -> Result<NativeCoordinate> {
        let [phi, theta] = match self {
            Self::Car => [x, y],
            // θ = asin(λ·y/(180/π)).
            Self::Cea { lambda } => [x, projection.checked_asin(lambda * y / R2D)? * R2D],
            // The Gudermannian: no cancellation near the equator, as 2 atan(eʸ) − 90°
            // has.
            Self::Mer => [x, (y / R2D).sinh().atan() * R2D],
            Self::Sfl => [sfl_longitude(projection, x, y)?, y],
            // Hammer–Aitoff inverse (CG 2002 eq. 51). The map's edge, the image of
            // φ = ±180°, is z² = 1/2, and the native poles lie on it; z² within rounding
            // of the edge is the edge, as wcslib's `aitx2s` takes it, where both atan2
            // arguments vanish at a pole and φ is 0, as in SFL.
            Self::Ait => {
                let (u, v) = (x * D2R, y * D2R);
                let z2 = 1.0 - (u / 4.0).powi(2) - (v / 2.0).powi(2);
                if z2.is_nan() || z2 < 0.5 - UNIT_TOLERANCE {
                    return Err(projection.domain_error());
                }
                let z2 = z2.max(0.5);
                let z = z2.sqrt();
                let (sine, cosine) = (z * u / 2.0, 2.0 * z2 - 1.0);
                let phi = if sine == 0.0 && cosine == 0.0 {
                    0.0
                } else {
                    2.0 * sine.atan2(cosine) * R2D
                };
                [phi, projection.checked_asin(v * z)? * R2D]
            }
            // Mollweide inverse (CG 2002 eq. 55), with sin γ = y/(√2·180/π). Within 45° of
            // the equator, θ from γ; nearer a pole, from β = π/2 − |γ|, where
            // 1 − sin|γ| = 2 sin²(β/2) is exact, and the colatitude c from
            // h(2β) = 2π sin²(c/2), as `mollweide_auxiliary` solves it.
            Self::Mol => {
                let sin_gamma = y / (SQRT_2 * R2D);
                // wcslib's `molx2s` pole cutoff on cos γ: x must vanish there.
                let longitude = |cos_gamma: f64| {
                    if cos_gamma < 1e-12 {
                        0.0
                    } else {
                        PI * x / (2.0 * SQRT_2 * cos_gamma)
                    }
                };
                if sin_gamma.abs() <= FRAC_1_SQRT_2 {
                    let gamma = sin_gamma.asin();
                    let theta =
                        projection.checked_asin((2.0 * gamma + (2.0 * gamma).sin()) / PI)? * R2D;
                    [longitude(gamma.cos()), theta]
                } else {
                    let gap = 1.0 - sin_gamma.abs();
                    if gap.is_nan() || gap < -UNIT_TOLERANCE {
                        return Err(projection.domain_error());
                    }
                    let beta = 2.0 * (gap.max(0.0) / 2.0).sqrt().asin();
                    let colatitude =
                        2.0 * projection.checked_asin((h(2.0 * beta) / (2.0 * PI)).sqrt())?;
                    return Ok(NativeCoordinate::from_sin_cos(
                        projection.native_longitude(longitude(beta.sin()))?,
                        colatitude.cos().copysign(sin_gamma),
                        colatitude.sin(),
                    ));
                }
            }
            // φ = x/λ; θ from η = (y/(180/π))/(μ+λ).
            Self::Cyp { mu, lambda } => {
                let eta = (y / R2D) / (mu + lambda);
                let theta = eta.atan2(1.0)
                    + projection.checked_asin(eta * mu / (1.0 + eta * eta).sqrt())?;
                [x / lambda, theta * R2D]
            }
            // PAR inverse (CG 2002 eq. 49).
            Self::Par => {
                let theta = 3.0 * projection.checked_asin(y / 180.0)?;
                [x / (2.0 * (2.0 * theta / 3.0).cos() - 1.0), theta * R2D]
            }
            // Polyconic inverse (CG 2002 §5.6.1): Newton on
            // f(θ) = X² + (Y−θ)² − 2(Y−θ)cotθ = 0, then recover φ; at the equator, wcslib's
            // `pcox2s` cutoff on y/r₀ (radians).
            Self::Pco if (y * D2R).abs() < 1e-12 => [x, 0.0],
            Self::Pco => {
                let (xr, yr) = (x * D2R, y * D2R);
                let th = pco_theta(xr, yr)?;
                let d = yr - th;
                let tanth = th.tan();
                let omega = (xr * tanth).atan2(1.0 - d * tanth);
                [omega / th.sin() * R2D, th * R2D]
            }
            // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0 (avoiding the
            // `1/tan 0` singularity below).
            Self::Bon(bon) if bon.theta1 == 0.0 => [sfl_longitude(projection, x, y)?, y],
            // Bonne's pseudoconic inverse (CG 2002 §5.5.1).
            Self::Bon(bon) => {
                let s = bon.theta1.signum();
                let yc = bon.y0 - y * D2R;
                let r = s * (x * D2R).hypot(yc);
                let tr = bon.y0 - r;
                let aphi = (s * x * D2R).atan2(s * yc);
                [aphi * r / tr.cos() * R2D, tr * R2D]
            }
        };
        projection.native_coordinate(phi, theta)
    }
}

/// The SFL longitude `x / cos y` of a point at `(x, y)` (deg): at a pole, where the
/// meridians meet, `x` must be 0 and φ is 0, as wcslib's `sflx2s` takes it.
fn sfl_longitude(projection: Projection, x: f64, y: f64) -> Result<f64> {
    let cos_y = angle::cosd(y);
    if cos_y == 0.0 {
        if x.abs() > DEGREE_TOLERANCE {
            return Err(projection.domain_error());
        }
        return Ok(0.0);
    }
    Ok(x / cos_y)
}

/// The sine and cosine of an angle.
#[derive(Debug, Clone, Copy)]
struct SinCos {
    sin: f64,
    cos: f64,
}

/// The Mollweide auxiliary angle γ of a native point, `2γ + sin 2γ = π sin θ`. Within
/// 45° of the equator γ itself is solved, which `|γ| < π/4` brackets. Nearer a pole,
/// π sin θ rounds the colatitude c away, and γ's double root at the pole leaves Newton
/// linear, so the solve is for β = π/2 − |γ| ∈ [0, π/2]:
/// `h(2β) = 2π sin²(c/2) = π cos²θ/(1 + |sin θ|)`, with `h(u) = u − sin u`, from the
/// small-β root `(3/4 · π cos²θ/(1 + |sin θ|))^(1/3)` of `h(2β) ≈ 4β³/3`, and
/// cos γ = sin β keeps its digits.
fn mollweide_auxiliary(native: NativeCoordinate) -> Result<SinCos> {
    let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
    if sin_theta.abs() <= FRAC_1_SQRT_2 {
        let target = PI * sin_theta;
        let bracket = Bracket {
            negative: -FRAC_PI_4,
            positive: FRAC_PI_4,
            start: sin_theta.asin(),
        };
        let gamma = solve_newton(Projection::Mol, bracket, |gamma| NewtonEvaluation {
            residual: 2.0 * gamma + (2.0 * gamma).sin() - target,
            derivative: 2.0 + 2.0 * (2.0 * gamma).cos(),
        })?;
        return Ok(SinCos {
            sin: gamma.sin(),
            cos: gamma.cos(),
        });
    }
    let target = PI * cos_theta * cos_theta / (1.0 + sin_theta.abs());
    let bracket = Bracket {
        negative: 0.0,
        positive: FRAC_PI_2,
        start: (0.75 * target).cbrt(),
    };
    let beta = solve_newton(Projection::Mol, bracket, |beta| NewtonEvaluation {
        residual: h(2.0 * beta) - target,
        derivative: 4.0 * beta.sin().powi(2),
    })?;
    Ok(SinCos {
        sin: beta.cos().copysign(sin_theta),
        cos: beta.sin(),
    })
}

/// `u − sin u` for `u ∈ [0, π]`. Below 1, where the difference cancels, its Taylor series
/// `Σ (−1)ᵏ⁺¹ u²ᵏ⁺¹/(2k + 1)!` for k = 1…9, whose first omitted term, `u²¹/21!`, is
/// 1e-19 of the sum at 1.
fn h(u: f64) -> f64 {
    /// `1/(2k + 1)!` for k = 1…9.
    const COEFFICIENTS: [f64; 9] = {
        let mut coefficients = [0.0; 9];
        let mut factorial = 6.0;
        let mut k = 0;
        while k < coefficients.len() {
            coefficients[k] = 1.0 / factorial;
            let next = 2.0 * k as f64 + 4.0;
            factorial *= next * (next + 1.0);
            k += 1;
        }
        coefficients
    };
    if u >= 1.0 {
        return u - u.sin();
    }
    let u2 = u * u;
    let series = COEFFICIENTS
        .iter()
        .rev()
        .fold(0.0, |sum, &coefficient| coefficient - u2 * sum);
    u * u2 * series
}

/// Solve `X² + (Y − θ)² − 2(Y − θ) cot θ = 0` for the polyconic latitude (rad), which
/// lies between 0 and `Y`, as wcslib's `pcox2s` brackets it: `Y = θ + cot θ (1 − cos ω)`
/// is past θ on the side of the equator θ is on. The residual tends to −∞ toward 0 and
/// is `X² ≥ 0` at `Y`.
fn pco_theta(x: f64, y: f64) -> Result<f64> {
    let bracket = Bracket {
        negative: 0.0,
        positive: y,
        start: y,
    };
    solve_newton(Projection::Pco, bracket, |theta| {
        let delta = y - theta;
        let cotangent = 1.0 / theta.tan();
        NewtonEvaluation {
            residual: x * x + delta * delta - 2.0 * delta * cotangent,
            derivative: -2.0 * delta + 2.0 * cotangent + 2.0 * delta / theta.sin().powi(2),
        }
    })
}

#[cfg(test)]
mod tests {
    use crate::world_coordinates::projection::equatorial::h;

    /// The series meets `u − sin u` at 1, where the closed form loses at most
    /// `1/h(1) ≈ 6.3` ulps of h to cancellation, and is `u³/6 − u⁵/120 + u⁷/5040` at 1e-3,
    /// whose next term is 1e-23 of the sum, where the closed form keeps none of its digits.
    #[test]
    fn u_minus_sin_u_keeps_its_digits_below_one() {
        let below = h(1.0_f64.next_down());
        let closed = 1.0 - 1.0_f64.sin();
        assert!(
            (below - closed).abs() <= 8.0 * f64::EPSILON * closed,
            "{below} vs {closed}, the closed form's cancellation"
        );
        let u = 1e-3_f64;
        let terms = u.powi(3) / 6.0 * (1.0 - u * u / 20.0 * (1.0 - u * u / 42.0));
        assert!(
            (h(u) - terms).abs() <= 2.0 * f64::EPSILON * terms,
            "{} vs {terms}, two roundings of each",
            h(u)
        );
        assert_eq!(h(0.0), 0.0);
    }
}
