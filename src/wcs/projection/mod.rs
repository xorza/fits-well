//! Celestial projection classification and closed-set transform kernels.

use std::f64::consts::{FRAC_PI_2, FRAC_PI_4, PI, SQRT_2};

use crate::error::{FitsError, Result};
use crate::wcs::projection::cube::Cube;
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

/// The projection family — it fixes the fiducial point and selects the kernel. Each
/// family's own enum names its members, so every dispatch is an exhaustive match.
#[derive(Debug, Clone, Copy)]
enum Kind {
    /// Fiducial point at the native pole (`θ₀ = 90°`), radial (de)projection.
    Zenithal(Zenithal),
    /// `SIN`: zenithal, with the slant parameters `ξ`, `η` of its own kernel.
    Sin,
    /// `θ₀ = 90°` too, but a tilted or slant perspective — `AZP`/`SZP`.
    Perspective(Perspective),
    /// `θ₀ = θ_a = PVi_1` — the conics.
    Conic(Conic),
    /// `θ₀ = 0°` — cylindrical, pseudo-cylindrical, polyconic, Bonne.
    Equatorial(Equatorial),
    /// `θ₀ = 0°` — the spherical cubes.
    Cube(Cube),
    /// `θ₀ = 0°` — HEALPix.
    Healpix,
}

#[derive(Debug, Clone, Copy)]
enum Zenithal {
    Tan,
    Arc,
    Stg,
    Zea,
    Zpn,
    Air,
}

#[derive(Debug, Clone, Copy)]
enum Perspective {
    Azp,
    Szp,
}

#[derive(Debug, Clone, Copy)]
enum Conic {
    Cop,
    Coe,
    Cod,
    Coo,
}

#[derive(Debug, Clone, Copy)]
enum Equatorial {
    Car,
    Cea,
    Mer,
    Sfl,
    Ait,
    Mol,
    Cyp,
    Par,
    Bon,
    Pco,
}

impl Projection {
    pub(super) fn from_code(code: &str) -> Option<Projection> {
        Some(match code {
            "TAN" => Projection::Tan,
            "SIN" => Projection::Sin,
            "ARC" => Projection::Arc,
            "STG" => Projection::Stg,
            "ZEA" => Projection::Zea,
            "ZPN" => Projection::Zpn,
            "AIR" => Projection::Air,
            "AZP" => Projection::Azp,
            "SZP" => Projection::Szp,
            "COP" => Projection::Cop,
            "COE" => Projection::Coe,
            "COD" => Projection::Cod,
            "COO" => Projection::Coo,
            "CAR" => Projection::Car,
            "CEA" => Projection::Cea,
            "MER" => Projection::Mer,
            "SFL" => Projection::Sfl,
            "AIT" => Projection::Ait,
            "MOL" => Projection::Mol,
            "CYP" => Projection::Cyp,
            "PAR" => Projection::Par,
            "BON" => Projection::Bon,
            "PCO" => Projection::Pco,
            "TSC" => Projection::Tsc,
            "CSC" => Projection::Csc,
            "QSC" => Projection::Qsc,
            "HPX" => Projection::Hpx,
            _ => return None,
        })
    }

