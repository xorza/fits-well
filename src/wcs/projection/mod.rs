//! Celestial projection classification and closed-set transform kernels.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2};

use crate::error::{FitsError, Result};
use crate::wcs::{D2R, DOMAIN_TOLERANCE, R2D, cosd};

mod cube;
mod healpix;

const NEWTON_RESIDUAL_TOLERANCE: f64 = 1e-12;

/// A celestial projection algorithm — the 3-letter `CTYPE` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `TAN` — gnomonic (zenithal).
    Tan,
    /// `SIN` — orthographic/slant (zenithal).
    Sin,
    /// `ARC` — zenithal equidistant.
    Arc,
    /// `STG` — stereographic (zenithal).
    Stg,
    /// `ZEA` — zenithal equal-area.
    Zea,
    /// `CAR` — plate carrée (cylindrical).
    Car,
    /// `CEA` — cylindrical equal-area.
    Cea,
    /// `MER` — Mercator (cylindrical).
    Mer,
    /// `SFL` — Sanson–Flamsteed (pseudo-cylindrical).
    Sfl,
    /// `AIT` — Hammer–Aitoff (all-sky, pseudo-cylindrical).
    Ait,
    /// `MOL` — Mollweide (all-sky, pseudo-cylindrical).
    Mol,
    /// `ZPN` — zenithal polynomial (`PVi_m` coefficients).
    Zpn,
    /// `CYP` — cylindrical perspective (`μ = PVi_1`, `λ = PVi_2`).
    Cyp,
    /// `PAR` — parabolic (pseudo-cylindrical).
    Par,
    /// `COP` — conic perspective (`θ_a = PVi_1`, `η = PVi_2`).
    Cop,
    /// `COE` — conic equal-area.
    Coe,
    /// `COD` — conic equidistant.
    Cod,
    /// `COO` — conic orthomorphic.
    Coo,
    /// `BON` — Bonne's equal-area (pseudo-conic, `θ₁ = PVi_1`).
    Bon,
    /// `AIR` — Airy (zenithal, minimum-error; `θ_b = PVi_1`).
    Air,
    /// `AZP` — zenithal perspective (`μ = PVi_1`, tilt `γ = PVi_2`).
    Azp,
    /// `PCO` — polyconic.
    Pco,
    /// `SZP` — slant zenithal perspective (`μ = PVi_1`, `φc = PVi_2`, `θc = PVi_3`).
    Szp,
    /// `TSC` — tangential spherical cube.
    Tsc,
    /// `CSC` — COBE quadrilateralized spherical cube.
    Csc,
    /// `QSC` — quadrilateralized spherical cube.
    Qsc,
    /// `HPX` — HEALPix (`H = PVi_1`, `K = PVi_2`).
    Hpx,
}

/// The projection family — it fixes the fiducial point and selects the deprojection
/// branch. The single source of truth for membership that `from_code`, `is_zenithal`,
/// `is_conic`, and `reference_point` all derive from (via [`PROJECTIONS`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// Fiducial point at the native pole (`θ₀ = 90°`), radial deprojection.
    Zenithal,
    /// `θ₀ = 90°` too, but a bespoke tilted/slant deprojection — `AZP`/`SZP`.
    ZenithalPerspective,
    /// `θ₀ = θ_a = PVi_1` — the conics.
    Conic,
    /// `θ₀ = 0°` — cylindrical, pseudo-cylindrical, polyconic, Bonne.
    Other,
}

/// The `CTYPE` code, variant, and [`Family`] for every supported projection — the one
/// membership table the classification methods consult, so adding a projection is a
/// single row rather than edits to four functions.
const PROJECTIONS: &[(&str, Projection, Family)] = &[
    ("TAN", Projection::Tan, Family::Zenithal),
    ("SIN", Projection::Sin, Family::Zenithal),
    ("ARC", Projection::Arc, Family::Zenithal),
    ("STG", Projection::Stg, Family::Zenithal),
    ("ZEA", Projection::Zea, Family::Zenithal),
    ("ZPN", Projection::Zpn, Family::Zenithal),
    ("AIR", Projection::Air, Family::Zenithal),
    ("AZP", Projection::Azp, Family::ZenithalPerspective),
    ("SZP", Projection::Szp, Family::ZenithalPerspective),
    ("COP", Projection::Cop, Family::Conic),
    ("COE", Projection::Coe, Family::Conic),
    ("COD", Projection::Cod, Family::Conic),
    ("COO", Projection::Coo, Family::Conic),
    ("CAR", Projection::Car, Family::Other),
    ("CEA", Projection::Cea, Family::Other),
    ("MER", Projection::Mer, Family::Other),
    ("SFL", Projection::Sfl, Family::Other),
    ("AIT", Projection::Ait, Family::Other),
    ("MOL", Projection::Mol, Family::Other),
    ("CYP", Projection::Cyp, Family::Other),
    ("PAR", Projection::Par, Family::Other),
    ("BON", Projection::Bon, Family::Other),
    ("PCO", Projection::Pco, Family::Other),
    ("TSC", Projection::Tsc, Family::Other),
    ("CSC", Projection::Csc, Family::Other),
    ("QSC", Projection::Qsc, Family::Other),
    ("HPX", Projection::Hpx, Family::Other),
];

impl Projection {
    pub(super) fn from_code(code: &str) -> Option<Projection> {
        PROJECTIONS
            .iter()
            .find(|&&(c, ..)| c == code)
            .map(|&(_, proj, _)| proj)
    }

