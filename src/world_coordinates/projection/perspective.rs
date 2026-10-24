//! The zenithal perspective projections, `AZP`, `SZP` and `SIN` (CG 2002 §5.1.1, §5.1.2,
//! §5.1.5): rays from a point of projection to a plane through the native pole.

use std::f64::consts::{FRAC_PI_2, PI};

use crate::fits_error::Result;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::angle::UNIT_TOLERANCE;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::{ProjectedCoordinate, Projection};

/// A perspective projection's kernel, with its constants.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Perspective {
    Azp(Azp),
    Szp(Szp),
    Sin(Sin),
}

/// AZP's distance `μ = PVi_1` and tilt `γ = PVi_2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Azp {
    mu: f64,
    cos_gamma: f64,
    tan_gamma: f64,
    /// From a point of projection outside the sphere (|μ| > 1), the latitude of the
    /// tangent cone sin θ = −1/μ, past which the far side is hidden (deg); else −90°.
    overlap: f64,
    /// Whether the point of projection lies close enough to the tilted plane that its
    /// rays meet it only on the near side of the cone through that point: |μ cos γ| < 1.
    diverges: bool,
}

/// SZP's point of projection `(x_p, y_p, z_p)` from `μ = PVi_1`, `φc = PVi_2` and
/// `θc = PVi_3`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Szp {
    x: f64,
    y: f64,
    z: f64,
    /// Within the sphere's depth range, the latitude sin θ = 1 − z_p past which no
    /// plane is seen (deg); else −90°.
    divergence: f64,
    /// Whether the point lies outside the sphere, |μ| > 1, where a ray can graze it.
    overlaps: bool,
}

/// SIN's slant `ξ = PVi_1` and `η = PVi_2`, both 0 for the orthographic form.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Sin {
    xi: f64,
    eta: f64,
}

impl Perspective {
    pub(super) fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        match self {
            Self::Azp(azp) => azp.project(projection, native),
            Self::Szp(szp) => szp.project(projection, native),
            Self::Sin(sin) => sin.project(projection, native),
        }
    }

    pub(super) fn deproject(
        self,
        projection: Projection,
        x: f64,
        y: f64,
    ) -> Result<NativeCoordinate> {
        match self {
            Self::Azp(azp) => azp.deproject(projection, x, y),
            Self::Szp(szp) => szp.deproject(projection, x, y),
            Self::Sin(sin) => sin.deproject(projection, x, y),
        }
    }
}

impl Azp {
    pub(super) fn new(pv: &[f64; 21]) -> Self {
        let (mu, gamma) = (pv[1], pv[2] * D2R);
        Self {
            mu,
            cos_gamma: gamma.cos(),
            tan_gamma: gamma.tan(),
            overlap: if mu.abs() > 1.0 {
                (-1.0 / mu).asin() * R2D
            } else {
                -90.0
            },
            diverges: (mu * gamma.cos()).abs() < 1.0,
        }
    }

    fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let theta = native.theta();
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let pr = native.phi * D2R;
        let tilt = self.tan_gamma * pr.cos();
        let denom = (self.mu + sin_theta) + cos_theta * tilt;
        if denom == 0.0 || theta < self.overlap {
            return Err(projection.world_domain_error());
        }
        if self.diverges {
            let t = self.mu / (1.0 + tilt * tilt).sqrt();
            if t.abs() <= 1.0 && theta < limb(-tilt.atan() * R2D, t.asin() * R2D) {
                return Err(projection.world_domain_error());
            }
        }
        let r = R2D * (self.mu + 1.0) * cos_theta / denom;
        projection.projected_coordinate(r * pr.sin(), -r * pr.cos() / self.cos_gamma)
    }

    /// Undo the γ shear, then solve A·sinθ + B·cosθ = C for θ, the root nearest the
    /// native pole.
    fn deproject(self, projection: Projection, x: f64, y: f64) -> Result<NativeCoordinate> {
        let yc = y * self.cos_gamma;
        let r = x.hypot(yc) / R2D;
        let phi = x.atan2(-yc);
        let (a, b, c) = (
            r,
            r * phi.cos() * self.tan_gamma - (self.mu + 1.0),
            -r * self.mu,
        );
        let psi = b.atan2(a);
        let base = projection.checked_asin(c / a.hypot(b))?;
        let theta = [base - psi, PI - base - psi]
            .into_iter()
            .min_by(|p, q| (p - FRAC_PI_2).abs().total_cmp(&(q - FRAC_PI_2).abs()))
            .expect("two roots");
        projection.native_coordinate(phi * R2D, theta * R2D)
    }
}

impl Szp {
    pub(super) fn new(pv: &[f64; 21]) -> Self {
        let mu = pv[1];
        let (phic, thetac) = (pv[2] * D2R, pv[3] * D2R);
        let z = mu * thetac.sin() + 1.0;
        Self {
            x: -mu * thetac.cos() * phic.sin(),
            y: mu * thetac.cos() * phic.cos(),
            z,
            divergence: if (z - 1.0).abs() < 1.0 {
                (1.0 - z).asin() * R2D
            } else {
                -90.0
            },
            overlaps: mu.abs() > 1.0,
        }
    }

    fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let theta = native.theta();
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let pr = native.phi * D2R;
        let sigma = cos_theta * cos_theta / (1.0 + sin_theta);
        let denom = self.z - sigma;
        if denom == 0.0 || theta < self.divergence {
            return Err(projection.world_domain_error());
        }
        if self.overlaps {
            // The ray from P = (x_p, y_p, z_p) grazes the unit sphere centred at depth 1
            // where (S − P)·(S − C) = 0, i.e. (z_p − 1) sinθ − s cosθ = −1 with
            // s = x_p sinφ − y_p cosφ, whose root is θ = ψ − asin(1/R) for
            // ψ = atan2(s, z_p − 1), R = √((z_p − 1)² + s²). wcslib's `szps2x` takes
            // R² = (z_p − 1)·z_p − 1 + s², which for θc = 90° misses the AZP limb
            // sinθ = −1/μ of the same geometry (−26.57° against −30° at μ = 2) and
            // refuses points that do have an image.
            let s = self.x * pr.sin() - self.y * pr.cos();
            let t = 1.0 / (self.z - 1.0).hypot(s);
            if t <= 1.0 && theta < limb(s.atan2(self.z - 1.0) * R2D, t.asin() * R2D) {
                return Err(projection.world_domain_error());
            }
        }
        let x = R2D * (self.z * cos_theta * pr.sin() - self.x * sigma) / denom;
        let y = R2D * (-self.z * cos_theta * pr.cos() - self.y * sigma) / denom;
        projection.projected_coordinate(x, y)
    }

    /// With σ = 1 − sinθ, the projection reduces to the quadratic
    /// `z_p²(2σ − σ²) = A² + B²` with A, B linear in σ.
    fn deproject(self, projection: Projection, x: f64, y: f64) -> Result<NativeCoordinate> {
        let (cx, cy) = (x / R2D, y / R2D);
        let (a0, a1) = (cx * self.z, -(cx - self.x));
        let (b0, b1) = (-cy * self.z, cy - self.y);
        let depth = Quadratic {
            a: a1 * a1 + b1 * b1 + self.z * self.z,
            b: 2.0 * (a0 * a1 + b0 * b1) - 2.0 * self.z * self.z,
            c: a0 * a0 + b0 * b0,
        };
        let sigma = depth.visible_sigma(projection)?;
        let phi = (a0 + a1 * sigma).atan2(b0 + b1 * sigma);
        projection.native_depth(phi * R2D, sigma)
    }
}

impl Sin {
    pub(super) const fn new(pv: &[f64; 21]) -> Self {
        Self {
            xi: pv[1],
            eta: pv[2],
        }
    }

    fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let (sin_theta, cos_theta) = (native.sin_theta(), native.cos_theta());
        let pr = native.phi * D2R;
        // The hemisphere facing the plane: θ ≥ 0 for the orthographic form, and for the
        // slant form θ ≥ −atan(ξ sinφ − η cosφ).
        let hidden = if self.xi == 0.0 && self.eta == 0.0 {
            sin_theta < 0.0
        } else {
            native.theta() < -(self.xi * pr.sin() - self.eta * pr.cos()).atan() * R2D
        };
        if hidden {
            return Err(projection.world_domain_error());
        }
        // 1 − sin θ = cos²θ / (1 + sin θ), which does not cancel next to the pole.
        let sigma = cos_theta * cos_theta / (1.0 + sin_theta);
        let x = R2D * (cos_theta * pr.sin() + self.xi * sigma);
        let y = R2D * (-cos_theta * pr.cos() + self.eta * sigma);
        projection.projected_coordinate(x, y)
    }

    fn deproject(self, projection: Projection, x: f64, y: f64) -> Result<NativeCoordinate> {
        let (xi, eta) = (self.xi, self.eta);
        let (cx, cy) = (x / R2D, y / R2D);
        let depth = Quadratic {
            a: xi * xi + eta * eta + 1.0,
            b: -2.0 * (cx * xi + cy * eta + 1.0),
            c: cx * cx + cy * cy,
        };
        let sigma = depth.visible_sigma(projection)?;
        let (a, b) = (cx - xi * sigma, cy - eta * sigma);
        let phi = if a == 0.0 && b == 0.0 {
            0.0
        } else {
            a.atan2(-b) * R2D
        };
        projection.native_depth(phi, sigma)
    }
}

/// The quadratic `a·σ² + b·σ + c = 0` in the depth σ = 1 − sin θ of a slant or
/// perspective deprojection.
#[derive(Debug, Clone, Copy)]
struct Quadratic {
    a: f64,
    b: f64,
    c: f64,
}

impl Quadratic {
    /// The depth in [0, 2] of the visible root.
    fn visible_sigma(self, projection: Projection) -> Result<f64> {
        let Self { a, b, c } = self;
        let discriminant = b * b - 4.0 * a * c;
        let disc = projection.checked_sqrt(discriminant, b * b + (4.0 * a * c).abs())?;
        // The root of the larger magnitude, then the other from the product c/a, which
        // does not cancel where b² ≫ 4ac: next to the pole, where σ is small.
        let q = -0.5 * (b + disc.copysign(b));
        let (s1, s2) = (q / a, c / q);
        let valid = |sigma: f64| {
            sigma.is_finite() && (-UNIT_TOLERANCE..=2.0 + UNIT_TOLERANCE).contains(&sigma)
        };
        match (valid(s1), valid(s2)) {
            (true, true) => Ok(s1.min(s2).clamp(0.0, 2.0)),
            (true, false) => Ok(s1.clamp(0.0, 2.0)),
            (false, true) => Ok(s2.clamp(0.0, 2.0)),
            (false, false) => Err(projection.domain_error()),
        }
    }
}

/// The lower of the two latitudes `a = s − t` and `b = s + t + 180°` (each folded to at
/// most 90°) a perspective projection's limb lies on, from its azimuthal offset `s` and
/// half-angle `t` (deg) — the bound below which a point has no image.
fn limb(s: f64, t: f64) -> f64 {
    let fold = |angle: f64| if angle > 90.0 { angle - 360.0 } else { angle };
    fold(s - t).max(fold(s + t + 180.0))
}