    pub(super) const fn code(self) -> &'static str {
        match self {
            Projection::Tan => "TAN",
            Projection::Sin => "SIN",
            Projection::Arc => "ARC",
            Projection::Stg => "STG",
            Projection::Zea => "ZEA",
            Projection::Zpn => "ZPN",
            Projection::Air => "AIR",
            Projection::Azp => "AZP",
            Projection::Szp => "SZP",
            Projection::Cop => "COP",
            Projection::Coe => "COE",
            Projection::Cod => "COD",
            Projection::Coo => "COO",
            Projection::Car => "CAR",
            Projection::Cea => "CEA",
            Projection::Mer => "MER",
            Projection::Sfl => "SFL",
            Projection::Ait => "AIT",
            Projection::Mol => "MOL",
            Projection::Cyp => "CYP",
            Projection::Par => "PAR",
            Projection::Bon => "BON",
            Projection::Pco => "PCO",
            Projection::Tsc => "TSC",
            Projection::Csc => "CSC",
            Projection::Qsc => "QSC",
            Projection::Hpx => "HPX",
        }
    }

    const fn kind(self) -> Kind {
        match self {
            Projection::Tan => Kind::Zenithal(Zenithal::Tan),
            Projection::Arc => Kind::Zenithal(Zenithal::Arc),
            Projection::Stg => Kind::Zenithal(Zenithal::Stg),
            Projection::Zea => Kind::Zenithal(Zenithal::Zea),
            Projection::Zpn => Kind::Zenithal(Zenithal::Zpn),
            Projection::Air => Kind::Zenithal(Zenithal::Air),
            Projection::Sin => Kind::Sin,
            Projection::Azp => Kind::Perspective(Perspective::Azp),
            Projection::Szp => Kind::Perspective(Perspective::Szp),
            Projection::Cop => Kind::Conic(Conic::Cop),
            Projection::Coe => Kind::Conic(Conic::Coe),
            Projection::Cod => Kind::Conic(Conic::Cod),
            Projection::Coo => Kind::Conic(Conic::Coo),
            Projection::Car => Kind::Equatorial(Equatorial::Car),
            Projection::Cea => Kind::Equatorial(Equatorial::Cea),
            Projection::Mer => Kind::Equatorial(Equatorial::Mer),
            Projection::Sfl => Kind::Equatorial(Equatorial::Sfl),
            Projection::Ait => Kind::Equatorial(Equatorial::Ait),
            Projection::Mol => Kind::Equatorial(Equatorial::Mol),
            Projection::Cyp => Kind::Equatorial(Equatorial::Cyp),
            Projection::Par => Kind::Equatorial(Equatorial::Par),
            Projection::Bon => Kind::Equatorial(Equatorial::Bon),
            Projection::Pco => Kind::Equatorial(Equatorial::Pco),
            Projection::Tsc => Kind::Cube(Cube::Tsc),
            Projection::Csc => Kind::Cube(Cube::Csc),
            Projection::Qsc => Kind::Cube(Cube::Qsc),
            Projection::Hpx => Kind::Healpix,
        }
    }

    pub(super) const fn is_conic(self) -> bool {
        matches!(self.kind(), Kind::Conic(_))
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
        let degenerate = || FitsError::InvalidWcs {
            detail: format!("degenerate {} projection parameters", self.code()),
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
    kind: Conic,
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
        let theta = match self.kind() {
            Kind::Zenithal(_) | Kind::Sin | Kind::Perspective(_) => 90.0,
            Kind::Conic(_) => pv[1],
            Kind::Equatorial(_) | Kind::Cube(_) | Kind::Healpix => 0.0,
        };
        NativeCoordinate { phi: 0.0, theta }
    }

    /// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
    pub(super) fn deproject(
        self,
        x: f64,
        y: f64,
        parameters: &ProjectionParameters,
    ) -> Result<NativeCoordinate> {
        let pv = &parameters.pv;
        match self.kind() {
            Kind::Cube(cube) => cube::deproject(cube, x, y),
            Kind::Healpix => healpix::deproject(self, x, y, pv),
            Kind::Perspective(Perspective::Azp) => {
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
                self.native_coordinate(phi * R2D, theta * R2D)
            }
            Kind::Perspective(Perspective::Szp) => {
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
                self.native_coordinate(phi * R2D, theta * R2D)
            }
            Kind::Sin => {
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
                self.native_coordinate(phi, theta * R2D)
            }
            Kind::Conic(conic) => {
                let conic = ConicConstants::new(conic, pv);
                let s = pv[1].signum();
                let r = s * x.hypot(conic.y0 - y);
                let phi = (s * x).atan2(s * (conic.y0 - y)) * R2D / conic.c;
                self.native_coordinate(phi, self.conic_theta(r, conic)?)
            }
            Kind::Zenithal(zenithal) => {
                let r = x.hypot(y);
                let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
                // Colatitude ζ (rad) from the radius, per projection.
                let u = r / R2D;
                let zeta = match zenithal {
                    Zenithal::Tan => u.atan(),
                    Zenithal::Arc => u,
                    Zenithal::Zea => 2.0 * self.checked_asin(u / 2.0)?,
                    Zenithal::Stg => 2.0 * (u / 2.0).atan(),
                    Zenithal::Zpn => self.zpn_zeta(u, pv, parameters.zpn)?,
                    // AIR: solve the transcendental radius for ζ (Newton).
                    Zenithal::Air => air_zeta(u, pv[1])?,
                };
                self.native_coordinate(phi, 90.0 - zeta * R2D)
            }
            Kind::Equatorial(equatorial) => {
                let [phi, theta] = match equatorial {
                    Equatorial::Car => [x, y],
                    // CEA: λ = PVi_1 (default 1); θ = asin(λ·y/(180/π)).
                    Equatorial::Cea => {
                        let lambda = pv[1];
                        [x, self.checked_asin(lambda * y / R2D)? * R2D]
                    }
                    Equatorial::Mer => [x, (2.0 * (y / R2D).exp().atan()) * R2D - 90.0],
                    Equatorial::Sfl => [x / (y * D2R).cos(), y],
                    // Hammer–Aitoff inverse (CG 2002 eq. 51).
                    Equatorial::Ait => {
                        let (u, v) = (x * D2R, y * D2R);
                        let z2 = 1.0 - (u / 4.0).powi(2) - (v / 2.0).powi(2);
                        let z = self.checked_sqrt(z2, 1.0)?;
                        let phi = 2.0 * (z * u / 2.0).atan2(2.0 * z2 - 1.0) * R2D;
                        let theta = self.checked_asin(v * z)? * R2D;
                        [phi, theta]
                    }
                    // Mollweide inverse (CG 2002 eq. 55).
                    Equatorial::Mol => {
                        let s2 = SQRT_2;
                        let gamma = self.checked_asin(y / (s2 * R2D))?;
                        let theta =
                            self.checked_asin((2.0 * gamma + (2.0 * gamma).sin()) / PI)? * R2D;
                        let phi = if gamma.cos().abs() < 1e-12 {
                            0.0
                        } else {
                            PI * x / (2.0 * s2 * gamma.cos())
                        };
                        [phi, theta]
                    }
                    // CYP inverse: φ = x/λ; θ from η = (y/(180/π))/(μ+λ).
                    Equatorial::Cyp => {
                        let (mu, lambda) = (pv[1], pv[2]);
                        let eta = (y / R2D) / (mu + lambda);
                        let theta = eta.atan2(1.0)
                            + self.checked_asin(eta * mu / (1.0 + eta * eta).sqrt())?;
                        [x / lambda, theta * R2D]
                    }
                    // PAR inverse (CG 2002 eq. 49).
                    Equatorial::Par => {
                        let theta = 3.0 * self.checked_asin(y / 180.0)?;
                        [x / (2.0 * (2.0 * theta / 3.0).cos() - 1.0), theta * R2D]
                    }
                    // Polyconic inverse (CG 2002 §5.6.1): Newton on
                    // f(θ) = X² + (Y−θ)² − 2(Y−θ)cotθ = 0, then recover φ.
                    Equatorial::Pco => {
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
                    Equatorial::Bon => {
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
                };
                self.native_coordinate(phi, theta)
            }
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
        match self.kind() {
            Kind::Cube(cube) => cube::project(cube, phi, theta),
            Kind::Healpix => healpix::project(self, phi, theta, pv),
            Kind::Perspective(Perspective::Azp) => {
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
                self.projected_coordinate(r * pr.sin(), -r * pr.cos() / gr.cos())
            }
            Kind::Perspective(Perspective::Szp) => {
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
                self.projected_coordinate(x, y)
            }
            Kind::Sin => {
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
                self.projected_coordinate(x, y)
            }
            Kind::Conic(conic) => {
                let conic = ConicConstants::new(conic, pv);
                let r = self.conic_radius(theta, conic)?;
                let cp = (conic.c * phi) * D2R;
                self.projected_coordinate(r * cp.sin(), conic.y0 - r * cp.cos())
            }
            Kind::Zenithal(zenithal) => {
                let zeta = (90.0 - theta) * D2R;
                let beyond = match zenithal {
                    // TAN images only the hemisphere above the plane; the equator diverges.
                    Zenithal::Tan => (theta * D2R).sin() <= 0.0,
                    // STG sends the antipode of its pole to infinity.
                    Zenithal::Stg => 1.0 + (theta * D2R).sin() == 0.0,
                    Zenithal::Zpn => zeta > parameters.zpn.zeta_max,
                    Zenithal::Air => theta == -90.0,
                    Zenithal::Arc | Zenithal::Zea => false,
                };
                if beyond {
                    return Err(self.world_domain_error());
                }
                let r = match zenithal {
                    Zenithal::Tan => R2D * zeta.tan(),
                    Zenithal::Arc => R2D * zeta,
                    Zenithal::Zea => 2.0 * R2D * (zeta / 2.0).sin(),
                    Zenithal::Stg => 2.0 * R2D * (zeta / 2.0).tan(),
                    Zenithal::Zpn => R2D * evaluate_zpn(zeta, pv).value,
                    Zenithal::Air => R2D * air_radius_u(zeta, pv[1]),
                };
                let p = phi * D2R;
                self.projected_coordinate(r * p.sin(), -r * p.cos())
            }
            Kind::Equatorial(equatorial) => {
                let t = theta * D2R;
                let [x, y] = match equatorial {
                    Equatorial::Car => [phi, theta],
                    Equatorial::Cea => {
                        let lambda = pv[1];
                        [phi, R2D * t.sin() / lambda]
                    }
                    // The poles are at infinity.
                    Equatorial::Mer if theta.abs() == 90.0 => {
                        return Err(self.world_domain_error());
                    }
                    Equatorial::Mer => [phi, R2D * ((45.0 + theta / 2.0) * D2R).tan().ln()],
                    Equatorial::Sfl => [phi * t.cos(), theta],
                    Equatorial::Ait => {
                        let pr = phi * D2R;
                        let gamma = R2D * (2.0 / (1.0 + t.cos() * (pr / 2.0).cos())).sqrt();
                        [2.0 * gamma * t.cos() * (pr / 2.0).sin(), gamma * t.sin()]
                    }
                    Equatorial::Mol => {
                        // Solve 2γ + sin2γ = π·sinθ for γ (Newton).
                        let s2 = SQRT_2;
                        let g = mollweide_gamma(t)?;
                        [(2.0 * s2 / PI) * phi * g.cos(), s2 * R2D * g.sin()]
                    }
                    Equatorial::Cyp => {
                        let (mu, lambda) = (pv[1], pv[2]);
                        // The latitude whose rays run parallel to the cylinder.
                        if mu + t.cos() == 0.0 {
                            return Err(self.world_domain_error());
                        }
                        [lambda * phi, R2D * (mu + lambda) * t.sin() / (mu + t.cos())]
                    }
                    Equatorial::Par => [
                        phi * (2.0 * (2.0 * t / 3.0).cos() - 1.0),
                        180.0 * (t / 3.0).sin(),
                    ],
                    Equatorial::Bon => {
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
                    Equatorial::Pco => {
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
                };
                self.projected_coordinate(x, y)
            }
        }
    }

    /// Conic radius `R_θ` (deg) for a native latitude `θ` (deg).
    fn conic_radius(self, theta: f64, conic: ConicConstants) -> Result<f64> {
        let theta_radians = theta * D2R;
        let radius = match conic.kind {
            Conic::Cop => {
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
            Conic::Coe => {
                let value =
                    1.0 + conic.sin_theta1 * conic.sin_theta2 - 2.0 * conic.c * theta_radians.sin();
                R2D / conic.c * self.checked_sqrt(value, 1.0)?
            }
            Conic::Cod => conic.y0 + (conic.theta_a_degrees - theta),
            // The far pole is at infinity unless the cone opens towards it.
            Conic::Coo if theta == -90.0 && conic.c >= 0.0 => {
                return Err(self.world_domain_error());
            }
            Conic::Coo => conic.psi * (FRAC_PI_4 - theta_radians / 2.0).tan().powf(conic.c),
        };
        if radius.is_finite() {
            Ok(radius)
        } else {
            Err(self.world_domain_error())
        }
    }

    /// Native latitude `θ` (deg) for a conic radius `R_θ` (deg).
    fn conic_theta(self, r: f64, conic: ConicConstants) -> Result<f64> {
        let theta = match conic.kind {
            Conic::Cop => {
                let tan = conic.cot_theta_a - r / (R2D * conic.cos_eta);
                conic.theta_a_degrees + tan.atan() * R2D
            }
            Conic::Coe => {
                let sin_t = (1.0 + conic.sin_theta1 * conic.sin_theta2
                    - (r * conic.c / R2D).powi(2))
                    / (2.0 * conic.c);
                self.checked_asin(sin_t)? * R2D
            }
            Conic::Cod => conic.theta_a_degrees - (r - conic.y0),
            Conic::Coo => 90.0 - 2.0 * (r / conic.psi).powf(1.0 / conic.c).atan() * R2D,
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
    fn new(kind: Conic, pv: &[f64; 21]) -> ConicConstants {
        let theta_a = pv[1] * D2R;
        let eta = pv[2] * D2R;
        let theta1 = theta_a - eta;
        let theta2 = theta_a + eta;
        let sin_theta1 = theta1.sin();
        let sin_theta2 = theta2.sin();
        let cos_eta = eta.cos();
        let cot_theta_a = 1.0 / theta_a.tan();
        let (c, y0, psi) = match kind {
            Conic::Cop => {
                let c = theta_a.sin();
                (c, R2D * cos_eta * cot_theta_a, 0.0)
            }
            Conic::Coe => {
                let c = (sin_theta1 + sin_theta2) / 2.0;
                let y0 = R2D / c
                    * (1.0 + sin_theta1 * sin_theta2 - 2.0 * c * theta_a.sin())
                        .max(0.0)
                        .sqrt();
                (c, y0, 0.0)
            }
            Conic::Cod => {
                // Equidistant: C = sinθ_a·sinη/η; Y0 = (180/π)·(η/tanη)·cotθ_a.
                let (c, k) = if eta.abs() < 1e-12 {
                    (theta_a.sin(), 1.0)
                } else {
                    (theta_a.sin() * eta.sin() / eta, eta / eta.tan())
                };
                (c, R2D * k * cot_theta_a, 0.0)
            }
            Conic::Coo => {
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
        };
        ConicConstants {
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