    /// This projection's [`Family`] (every variant is listed in [`PROJECTIONS`]).
    fn family(self) -> Family {
        PROJECTIONS
            .iter()
            .find(|&&(_, proj, _)| proj == self)
            .map(|&(.., fam)| fam)
            .expect("every Projection variant is listed in PROJECTIONS")
    }

    pub(super) fn is_conic(self) -> bool {
        self.family() == Family::Conic
    }

    pub(super) fn code(self) -> &'static str {
        PROJECTIONS
            .iter()
            .find(|&&(_, projection, _)| projection == self)
            .map(|&(code, ..)| code)
            .expect("every Projection variant is listed in PROJECTIONS")
    }

    pub(super) fn parameter_defaults(self) -> [f64; 21] {
        let mut pv = [0.0; 21];
        match self {
            Projection::Air => pv[1] = 90.0,
            Projection::Cyp => {
                pv[1] = 1.0;
                pv[2] = 1.0;
            }
            Projection::Cea => pv[1] = 1.0,
            Projection::Szp => pv[3] = 90.0,
            Projection::Hpx => {
                pv[1] = 4.0;
                pv[2] = 3.0;
            }
            _ => {}
        }
        pv
    }

    /// Validates `pv` for this projection and derives what the kernels need from it once.
    pub(super) fn parameters(self, pv: [f64; 21]) -> Result<ProjectionParameters> {
        let degenerate = || FitsError::InvalidValue {
            card: format!("degenerate {} projection parameters", self.code()),
        };
        let invalid = match self {
            Projection::Cea => pv[1] == 0.0,
            Projection::Cyp => pv[2] == 0.0 || pv[1] + pv[2] == 0.0,
            Projection::Hpx => {
                !pv[1].is_finite() || pv[1] <= 0.0 || !pv[2].is_finite() || pv[2] <= 0.0
            }
            _ => false,
        };
        if invalid {
            return Err(degenerate());
        }
        let zpn = if self == Projection::Zpn {
            ZpnBranch::new(&pv).ok_or_else(degenerate)?
        } else {
            ZpnBranch::NONE
        };
        Ok(ProjectionParameters { pv, zpn })
    }
}

/// A projection's `PVi_m` parameters, validated, with what is derived from them once
/// rather than per coordinate.
#[derive(Debug, Clone, Copy)]
pub(super) struct ProjectionParameters {
    pub(super) pv: [f64; 21],
    zpn: ZpnBranch,
}

/// The branch of a ZPN polynomial `R(ζ) = Σ Pₘ ζᵐ` that the projection uses: from the
/// native pole out to the first point where `R′` vanishes, past which the radius folds
/// back and two colatitudes share a radius (wcslib `zpnset`).
#[derive(Debug, Clone, Copy)]
struct ZpnBranch {
    /// The highest non-zero coefficient's index.
    degree: usize,
    /// The colatitude (rad) where the branch ends: the first zero of `R′`, or π.
    zeta_max: f64,
    /// `R(zeta_max)`, the largest radius the branch reaches.
    radius_max: f64,
}

impl ZpnBranch {
    /// No branch: the parameters of a projection other than ZPN.
    const NONE: ZpnBranch = ZpnBranch {
        degree: 0,
        zeta_max: PI,
        radius_max: f64::INFINITY,
    };

