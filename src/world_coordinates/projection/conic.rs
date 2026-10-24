//! The conic projections, `COP`, `COE`, `COD` and `COO` (CG 2002 §5.4): a cone about the
//! native pole, cut at `θ_a = PVi_1` with half-width `η = PVi_2`.

use crate::world_coordinates::angle;
use std::f64::consts::FRAC_PI_4;

use crate::error::Result;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::{ProjectedCoordinate, Projection};

#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum ConicKind {
    Cop,
    Coe,
    Cod,
    Coo,
}

/// A conic projection's kernel: the cone constant `C`, the offset `Y₀` of its apex, and
/// what each radius formula takes from `θ_a` and `η`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Conic {
    kind: ConicKind,
    pub(super) c: f64,
    y0: f64,
    theta_a_degrees: f64,
    cos_eta: f64,
    cot_theta_a: f64,
    sin_theta1: f64,
    sin_theta2: f64,
    psi: f64,
}

impl Conic {
    pub(super) fn new(kind: ConicKind, pv: &[f64; 21]) -> Self {
        let theta_a = pv[1] * D2R;
        let eta = pv[2] * D2R;
        let theta1 = theta_a - eta;
        let theta2 = theta_a + eta;
        let sin_theta1 = theta1.sin();
        let sin_theta2 = theta2.sin();
        let cos_eta = eta.cos();
        let cot_theta_a = 1.0 / theta_a.tan();
        let (c, y0, psi) = match kind {
            ConicKind::Cop => {
                let c = theta_a.sin();
                (c, R2D * cos_eta * cot_theta_a, 0.0)
            }
            ConicKind::Coe => {
                let c = f64::midpoint(sin_theta1, sin_theta2);
                let y0 = R2D / c
                    * (1.0 + sin_theta1 * sin_theta2 - 2.0 * c * theta_a.sin())
                        .max(0.0)
                        .sqrt();
                (c, y0, 0.0)
            }
            ConicKind::Cod => {
                // Equidistant: C = sinθ_a·sinη/η; Y0 = (180/π)·(η/tanη)·cotθ_a. Both
                // quotients are accurate for any η ≠ 0; η = 0 takes their limits.
                let (c, k) = if eta == 0.0 {
                    (theta_a.sin(), 1.0)
                } else {
                    (theta_a.sin() * eta.sin() / eta, eta / eta.tan())
                };
                (c, R2D * k * cot_theta_a, 0.0)
            }
            ConicKind::Coo => {
                // C = ln(cosθ₂/cosθ₁) / ln(tan b₂/tan b₁), bᵢ = π/4 − θᵢ/2. Both ratios
                // tend to 1 as η → 0, so each logarithm is taken as `ln_1p` of the
                // ratio's exact excess over 1: cosθ₂ − cosθ₁ = −2·sinθ_a·sinη and
                // tan b₂/tan b₁ − 1 = sin(b₂ − b₁)/(cos b₂·sin b₁) with b₂ − b₁ = −η.
                let c = if eta == 0.0 {
                    theta_a.sin()
                } else {
                    let (b1, b2) = (FRAC_PI_4 - theta1 / 2.0, FRAC_PI_4 - theta2 / 2.0);
                    (-2.0 * theta_a.sin() * eta.sin() / theta1.cos()).ln_1p()
                        / ((-eta).sin() / (b2.cos() * b1.sin())).ln_1p()
                };
                let psi = R2D * theta1.cos() / (c * (FRAC_PI_4 - theta1 / 2.0).tan().powf(c));
                let y0 = psi * (FRAC_PI_4 - theta_a / 2.0).tan().powf(c);
                (c, y0, psi)
            }
        };
        Self {
            kind,
            c,
            y0,
            theta_a_degrees: pv[1],
            cos_eta,
            cot_theta_a,
            sin_theta1,
            sin_theta2,
            psi,
        }
    }

    pub(super) fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let r = self.radius(projection, native.theta())?;
        let cp = (self.c * native.phi) * D2R;
        projection.projected_coordinate(r * cp.sin(), self.y0 - r * cp.cos())
    }

    pub(super) fn deproject(
        self,
        projection: Projection,
        x: f64,
        y: f64,
    ) -> Result<NativeCoordinate> {
        let s = self.theta_a_degrees.signum();
        let r = s * x.hypot(self.y0 - y);
        let phi = (s * x).atan2(s * (self.y0 - y)) * R2D / self.c;
        projection.native_coordinate(phi, self.theta(projection, r)?)
    }

    /// The radius `R_θ` (deg) of a native latitude `θ` (deg).
    fn radius(self, projection: Projection, theta: f64) -> Result<f64> {
        let theta_radians = theta * D2R;
        let radius = match self.kind {
            ConicKind::Cop => {
                let offset = theta - self.theta_a_degrees;
                // θ − θa = ±90° diverges; a pole is the cone's apex only on the side
                // of θa; elsewhere a radius of the wrong sign is the far nappe.
                if angle::cosd(offset) == 0.0 {
                    return Err(projection.world_domain_error());
                }
                if theta.abs() == 90.0 {
                    if (theta < 0.0) != (self.theta_a_degrees < 0.0) {
                        return Err(projection.world_domain_error());
                    }
                    return Ok(0.0);
                }
                let radius = R2D * self.cos_eta * (self.cot_theta_a - (offset * D2R).tan());
                if radius * self.c < 0.0 {
                    return Err(projection.world_domain_error());
                }
                radius
            }
            ConicKind::Coe => {
                let value =
                    1.0 + self.sin_theta1 * self.sin_theta2 - 2.0 * self.c * theta_radians.sin();
                R2D / self.c * projection.checked_sqrt(value, 1.0)?
            }
            ConicKind::Cod => self.y0 + (self.theta_a_degrees - theta),
            // The far pole is at infinity unless the cone opens towards it.
            ConicKind::Coo if theta == -90.0 && self.c >= 0.0 => {
                return Err(projection.world_domain_error());
            }
            ConicKind::Coo => self.psi * (FRAC_PI_4 - theta_radians / 2.0).tan().powf(self.c),
        };
        if radius.is_finite() {
            Ok(radius)
        } else {
            Err(projection.world_domain_error())
        }
    }

    /// The native latitude `θ` (deg) of a radius `R_θ` (deg).
    fn theta(self, projection: Projection, r: f64) -> Result<f64> {
        let theta = match self.kind {
            ConicKind::Cop => {
                let tan = self.cot_theta_a - r / (R2D * self.cos_eta);
                self.theta_a_degrees + tan.atan() * R2D
            }
            ConicKind::Coe => {
                let sin_t = (1.0 + self.sin_theta1 * self.sin_theta2 - (r * self.c / R2D).powi(2))
                    / (2.0 * self.c);
                projection.checked_asin(sin_t)? * R2D
            }
            ConicKind::Cod => self.theta_a_degrees - (r - self.y0),
            ConicKind::Coo => 90.0 - 2.0 * (r / self.psi).powf(1.0 / self.c).atan() * R2D,
        };
        if theta.is_finite() {
            Ok(theta)
        } else {
            Err(projection.domain_error())
        }
    }
}