    /// The branch of `pv`, or `None` when the polynomial is zero, or of degree two or more
    /// with `P₁ ≤ 0` — falling from the pole, so no branch exists.
    fn new(pv: &[f64; 21]) -> Option<ZpnBranch> {
        let degree = pv.iter().rposition(|&coefficient| coefficient != 0.0)?;
        if degree < 2 {
            return Some(ZpnBranch {
                degree,
                zeta_max: PI,
                radius_max: evaluate_zpn(PI, pv).value,
            });
        }
        if pv[1] <= 0.0 {
            return None;
        }
        // The first sign change of R′ on a one-degree grid, then bisected to the
        // precision of the colatitude itself; no change within 180° leaves the whole
        // sphere on the branch.
        let derivative = |zeta: f64| evaluate_zpn(zeta, pv).derivative;
        let zeta_max = (1..=180)
            .map(|degree| f64::from(degree) * D2R)
            .find(|&zeta| derivative(zeta) <= 0.0)
            .map_or(PI, |above| {
                let (mut low, mut high) = (above - D2R, above);
                while high - low > f64::EPSILON * high {
                    let middle = 0.5 * (low + high);
                    if derivative(middle) > 0.0 {
                        low = middle;
                    } else {
                        high = middle;
                    }
                }
                low
            });
        Some(ZpnBranch {
            degree,
            zeta_max,
            radius_max: evaluate_zpn(zeta_max, pv).value,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct ConicConstants {
    c: f64,
    y0: f64,
    theta_a_degrees: f64,
    cos_eta: f64,
    cot_theta_a: f64,
    sin_theta1: f64,
    sin_theta2: f64,
    psi: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct NativeCoordinate {
    pub(super) phi: f64,
    pub(super) theta: f64,
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProjectedCoordinate {
    pub(super) x: f64,
    pub(super) y: f64,
}

#[derive(Debug, Clone, Copy)]
struct SzpVertex {
    x: f64,
    y: f64,
    z: f64,
}

impl Projection {
    pub(super) fn domain_error(self) -> FitsError {
        FitsError::WcsProjectionDomain {
            projection: self.code(),
        }
    }

    pub(super) fn world_domain_error(self) -> FitsError {
        FitsError::WcsWorldOutOfDomain {
            projection: self.code(),
        }
    }

    fn checked_asin(self, value: f64) -> Result<f64> {
        if !value.is_finite()
            || !(-1.0 - DOMAIN_TOLERANCE..=1.0 + DOMAIN_TOLERANCE).contains(&value)
        {
            return Err(self.domain_error());
        }
        Ok(value.clamp(-1.0, 1.0).asin())
    }

    fn checked_sqrt(self, value: f64, scale: f64) -> Result<f64> {
        if !value.is_finite() || value < -DOMAIN_TOLERANCE * scale.abs().max(1.0) {
            return Err(self.domain_error());
        }
        Ok(value.max(0.0).sqrt())
    }

    fn visible_sigma(self, qa: f64, qb: f64, qc: f64) -> Result<f64> {
        let discriminant = qb * qb - 4.0 * qa * qc;
        let disc = self.checked_sqrt(discriminant, qb * qb + (4.0 * qa * qc).abs())?;
        let s1 = (-qb - disc) / (2.0 * qa);
        let s2 = (-qb + disc) / (2.0 * qa);
        let valid = |sigma: f64| {
            sigma.is_finite() && (-DOMAIN_TOLERANCE..=2.0 + DOMAIN_TOLERANCE).contains(&sigma)
        };
        match (valid(s1), valid(s2)) {
            (true, true) => Ok(s1.min(s2).clamp(0.0, 2.0)),
            (true, false) => Ok(s1.clamp(0.0, 2.0)),
            (false, true) => Ok(s2.clamp(0.0, 2.0)),
            (false, false) => Err(self.domain_error()),
        }
    }

    fn native_coordinate(self, phi: f64, theta: f64) -> Result<NativeCoordinate> {
        if !phi.is_finite()
            || !theta.is_finite()
            || !(-90.0 - DOMAIN_TOLERANCE..=90.0 + DOMAIN_TOLERANCE).contains(&theta)
        {
            return Err(self.domain_error());
        }
        Ok(NativeCoordinate {
            phi,
            theta: theta.clamp(-90.0, 90.0),
        })
    }

    fn projected_coordinate(self, x: f64, y: f64) -> Result<ProjectedCoordinate> {
        if !x.is_finite() || !y.is_finite() {
            return Err(self.world_domain_error());
        }
        Ok(ProjectedCoordinate { x, y })
    }

    /// The fiducial point `(φ₀, θ₀)` in degrees. Zenithal (incl. the perspective
    /// `AZP`/`SZP`): `(0, 90)`; conics: `(0, θ_a)` where `θ_a = PVi_1`; else `(0, 0)`.
    pub(super) fn reference_point(self, pv: &[f64; 21]) -> NativeCoordinate {
        match self.family() {
            Family::Zenithal | Family::ZenithalPerspective => NativeCoordinate {
                phi: 0.0,
                theta: 90.0,
            },
            Family::Conic => NativeCoordinate {
                phi: 0.0,
                theta: pv[1],
            },
            Family::Other => NativeCoordinate {
                phi: 0.0,
                theta: 0.0,
            },
        }
    }

    /// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
    pub(super) fn deproject(
        self,
        x: f64,
        y: f64,
        parameters: &ProjectionParameters,
    ) -> Result<NativeCoordinate> {
        let pv = &parameters.pv;
        let projection = self;
        if matches!(
            projection,
            Projection::Tsc | Projection::Csc | Projection::Qsc
        ) {
            return cube::deproject(projection, x, y);
        }
        if matches!(projection, Projection::Hpx) {
            return healpix::deproject(projection, x, y, pv);
        }
        if matches!(projection, Projection::Azp) {
            // Tilted zenithal perspective (CG 2002 §5.1.1): undo the γ shear, then
            // solve A·sinθ + B·cosθ = C for θ.
            let (mu, gr) = (pv[1], pv[2] * D2R);
            let yc = y * gr.cos();
            let r = x.hypot(yc) / R2D;
            let phi = x.atan2(-yc);
            let (a, b, c) = (r, r * phi.cos() * gr.tan() - (mu + 1.0), -r * mu);
            let rad = a.hypot(b);
            let psi = b.atan2(a);
            let base = self.checked_asin(c / rad)?;
            // Pick the θ root nearest the native pole (θ = 90°).
            let half_pi = FRAC_PI_2;
            let cand = [base - psi, PI - base - psi];
            let theta = cand
                .into_iter()
                .min_by(|p, q| {
                    (p - half_pi)
                        .abs()
                        .partial_cmp(&(q - half_pi).abs())
                        .unwrap()
                })
                .unwrap();
            return self.native_coordinate(phi * R2D, theta * R2D);
        }
        if matches!(projection, Projection::Szp) {
            // Slant zenithal perspective (CG 2002 §5.1.2). With the vertex
            // P = (xp, yp, zp), substitute σ = 1 − sinθ and reduce to a quadratic
            // `zp²(2σ − σ²) = A² + B²` with A, B linear in σ.
            let vertex = szp_vertex(pv);
            let (cx, cy) = (x / R2D, y / R2D);
            // A = a0 + a1·σ, B = b0 + b1·σ.
            let (a0, a1) = (cx * vertex.z, -(cx - vertex.x));
            let (b0, b1) = (-cy * vertex.z, cy - vertex.y);
            let qa = a1 * a1 + b1 * b1 + vertex.z * vertex.z;
            let qb = 2.0 * (a0 * a1 + b0 * b1) - 2.0 * vertex.z * vertex.z;
            let qc = a0 * a0 + b0 * b0;
            let sigma = self.visible_sigma(qa, qb, qc)?;
            let theta = self.checked_asin(1.0 - sigma)?;
            let (a, b) = (a0 + a1 * sigma, b0 + b1 * sigma);
            let phi = a.atan2(b);
            return self.native_coordinate(phi * R2D, theta * R2D);
        }
        if matches!(projection, Projection::Sin) {
            let (xi, eta) = (pv[1], pv[2]);
            let (cx, cy) = (x / R2D, y / R2D);
            let qa = xi * xi + eta * eta + 1.0;
            let qb = -2.0 * (cx * xi + cy * eta + 1.0);
            let qc = cx * cx + cy * cy;
            let sigma = self.visible_sigma(qa, qb, qc)?;
            let theta = self.checked_asin(1.0 - sigma)?;
            let (a, b) = (cx - xi * sigma, cy - eta * sigma);
            let phi = if a == 0.0 && b == 0.0 {
                0.0
            } else {
                a.atan2(-b) * R2D
            };
            return self.native_coordinate(phi, theta * R2D);
        }
        if self.family() == Family::Conic {
            let conic = ConicConstants::new(self, pv);
            let s = pv[1].signum();
            let r = s * x.hypot(conic.y0 - y);
            let phi = (s * x).atan2(s * (conic.y0 - y)) * R2D / conic.c;
            return self.native_coordinate(phi, self.conic_theta(r, conic)?);
        }
        if self.family() == Family::Zenithal {
            let r = x.hypot(y);
            let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
            // Colatitude ζ (rad) from the radius, per projection.
            let u = r / R2D;
            let zeta = match projection {
                Projection::Tan => u.atan(),
                Projection::Sin => unreachable!(),
                Projection::Arc => u,
                Projection::Zea => 2.0 * self.checked_asin(u / 2.0)?,
                Projection::Stg => 2.0 * (u / 2.0).atan(),
                Projection::Zpn => self.zpn_zeta(u, pv, parameters.zpn)?,
                // AIR: solve the transcendental radius for ζ (Newton).
                Projection::Air => air_zeta(u, pv[1])?,
                _ => unreachable!(),
            };
            self.native_coordinate(phi, 90.0 - zeta * R2D)
        } else {
            let [phi, theta] = match projection {
                Projection::Car => [x, y],
                // CEA: λ = PVi_1 (default 1); θ = asin(λ·y/(180/π)).
                Projection::Cea => {
                    let lambda = pv[1];
                    [x, self.checked_asin(lambda * y / R2D)? * R2D]
                }
                Projection::Mer => [x, (2.0 * (y / R2D).exp().atan()) * R2D - 90.0],
                Projection::Sfl => [x / (y * D2R).cos(), y],
                // Hammer–Aitoff inverse (CG 2002 eq. 51).
                Projection::Ait => {
                    let (u, v) = (x * D2R, y * D2R);
                    let z2 = 1.0 - (u / 4.0).powi(2) - (v / 2.0).powi(2);
                    let z = self.checked_sqrt(z2, 1.0)?;
                    let phi = 2.0 * (z * u / 2.0).atan2(2.0 * z2 - 1.0) * R2D;
                    let theta = self.checked_asin(v * z)? * R2D;
                    [phi, theta]
                }
                // Mollweide inverse (CG 2002 eq. 55).
                Projection::Mol => {
                    let s2 = SQRT_2;
                    let gamma = self.checked_asin(y / (s2 * R2D))?;
                    let theta = self.checked_asin((2.0 * gamma + (2.0 * gamma).sin()) / PI)? * R2D;
                    let phi = if gamma.cos().abs() < 1e-12 {
                        0.0
                    } else {
                        PI * x / (2.0 * s2 * gamma.cos())
                    };
                    [phi, theta]
                }
                // CYP inverse: φ = x/λ; θ from η = (y/(180/π))/(μ+λ).
                Projection::Cyp => {
                    let (mu, lambda) = (pv[1], pv[2]);
                    let eta = (y / R2D) / (mu + lambda);
                    let theta =
                        eta.atan2(1.0) + self.checked_asin(eta * mu / (1.0 + eta * eta).sqrt())?;
                    [x / lambda, theta * R2D]
                }
                // PAR inverse (CG 2002 eq. 49).
                Projection::Par => {
                    let theta = 3.0 * self.checked_asin(y / 180.0)?;
                    [x / (2.0 * (2.0 * theta / 3.0).cos() - 1.0), theta * R2D]
                }
                // Polyconic inverse (CG 2002 §5.6.1): Newton on
                // f(θ) = X² + (Y−θ)² − 2(Y−θ)cotθ = 0, then recover φ.
                Projection::Pco => {
                    let (xr, yr) = (x * D2R, y * D2R);
                    if yr.abs() < 1e-12 {
                        return self.native_coordinate(x, 0.0);
                    }
                    let th = pco_theta(xr, yr)?;
                    let d = yr - th;
                    let tanth = th.tan();
                    let omega = (xr * tanth).atan2(1.0 - d * tanth);
                    [omega / th.sin() * R2D, th * R2D]
                }
                // Bonne's pseudoconic inverse (CG 2002 §5.5.1), θ₁ = PVi_1.
                Projection::Bon => {
                    // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0
                    // (avoiding the `1/tan 0` singularity below).
                    if pv[1] == 0.0 {
                        return self.native_coordinate(x / (y * D2R).cos(), y);
                    }
                    let t1 = pv[1] * D2R;
                    let y0 = t1 + 1.0 / t1.tan();
                    let s = pv[1].signum();
                    let yc = y0 - y * D2R;
                    let r = s * (x * D2R).hypot(yc);
                    let tr = y0 - r;
                    let aphi = (s * x * D2R).atan2(s * yc);
                    [aphi * r / tr.cos() * R2D, tr * R2D]
                }
                _ => unreachable!(),
            };
            self.native_coordinate(phi, theta)
        }
    }

    /// Project native `(φ, θ)` (deg) to intermediate world `(x, y)` (deg).
    ///
    /// A point the projection has no image for is refused: behind a zenithal
    /// projection's horizon, past a perspective projection's limb or beyond its point of
    /// divergence, at a pole a projection sends to infinity, or past ZPN's inflection.
    /// The bounds are wcslib's (`*s2x` with `bounds & 1`), SZP's limb excepted.
    pub(super) fn project(
        self,
        phi: f64,
        theta: f64,
        parameters: &ProjectionParameters,
    ) -> Result<ProjectedCoordinate> {
        if !phi.is_finite()
            || !theta.is_finite()
            || !(-90.0 - DOMAIN_TOLERANCE..=90.0 + DOMAIN_TOLERANCE).contains(&theta)
        {
            return Err(self.world_domain_error());
        }
        let theta = theta.clamp(-90.0, 90.0);
        let pv = &parameters.pv;
        let projection = self;
        if matches!(
            projection,
            Projection::Tsc | Projection::Csc | Projection::Qsc
        ) {
            return cube::project(projection, phi, theta);
        }
        if matches!(projection, Projection::Hpx) {
            return healpix::project(projection, phi, theta, pv);
        }
        if matches!(projection, Projection::Azp) {
            let (mu, gr) = (pv[1], pv[2] * D2R);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let tilt = gr.tan() * pr.cos();
            let denom = (mu + tr.sin()) + tr.cos() * tilt;
            if denom == 0.0 {
                return Err(self.world_domain_error());
            }
            // Overlap: from a point of projection outside the sphere (|μ| > 1), the far
            // side beyond the tangent cone sinθ = −1/μ is hidden.
            let overlap = if mu.abs() > 1.0 {
                (-1.0 / mu).asin() * R2D
            } else {
                -90.0
            };
            if theta < overlap {
                return Err(self.world_domain_error());
            }
            if (mu * gr.cos()).abs() < 1.0 {
                // Divergence: rays from a point of projection that close to the tilted
                // plane meet it only on the near side of the cone through that point.
                let t = mu / (1.0 + tilt * tilt).sqrt();
                if t.abs() <= 1.0 && theta < limb(-tilt.atan() * R2D, t.asin() * R2D) {
                    return Err(self.world_domain_error());
                }
            }
            let r = R2D * (mu + 1.0) * tr.cos() / denom;
            return self.projected_coordinate(r * pr.sin(), -r * pr.cos() / gr.cos());
        }
        if matches!(projection, Projection::Szp) {
            let vertex = szp_vertex(pv);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let sigma = 1.0 - tr.sin();
            let denom = vertex.z - sigma;
            if denom == 0.0 {
                return Err(self.world_domain_error());
            }
            // Divergence: a point of projection within the sphere's depth range sees no
            // plane past the depth σ = z_p, sinθ = 1 − z_p.
            let divergence = if (vertex.z - 1.0).abs() < 1.0 {
                (1.0 - vertex.z).asin() * R2D
            } else {
                -90.0
            };
            if theta < divergence {
                return Err(self.world_domain_error());
            }
            if pv[1].abs() > 1.0 {
                // Overlap: the ray from P = (x_p, y_p, z_p) grazes the unit sphere centred
                // at depth 1 where (S − P)·(S − C) = 0, i.e.
                // (z_p − 1) sinθ − s cosθ = −1 with s = x_p sinφ − y_p cosφ, whose root is
                // θ = ψ − asin(1/R) for ψ = atan2(s, z_p − 1), R = √((z_p − 1)² + s²).
                // wcslib's `szps2x` takes R² = (z_p − 1)·z_p − 1 + s², which for θc = 90°
                // misses the AZP limb sinθ = −1/μ of the same geometry (−26.57° against
                // −30° at μ = 2) and refuses points that do have an image.
                let s = vertex.x * pr.sin() - vertex.y * pr.cos();
                let t = 1.0 / (vertex.z - 1.0).hypot(s);
                if t <= 1.0 && theta < limb(s.atan2(vertex.z - 1.0) * R2D, t.asin() * R2D) {
                    return Err(self.world_domain_error());
                }
            }
            let x = R2D * (vertex.z * tr.cos() * pr.sin() - vertex.x * sigma) / denom;
            let y = R2D * (-vertex.z * tr.cos() * pr.cos() - vertex.y * sigma) / denom;
            return self.projected_coordinate(x, y);
        }
        if matches!(projection, Projection::Sin) {
            let (tr, pr) = (theta * D2R, phi * D2R);
            let (xi, eta) = (pv[1], pv[2]);
            // The hemisphere facing the plane: θ ≥ 0 for the orthographic form, and for
            // the slant form θ ≥ −atan(ξ sinφ − η cosφ).
            let horizon = if xi == 0.0 && eta == 0.0 {
                0.0
            } else {
                -(xi * pr.sin() - eta * pr.cos()).atan() * R2D
            };
            if theta < horizon {
                return Err(self.world_domain_error());
            }
            let sigma = 1.0 - tr.sin();
            let x = R2D * (tr.cos() * pr.sin() + xi * sigma);
            let y = R2D * (-tr.cos() * pr.cos() + eta * sigma);
            return self.projected_coordinate(x, y);
        }
        if self.family() == Family::Conic {
            let conic = ConicConstants::new(self, pv);
            let r = self.conic_radius(theta, conic)?;
            let cp = (conic.c * phi) * D2R;
            return self.projected_coordinate(r * cp.sin(), conic.y0 - r * cp.cos());
        }
        if self.family() == Family::Zenithal {
            let zeta = (90.0 - theta) * D2R;
            let beyond = match projection {
                // TAN images only the hemisphere above the plane; the equator diverges.
                Projection::Tan => (theta * D2R).sin() <= 0.0,
                // STG sends the antipode of its pole to infinity.
                Projection::Stg => 1.0 + (theta * D2R).sin() == 0.0,
                Projection::Zpn => zeta > parameters.zpn.zeta_max,
                Projection::Air => theta == -90.0,
                _ => false,
            };
            if beyond {
                return Err(self.world_domain_error());
            }
            let r = match projection {
                Projection::Tan => R2D * zeta.tan(),
                Projection::Sin => unreachable!(),
                Projection::Arc => R2D * zeta,
                Projection::Zea => 2.0 * R2D * (zeta / 2.0).sin(),
                Projection::Stg => 2.0 * R2D * (zeta / 2.0).tan(),
                Projection::Zpn => R2D * evaluate_zpn(zeta, pv).value,
                Projection::Air => R2D * air_radius_u(zeta, pv[1]),
                _ => unreachable!(),
            };
            let p = phi * D2R;
            self.projected_coordinate(r * p.sin(), -r * p.cos())
        } else {
            let t = theta * D2R;
            let [x, y] = match projection {
                Projection::Car => [phi, theta],
                Projection::Cea => {
                    let lambda = pv[1];
                    [phi, R2D * t.sin() / lambda]
                }
                // The poles are at infinity.
                Projection::Mer if theta.abs() == 90.0 => {
                    return Err(self.world_domain_error());
                }
                Projection::Mer => [phi, R2D * ((45.0 + theta / 2.0) * D2R).tan().ln()],
                Projection::Sfl => [phi * t.cos(), theta],
                Projection::Ait => {
                    let pr = phi * D2R;
                    let gamma = R2D * (2.0 / (1.0 + t.cos() * (pr / 2.0).cos())).sqrt();
                    [2.0 * gamma * t.cos() * (pr / 2.0).sin(), gamma * t.sin()]
                }
                Projection::Mol => {
                    // Solve 2γ + sin2γ = π·sinθ for γ (Newton).
                    let s2 = SQRT_2;
                    let g = mollweide_gamma(t)?;
                    [(2.0 * s2 / PI) * phi * g.cos(), s2 * R2D * g.sin()]
                }
                Projection::Cyp => {
                    let (mu, lambda) = (pv[1], pv[2]);
                    // The latitude whose rays run parallel to the cylinder.
                    if mu + t.cos() == 0.0 {
                        return Err(self.world_domain_error());
                    }
                    [lambda * phi, R2D * (mu + lambda) * t.sin() / (mu + t.cos())]
                }
                Projection::Par => [
                    phi * (2.0 * (2.0 * t / 3.0).cos() - 1.0),
                    180.0 * (t / 3.0).sin(),
                ],
                Projection::Bon => {
                    // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0.
                    if pv[1] == 0.0 {
                        return self.projected_coordinate(phi * t.cos(), theta);
                    }
                    let t1 = pv[1] * D2R;
                    let y0 = t1 + 1.0 / t1.tan();
                    let r = y0 - t;
                    let aphi = phi * D2R * t.cos() / r;
                    [R2D * r * aphi.sin(), R2D * (y0 - r * aphi.cos())]
                }
                Projection::Pco => {
                    if theta.abs() < 1e-12 {
                        return self.projected_coordinate(phi, 0.0);
                    }
                    let omega = phi * D2R * t.sin();
                    let cot = 1.0 / t.tan();
                    [
                        R2D * cot * omega.sin(),
                        theta + R2D * cot * (1.0 - omega.cos()),
                    ]
                }
                _ => unreachable!(),
            };
            self.projected_coordinate(x, y)
        }
    }

    /// Conic radius `R_θ` (deg) for a native latitude `θ` (deg).
    fn conic_radius(self, theta: f64, conic: ConicConstants) -> Result<f64> {
        let theta_radians = theta * D2R;
        let radius = match self {
            Projection::Cop => {
                let offset = theta - conic.theta_a_degrees;
                // θ − θa = ±90° diverges; a pole is the cone's apex only on the side
                // of θa; elsewhere a radius of the wrong sign is the far nappe.
                if cosd(offset) == 0.0 {
                    return Err(self.world_domain_error());
                }
                if theta.abs() == 90.0 {
                    if (theta < 0.0) != (conic.theta_a_degrees < 0.0) {
                        return Err(self.world_domain_error());
                    }
                    return Ok(0.0);
                }
                let radius = R2D * conic.cos_eta * (conic.cot_theta_a - (offset * D2R).tan());
                if radius * conic.c < 0.0 {
                    return Err(self.world_domain_error());
                }
                radius
            }
            Projection::Coe => {
                let value =
                    1.0 + conic.sin_theta1 * conic.sin_theta2 - 2.0 * conic.c * theta_radians.sin();
                R2D / conic.c * self.checked_sqrt(value, 1.0)?
            }
            Projection::Cod => conic.y0 + (conic.theta_a_degrees - theta),
            // The far pole is at infinity unless the cone opens towards it.
            Projection::Coo if theta == -90.0 && conic.c >= 0.0 => {
                return Err(self.world_domain_error());
            }
            Projection::Coo => conic.psi * (FRAC_PI_4 - theta_radians / 2.0).tan().powf(conic.c),
            _ => unreachable!(),
        };
        if radius.is_finite() {
            Ok(radius)
        } else {
            Err(self.world_domain_error())
        }
    }

    /// Native latitude `θ` (deg) for a conic radius `R_θ` (deg).
    fn conic_theta(self, r: f64, conic: ConicConstants) -> Result<f64> {
        let theta = match self {
            Projection::Cop => {
                let tan = conic.cot_theta_a - r / (R2D * conic.cos_eta);
                conic.theta_a_degrees + tan.atan() * R2D
            }
            Projection::Coe => {
                let sin_t = (1.0 + conic.sin_theta1 * conic.sin_theta2
                    - (r * conic.c / R2D).powi(2))
                    / (2.0 * conic.c);
                self.checked_asin(sin_t)? * R2D
            }
            Projection::Cod => conic.theta_a_degrees - (r - conic.y0),
            Projection::Coo => 90.0 - 2.0 * (r / conic.psi).powf(1.0 / conic.c).atan() * R2D,
            _ => unreachable!(),
        };
        if theta.is_finite() {
            Ok(theta)
        } else {
            Err(self.domain_error())
        }
    }
}

impl Projection {
    /// The colatitude ζ (rad) on the ZPN branch whose radius is `u = R/(180/π)`, as
    /// wcslib's `zpnx2s` takes it: exact for a linear or quadratic polynomial, the root
    /// nearest the pole; above that, bracketed on `[0, ζ_max]` by `[P₀, R(ζ_max)]` and
    /// narrowed by false position (stepping at least a tenth of the bracket) until the
    /// residual or the bracket is below `1e-13`, wcslib's tolerance.
    fn zpn_zeta(self, u: f64, pv: &[f64; 21], branch: ZpnBranch) -> Result<f64> {
        const TOLERANCE: f64 = 1e-13;
        match branch.degree {
            0 => Err(self.domain_error()),
            1 => Ok((u - pv[0]) / pv[1]),
            2 => {
                let (a, b, c) = (pv[2], pv[1], pv[0] - u);
                let discriminant = b * b - 4.0 * a * c;
                if discriminant < 0.0 {
                    return Err(self.domain_error());
                }
                let root = discriminant.sqrt();
                let (z1, z2) = ((-b + root) / (2.0 * a), (-b - root) / (2.0 * a));
                let mut zeta = z1.min(z2);
                if zeta < -TOLERANCE {
                    zeta = z1.max(z2);
                }
                if !(-TOLERANCE..=PI + TOLERANCE).contains(&zeta) {
                    return Err(self.domain_error());
                }
                Ok(zeta.clamp(0.0, PI))
            }
            _ => {
                let (mut zeta1, mut radius1) = (0.0, pv[0]);
                let (mut zeta2, mut radius2) = (branch.zeta_max, branch.radius_max);
                if u < radius1 - TOLERANCE || u > radius2 + TOLERANCE {
                    return Err(self.domain_error());
                }
                if u <= radius1 {
                    return Ok(zeta1);
                }
                if u >= radius2 {
                    return Ok(zeta2);
                }
                let mut zeta = zeta2;
                for _ in 0..100 {
                    let lambda = ((radius2 - u) / (radius2 - radius1)).clamp(0.1, 0.9);
                    zeta = zeta2 - lambda * (zeta2 - zeta1);
                    let radius = evaluate_zpn(zeta, pv).value;
                    if (radius - u).abs() < TOLERANCE {
                        break;
                    }
                    if radius < u {
                        (zeta1, radius1) = (zeta, radius);
                    } else {
                        (zeta2, radius2) = (zeta, radius);
                    }
                    if (zeta2 - zeta1).abs() < TOLERANCE {
                        break;
                    }
                }
                Ok(zeta)
            }
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

impl ConicConstants {
    fn new(projection: Projection, pv: &[f64; 21]) -> ConicConstants {
        let theta_a = pv[1] * D2R;
        let eta = pv[2] * D2R;
        let theta1 = theta_a - eta;
        let theta2 = theta_a + eta;
        let sin_theta1 = theta1.sin();
        let sin_theta2 = theta2.sin();
        let cos_eta = eta.cos();
        let cot_theta_a = 1.0 / theta_a.tan();
        let (c, y0, psi) = match projection {
            Projection::Cop => {
                let c = theta_a.sin();
                (c, R2D * cos_eta * cot_theta_a, 0.0)
            }
            Projection::Coe => {
                let c = (sin_theta1 + sin_theta2) / 2.0;
                let y0 = R2D / c
                    * (1.0 + sin_theta1 * sin_theta2 - 2.0 * c * theta_a.sin())
                        .max(0.0)
                        .sqrt();
                (c, y0, 0.0)
            }
            Projection::Cod => {
                // Equidistant: C = sinθ_a·sinη/η; Y0 = (180/π)·(η/tanη)·cotθ_a.
                let (c, k) = if eta.abs() < 1e-12 {
                    (theta_a.sin(), 1.0)
                } else {
                    (theta_a.sin() * eta.sin() / eta, eta / eta.tan())
                };
                (c, R2D * k * cot_theta_a, 0.0)
            }
            Projection::Coo => {
                let c = if eta.abs() < 1e-12 {
                    theta_a.sin()
                } else {
                    (theta2.cos() / theta1.cos()).ln()
                        / ((FRAC_PI_4 - theta2 / 2.0).tan() / (FRAC_PI_4 - theta1 / 2.0).tan()).ln()
                };
                let psi = R2D * theta1.cos() / (c * (FRAC_PI_4 - theta1 / 2.0).tan().powf(c));
                let y0 = psi * (FRAC_PI_4 - theta_a / 2.0).tan().powf(c);
                (c, y0, psi)
            }
            _ => unreachable!(),
        };
        ConicConstants {
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
}

/// SZP projection vertex `(x_p, y_p, z_p)` from `μ = PVi_1`, `φc = PVi_2`,
/// `θc = PVi_3` (CG 2002 §5.1.2).
fn szp_vertex(pv: &[f64]) -> SzpVertex {
    let mu = pv[1];
    let (phic, thetac) = (pv[2] * D2R, pv[3] * D2R);
    SzpVertex {
        x: -mu * thetac.cos() * phic.sin(),
        y: mu * thetac.cos() * phic.cos(),
        z: mu * thetac.sin() + 1.0,
    }
}

/// AIR `K = ln(cos ξ_b)/tan²ξ_b` constant (`ξ_b = (90°−θ_b)/2`); the `θ_b = 90`
/// limit is `−1/2`.
fn air_k(theta_b: f64) -> f64 {
    let xi_b = (90.0 - theta_b) * D2R / 2.0;
    if xi_b.abs() < 1e-12 {
        -0.5
    } else {
        xi_b.cos().ln() / xi_b.tan().powi(2)
    }
}

/// AIR radius `R/(180/π)` for colatitude `ζ` (rad): `−2[ln(cos ξ)/tan ξ + K tan ξ]`,
/// `ξ = ζ/2`.
fn air_radius_u(zeta: f64, theta_b: f64) -> f64 {
    let xi = zeta / 2.0;
    if xi.abs() < 1e-12 {
        return 0.0;
    }
    -2.0 * (xi.cos().ln() / xi.tan() + air_k(theta_b) * xi.tan())
}

fn no_convergence(projection: Projection) -> FitsError {
    FitsError::WcsNoConvergence {
        algorithm: projection.code(),
    }
}

#[derive(Debug, Clone, Copy)]
struct NewtonEvaluation {
    residual: f64,
    derivative: f64,
}

fn solve_newton(
    projection: Projection,
    initial: f64,
    evaluate: impl Fn(f64) -> NewtonEvaluation,
) -> Result<f64> {
    let mut value = initial;
    for _ in 0..100 {
        let evaluation = evaluate(value);
        if evaluation.residual.is_finite() && evaluation.residual.abs() <= NEWTON_RESIDUAL_TOLERANCE
        {
            return Ok(value);
        }
        if !evaluation.residual.is_finite()
            || !evaluation.derivative.is_finite()
            || evaluation.derivative == 0.0
        {
            return Err(no_convergence(projection));
        }
        let step = evaluation.residual / evaluation.derivative;
        value -= step;
        if !step.is_finite() || !value.is_finite() {
            return Err(no_convergence(projection));
        }
    }
    let residual = evaluate(value).residual;
    if residual.is_finite() && residual.abs() <= NEWTON_RESIDUAL_TOLERANCE {
        Ok(value)
    } else {
        Err(no_convergence(projection))
    }
}

/// Invert the AIR radius for ζ given `u = R/(180/π)` (Newton).
fn air_zeta(u: f64, theta_b: f64) -> Result<f64> {
    solve_newton(Projection::Air, u.max(1e-6), |zeta| NewtonEvaluation {
        residual: air_radius_u(zeta, theta_b) - u,
        derivative: (air_radius_u(zeta + 1e-7, theta_b) - air_radius_u(zeta - 1e-7, theta_b))
            / 2e-7,
    })
}

#[derive(Debug, Clone, Copy)]
struct ZpnEvaluation {
    value: f64,
    derivative: f64,
}

/// Evaluate `Σ Pₘ ζᵐ` and its derivative together with extended Horner.
fn evaluate_zpn(zeta: f64, pv: &[f64; 21]) -> ZpnEvaluation {
    let mut value = pv[20];
    let mut derivative = 0.0;
    for &coefficient in pv[..20].iter().rev() {
        derivative = derivative * zeta + value;
        value = value * zeta + coefficient;
    }
    ZpnEvaluation { value, derivative }
}

fn mollweide_gamma(theta: f64) -> Result<f64> {
    if (theta.abs() - FRAC_PI_2).abs() < DOMAIN_TOLERANCE {
        return Ok(theta.signum() * FRAC_PI_2);
    }
    let target = PI * theta.sin();
    solve_newton(Projection::Mol, theta, |gamma| NewtonEvaluation {
        residual: 2.0 * gamma + (2.0 * gamma).sin() - target,
        derivative: 2.0 + 2.0 * (2.0 * gamma).cos(),
    })
}

fn pco_theta(x: f64, y: f64) -> Result<f64> {
    solve_newton(Projection::Pco, y, |theta| {
        let delta = y - theta;
        let cotangent = 1.0 / theta.tan();
        NewtonEvaluation {
            residual: x * x + delta * delta - 2.0 * delta * cotangent,
            derivative: -2.0 * delta + 2.0 * cotangent + 2.0 * delta / theta.sin().powi(2),
        }
    })
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::wcs::projection::{ProjectionParameters, ZpnBranch};

    impl ProjectionParameters {
        /// Parameters taken as given, for kernel tests that bypass validation.
        pub(crate) fn raw(pv: [f64; 21]) -> Self {
            Self {
                pv,
                zpn: ZpnBranch::new(&pv).unwrap_or(ZpnBranch::NONE),
            }
        }
    }
}

#[cfg(test)]
mod tests;
