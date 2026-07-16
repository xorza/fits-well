//! Typed World Coordinate System (§8).
//!
//! Parses the per-axis WCS keywords from a [`Header`] and evaluates the standard
//! pixel↔world pipeline (Greisen & Calabretta, FITS WCS papers I & II):
//!
//! ```text
//! pixel ─ CRPIX ─►  ·(PC|CD, ×CDELT)  ─►  intermediate world (deg)
//!        ─► deproject (CTYPE algorithm) ─► native sphere
//!        ─► rotate (CRVAL, LONPOLE) ─► celestial (α, δ)
//! ```
//!
//! The linear layer is `PC`+`CDELT`, `CD`, or legacy `CDELT`+`CROTA`, with general
//! matrix inversion for the reverse direction, and full `PVi_m` parameters
//! (φ₀/θ₀/LONPOLE/LATPOLE overrides plus per-projection params). Projections, via
//! the general fiducial-point pole computation: zenithal `TAN`/`SIN`/`ARC`/`STG`/
//! `ZEA`/`ZPN`/`AIR`, zenithal-perspective `AZP`/`SZP`, cylindrical `CAR`/`CEA`/
//! `MER`/`SFL`/`CYP`, all-sky `AIT`/`MOL`/`PAR`, conic `COP`/`COE`/`COD`/`COO`,
//! pseudoconic `BON`, and polyconic `PCO`. All validated against `astropy.wcs`
//! (wcslib). The unimplemented non-linear transforms — quad-cube `TSC`/`CSC`/`QSC`,
//! HEALPix `HPX`/`XPH`, and the non-linear spectral algorithms (§8.4) — are not
//! evaluated: such an axis passes through the linear stage only (its intermediate
//! world coordinate) and is listed in [`WcsView::unsupported_axes`], so a file using
//! one still reads, just with that axis not fully decoded. These source values and
//! flags are available through the immutable [`Wcs::view`] snapshot.
//!
//! Binary-table WCS (Table 22) is supported for both the pixel-list
//! ([`Header::wcs_pixel_list`](crate::Header::wcs_pixel_list)) and vector-cell
//! ([`Header::wcs_array_column`](crate::Header::wcs_array_column)) forms.
//!
//! Pixel↔world yields celestial coordinates in the frame the file declares
//! (`RADESYS`/`EQUINOX`); converting *between* reference frames is astrometry
//! beyond the FITS standard and is intentionally out of scope. Transform methods
//! write into caller-owned coordinate storage for allocation-free scalar loops and
//! return explicit errors for invalid projection domains or failed iterations.

use std::f64::consts::FRAC_PI_2;
use std::f64::consts::FRAC_PI_4;
use std::f64::consts::PI;
use std::f64::consts::SQRT_2;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

const R2D: f64 = 180.0 / PI;
const D2R: f64 = PI / 180.0;
const DOMAIN_TOLERANCE: f64 = 1e-12;
const NEWTON_RESIDUAL_TOLERANCE: f64 = 1e-12;

/// The §8.4 spectral coordinate types (the 4-character `CTYPE` prefix). A bare
/// type is sampled linearly (handled by the generic linear axis); a `TTTT-AAA`
/// algorithm suffix means non-linear sampling, which is not yet evaluated.
const SPECTRAL_TYPES: &[&str] = &[
    "FREQ", "ENER", "WAVN", "VRAD", "WAVE", "VOPT", "ZOPT", "AWAV", "VELO", "BETA",
];

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
];

impl Projection {
    fn from_code(code: &str) -> Option<Projection> {
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

    fn code(self) -> &'static str {
        PROJECTIONS
            .iter()
            .find(|&&(_, projection, _)| projection == self)
            .map(|&(code, ..)| code)
            .expect("every Projection variant is listed in PROJECTIONS")
    }
}

#[derive(Debug, Clone)]
struct PreparedProjection {
    projection: Projection,
    family: Family,
    pv: [f64; 21],
    conic: Option<ConicConstants>,
}

#[derive(Debug, Clone, Copy)]
struct ConicConstants {
    c: f64,
    y0: f64,
    theta_a: f64,
    theta_a_degrees: f64,
    cos_eta: f64,
    cot_theta_a: f64,
    sin_theta1: f64,
    sin_theta2: f64,
    psi: f64,
}

#[derive(Debug, Clone, Copy)]
struct NativeCoordinate {
    phi: f64,
    theta: f64,
}

#[derive(Debug, Clone, Copy)]
struct ProjectedCoordinate {
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct CelestialCoordinate {
    ra: f64,
    dec: f64,
}

#[derive(Debug, Clone, Copy)]
struct SzpVertex {
    x: f64,
    y: f64,
    z: f64,
}

#[derive(Debug, Clone, Copy)]
struct CelestialCorrection {
    longitude_axis: usize,
    latitude_axis: usize,
    longitude_delta: f64,
    latitude_delta: f64,
}

impl PreparedProjection {
    fn new(projection: Projection, pv: [f64; 21]) -> PreparedProjection {
        let family = projection.family();
        assert!(
            family != Family::Conic || pv[1] != 0.0,
            "conic projection requires non-zero PVi_1"
        );
        let conic = (family == Family::Conic).then(|| ConicConstants::new(projection, &pv));
        PreparedProjection {
            projection,
            family,
            pv,
            conic,
        }
    }

    fn domain_error(&self) -> FitsError {
        FitsError::WcsProjectionDomain {
            projection: self.projection.code(),
        }
    }

    fn checked_asin(&self, value: f64) -> Result<f64> {
        if !value.is_finite()
            || !(-1.0 - DOMAIN_TOLERANCE..=1.0 + DOMAIN_TOLERANCE).contains(&value)
        {
            return Err(self.domain_error());
        }
        Ok(value.clamp(-1.0, 1.0).asin())
    }

    fn checked_sqrt(&self, value: f64, scale: f64) -> Result<f64> {
        if !value.is_finite() || value < -DOMAIN_TOLERANCE * scale.abs().max(1.0) {
            return Err(self.domain_error());
        }
        Ok(value.max(0.0).sqrt())
    }

    fn native_coordinate(&self, phi: f64, theta: f64) -> Result<NativeCoordinate> {
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

    fn projected_coordinate(&self, x: f64, y: f64) -> Result<ProjectedCoordinate> {
        if !x.is_finite() || !y.is_finite() {
            return Err(self.domain_error());
        }
        Ok(ProjectedCoordinate { x, y })
    }

    /// The fiducial point `(φ₀, θ₀)` in degrees. Zenithal (incl. the perspective
    /// `AZP`/`SZP`): `(0, 90)`; conics: `(0, θ_a)` where `θ_a = PVi_1`; else `(0, 0)`.
    fn reference_point(&self) -> NativeCoordinate {
        match self.family {
            Family::Zenithal | Family::ZenithalPerspective => NativeCoordinate {
                phi: 0.0,
                theta: 90.0,
            },
            Family::Conic => NativeCoordinate {
                phi: 0.0,
                theta: self.pv[1],
            },
            Family::Other => NativeCoordinate {
                phi: 0.0,
                theta: 0.0,
            },
        }
    }

    /// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
    fn deproject(&self, x: f64, y: f64) -> Result<NativeCoordinate> {
        let projection = self.projection;
        let pv = &self.pv;
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
            let discriminant = qb * qb - 4.0 * qa * qc;
            let disc = self.checked_sqrt(discriminant, qb * qb + (4.0 * qa * qc).abs())?;
            let s1 = (-qb - disc) / (2.0 * qa);
            let s2 = (-qb + disc) / (2.0 * qa);
            // σ ∈ [0, 2]; prefer the visible-hemisphere root (smaller σ).
            let valid_sigma = |sigma: f64| {
                sigma.is_finite() && (-DOMAIN_TOLERANCE..=2.0 + DOMAIN_TOLERANCE).contains(&sigma)
            };
            let sigma = match (valid_sigma(s1), valid_sigma(s2)) {
                (true, true) => s1.min(s2),
                (true, false) => s1,
                (false, true) => s2,
                (false, false) => return Err(self.domain_error()),
            }
            .clamp(0.0, 2.0);
            let theta = self.checked_asin(1.0 - sigma)?;
            let (a, b) = (a0 + a1 * sigma, b0 + b1 * sigma);
            let phi = a.atan2(b);
            return self.native_coordinate(phi * R2D, theta * R2D);
        }
        if self.family == Family::Conic {
            let conic = self.conic.expect("conic projection has prepared constants");
            let s = pv[1].signum();
            let r = s * x.hypot(conic.y0 - y);
            let phi = (s * x).atan2(s * (conic.y0 - y)) * R2D / conic.c;
            return self.native_coordinate(phi, self.conic_theta(r)?);
        }
        if self.family == Family::Zenithal {
            let r = x.hypot(y);
            let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
            // Colatitude ζ (rad) from the radius, per projection.
            let u = r / R2D;
            let zeta = match projection {
                Projection::Tan => u.atan(),
                Projection::Sin => self.checked_asin(u)?,
                Projection::Arc => u,
                Projection::Zea => 2.0 * self.checked_asin(u / 2.0)?,
                Projection::Stg => 2.0 * (u / 2.0).atan(),
                // ZPN: solve Σ Pₘ ζᵐ = u for ζ (Newton from ζ = u).
                Projection::Zpn => zpn_zeta(u, pv)?,
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
                    let lambda = pv.get(1).filter(|&&v| v != 0.0).copied().unwrap_or(1.0);
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
                    let (mu, lambda) = (
                        pv[1],
                        pv.get(2).filter(|&&v| v != 0.0).copied().unwrap_or(1.0),
                    );
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
    fn project(&self, phi: f64, theta: f64) -> Result<ProjectedCoordinate> {
        if !phi.is_finite()
            || !theta.is_finite()
            || !(-90.0 - DOMAIN_TOLERANCE..=90.0 + DOMAIN_TOLERANCE).contains(&theta)
        {
            return Err(self.domain_error());
        }
        let theta = theta.clamp(-90.0, 90.0);
        let projection = self.projection;
        let pv = &self.pv;
        if matches!(projection, Projection::Azp) {
            let (mu, gr) = (pv[1], pv[2] * D2R);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let denom = (mu + tr.sin()) + tr.cos() * pr.cos() * gr.tan();
            let r = R2D * (mu + 1.0) * tr.cos() / denom;
            return self.projected_coordinate(r * pr.sin(), -r * pr.cos() / gr.cos());
        }
        if matches!(projection, Projection::Szp) {
            let vertex = szp_vertex(pv);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let sigma = 1.0 - tr.sin();
            let denom = vertex.z - sigma;
            let x = R2D * (vertex.z * tr.cos() * pr.sin() - vertex.x * sigma) / denom;
            let y = R2D * (-vertex.z * tr.cos() * pr.cos() - vertex.y * sigma) / denom;
            return self.projected_coordinate(x, y);
        }
        if self.family == Family::Conic {
            let conic = self.conic.expect("conic projection has prepared constants");
            let r = self.conic_radius(theta)?;
            let cp = (conic.c * phi) * D2R;
            return self.projected_coordinate(r * cp.sin(), conic.y0 - r * cp.cos());
        }
        if self.family == Family::Zenithal {
            let zeta = (90.0 - theta) * D2R;
            let r = match projection {
                Projection::Tan => R2D * zeta.tan(),
                Projection::Sin => R2D * zeta.sin(),
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
                    let lambda = pv.get(1).filter(|&&v| v != 0.0).copied().unwrap_or(1.0);
                    [phi, R2D * t.sin() / lambda]
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
                    let (mu, lambda) = (
                        pv[1],
                        pv.get(2).filter(|&&v| v != 0.0).copied().unwrap_or(1.0),
                    );
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
    fn conic_radius(&self, theta: f64) -> Result<f64> {
        let conic = self.conic.expect("conic projection has prepared constants");
        let theta_radians = theta * D2R;
        let radius = match self.projection {
            Projection::Cop => {
                R2D * conic.cos_eta * (conic.cot_theta_a - (theta_radians - conic.theta_a).tan())
            }
            Projection::Coe => {
                let value =
                    1.0 + conic.sin_theta1 * conic.sin_theta2 - 2.0 * conic.c * theta_radians.sin();
                R2D / conic.c * self.checked_sqrt(value, 1.0)?
            }
            Projection::Cod => conic.y0 + (conic.theta_a_degrees - theta),
            Projection::Coo => conic.psi * (FRAC_PI_4 - theta_radians / 2.0).tan().powf(conic.c),
            _ => unreachable!(),
        };
        if radius.is_finite() {
            Ok(radius)
        } else {
            Err(self.domain_error())
        }
    }

    /// Native latitude `θ` (deg) for a conic radius `R_θ` (deg).
    fn conic_theta(&self, r: f64) -> Result<f64> {
        let conic = self.conic.expect("conic projection has prepared constants");
        let theta = match self.projection {
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
            theta_a,
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
        projection: projection.code(),
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

/// Invert the ZPN polynomial for ζ given `u = R/(180/π)` (Newton from ζ = u).
fn zpn_zeta(u: f64, pv: &[f64; 21]) -> Result<f64> {
    solve_newton(Projection::Zpn, u, |zeta| {
        let evaluation = evaluate_zpn(zeta, pv);
        NewtonEvaluation {
            residual: evaluation.value - u,
            derivative: evaluation.derivative,
        }
    })
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

/// Immutable source metadata for one WCS axis.
#[derive(Debug, Clone, PartialEq)]
pub struct WcsAxis {
    /// The `CTYPEi` string.
    pub ctype: String,
    /// `CRVALi` — world coordinate at the reference pixel.
    pub crval: f64,
    /// `CRPIXi` — reference pixel (1-based).
    pub crpix: f64,
}

/// A read-only snapshot of a parsed WCS's source metadata and support status.
#[derive(Debug, Clone, Copy)]
pub struct WcsView<'a> {
    pub axes: &'a [WcsAxis],
    /// Zero-based axes whose non-linear transform is not evaluated.
    pub unsupported_axes: &'a [usize],
}

/// A parsed world coordinate system for one (optionally alternate) axis set.
#[derive(Debug, Clone)]
pub struct Wcs {
    axes: Vec<WcsAxis>,
    /// Linear transform `A` mapping `(pixel − CRPIX)` to intermediate world
    /// coordinates: `PCi_j × CDELTi`, or `CDi_j` directly. Row-major `naxis²`.
    matrix: Vec<f64>,
    /// Inverse of `matrix`, for world→pixel.
    inverse: Vec<f64>,
    /// The (longitude axis, latitude axis, projection, celestial pole) when a
    /// celestial pair is present; `None` for an all-linear system.
    celestial: Option<Celestial>,
    /// Axes (0-based) whose non-linear transform is not evaluated — an unsupported
    /// projection (quad-cube/HEALPix) or a non-linear spectral algorithm (§8.3/§8.4).
    /// [`Wcs::pixel_to_world`] returns their *intermediate* world coordinate (the
    /// linear stage only), not a fully decoded celestial/spectral value.
    unsupported_axes: Vec<usize>,
}

/// The rotation from native to celestial coordinates: the celestial pole
/// `(α_p, δ_p)` and the native longitude of the pole `φ_p` (LONPOLE), all degrees.
#[derive(Debug, Clone, Copy, PartialEq)]
struct CelestialPole {
    ra: f64,
    dec: f64,
    lonpole: f64,
}

#[derive(Debug, Clone)]
struct Celestial {
    lng: usize,
    lat: usize,
    projection: PreparedProjection,
    /// The native→celestial pole, computed from the fiducial point.
    pole: CelestialPole,
}

impl Wcs {
    /// Parse the primary WCS (`alt = None`) or an alternate description
    /// (`alt = Some('A'..='Z')`) from `header`. The public entry point is
    /// [`Header::wcs`](crate::Header::wcs), which forwards here.
    pub(crate) fn from_header(header: &Header, alt: Option<char>) -> Result<Wcs> {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        let naxis_value = match header.get_integer(key!("WCSAXES{a}").as_str())? {
            Some(naxis) => Some(naxis),
            None => header.get_integer("NAXIS")?,
        }
        .ok_or(FitsError::MissingKeyword { name: "WCSAXES" })?;
        let naxis = axis_count(naxis_value, "WCSAXES")?;

        let ctype: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CTYPE{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let mut crval = axis_vec(header, "CRVAL", &a, naxis, 0.0)?;
        let crpix = axis_vec(header, "CRPIX", &a, naxis, 0.0)?;
        let cdelt = axis_vec(header, "CDELT", &a, naxis, 1.0)?;
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CUNIT{i}{a}").as_str())
                    .map(|value| value.unwrap_or("").to_string())
            })
            .collect::<Result<_>>()?;
        let celestial_axes = find_celestial(&ctype)?;

        // Axes whose non-linear transform this library doesn't evaluate — an
        // unsupported celestial projection (quad-cube `TSC`/`CSC`/`QSC`, HEALPix
        // `HPX`/`XPH`) or a non-linearly-sampled spectral axis (§8.4). Rather than
        // fail the whole WCS, these pass through the linear stage only, so
        // `pixel_to_world` returns their *intermediate* world coordinate;
        // `unsupported_axes` records them so a caller never mistakes that for a
        // fully-decoded sky/spectral value.
        let mut unsupported_axes = nonlinear_unsupported_axes(&ctype);

        // Build the linear transform A. Precedence: CD, then PC×CDELT, then the
        // legacy CROTA rotation, then a bare CDELT diagonal.
        let has_cd = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get(key!("CD{i}_{j}{a}").as_str()).is_some()));
        let has_pc = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get(key!("PC{i}_{j}{a}").as_str()).is_some()));
        let has_crota = (1..=naxis).any(|i| header.get(key!("CROTA{i}{a}").as_str()).is_some());
        // §8: the PC/CDELT, CD, and legacy CROTA conventions are mutually exclusive.
        if [has_cd, has_pc, has_crota]
            .into_iter()
            .filter(|&present| present)
            .count()
            > 1
        {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "PC, CD, and CROTA conventions overlap",
            });
        }
        let mut matrix = vec![0.0; naxis * naxis];
        if has_cd {
            for i in 0..naxis {
                for j in 0..naxis {
                    matrix[i * naxis + j] = header
                        .get_real(key!("CD{}_{}{a}", i + 1, j + 1).as_str())?
                        .unwrap_or(0.0);
                }
            }
        } else {
            for i in 0..naxis {
                for j in 0..naxis {
                    let pc = header
                        .get_real(key!("PC{}_{}{a}", i + 1, j + 1).as_str())?
                        .unwrap_or(if i == j { 1.0 } else { 0.0 });
                    matrix[i * naxis + j] = cdelt[i] * pc;
                }
            }
            // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC
            // was given, per the convention that CROTA and PC are exclusive).
            if !has_pc && let Some((lng, lat, _)) = celestial_axes {
                let rho = first_real(
                    header,
                    key!("CROTA{}{a}", lat + 1).as_str(),
                    key!("CROTA{}{a}", lng + 1).as_str(),
                )?
                .unwrap_or(0.0);
                if rho != 0.0 {
                    let (c, s) = ((rho * D2R).cos(), (rho * D2R).sin());
                    matrix[lng * naxis + lng] = cdelt[lng] * c;
                    matrix[lng * naxis + lat] = -cdelt[lat] * s;
                    matrix[lat * naxis + lng] = cdelt[lng] * s;
                    matrix[lat * naxis + lat] = cdelt[lat] * c;
                }
            }
        }
        // §8.2: CRVAL/CDELT are in CUNITia units, but the projection math runs in
        // degrees — scale each celestial axis's reference value and its matrix row
        // (the inverse is computed after, so both directions stay consistent).
        if let Some((lng, lat, _)) = celestial_axes {
            for ax in [lng, lat] {
                let f = unit_to_degrees(&cunit[ax]);
                crval[ax] *= f;
                for j in 0..naxis {
                    matrix[ax * naxis + j] *= f;
                }
            }
        }
        let inverse = invert(&matrix, naxis).ok_or(FitsError::InvalidValue {
            card: "singular WCS transform matrix".to_string(),
        })?;

        let celestial = match celestial_axes {
            Some((lng, lat, proj)) => {
                // Latitude-axis PVi_0..PVi_20 — the projection parameters.
                let mut pv = [0.0; 21];
                for (m, value) in pv.iter_mut().enumerate() {
                    *value = header
                        .get_real(key!("PV{}_{m}{a}", lat + 1).as_str())?
                        .unwrap_or(0.0);
                }
                let family = proj.family();
                // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be
                // non-zero; θ_a = 0 (absent, or explicitly 0) is a degenerate cone
                // (`1/tan 0`). Treat it like an unimplemented projection — flag the
                // axes and skip deprojection so they pass through the linear stage
                // (an intermediate world coordinate) rather than returning NaN.
                if family == Family::Conic && pv[1] == 0.0 {
                    unsupported_axes.push(lng);
                    unsupported_axes.push(lat);
                    unsupported_axes.sort_unstable();
                    None
                } else {
                    let projection = PreparedProjection::new(proj, pv);
                    // Fiducial point: projection default, overridable by PVi_1a/
                    // PVi_2a on the longitude axis (§8.3).
                    let reference = projection.reference_point();
                    let (mut phi0, mut theta0) = (reference.phi, reference.theta);
                    if let Some(v) = header.get_real(key!("PV{}_1{a}", lng + 1).as_str())? {
                        phi0 = v;
                    }
                    if let Some(v) = header.get_real(key!("PV{}_2{a}", lng + 1).as_str())? {
                        theta0 = v;
                    }
                    let (alpha0, delta0) = (crval[lng], crval[lat]);
                    // LONPOLE (= LONPOLEa or PVi_3a): default φ0 if δ0 ≥ θ0, else φ0 + 180°.
                    let phip = first_real(
                        header,
                        key!("LONPOLE{a}").as_str(),
                        key!("PV{}_3{a}", lng + 1).as_str(),
                    )?
                    .unwrap_or(if delta0 >= theta0 {
                        phi0
                    } else {
                        phi0 + 180.0
                    });
                    // LATPOLE (= LATPOLEa or PVi_4a): default 90°.
                    let thetap = first_real(
                        header,
                        key!("LATPOLE{a}").as_str(),
                        key!("PV{}_4{a}", lng + 1).as_str(),
                    )?
                    .unwrap_or(90.0);
                    let pole = compute_pole(phi0, theta0, alpha0, delta0, phip, thetap);
                    Some(Celestial {
                        lng,
                        lat,
                        projection,
                        pole,
                    })
                }
            }
            None => None,
        };

        let axes = ctype
            .into_iter()
            .enumerate()
            .map(|(i, ctype)| WcsAxis {
                ctype,
                crval: crval[i],
                crpix: crpix[i],
            })
            .collect();
        Ok(Wcs {
            axes,
            matrix,
            inverse,
            celestial,
            unsupported_axes,
        })
    }

    /// Borrow the parsed axis metadata and unsupported-axis flags without exposing
    /// the transform's invariant-bearing owned state for mutation.
    pub fn view(&self) -> WcsView<'_> {
        WcsView {
            axes: &self.axes,
            unsupported_axes: &self.unsupported_axes,
        }
    }

    /// Build a WCS for a binary-table **pixel list** (event list, §8.5, Table 22):
    /// `columns` lists the 1-based table column numbers forming the coordinate axes
    /// in order. Reads the column-indexed keyword family — `TCTYPn`/`TCRPXn`/
    /// `TCRVLn`/`TCDLTn`/`TCROTn`/`TCUNIn`, the `TPCn_ka`/`TCDn_ka` matrices, and
    /// `TPVn_ma` parameters — then evaluates it through the same pipeline as image
    /// WCS (so projections, `CUNIT`, and the pole computation all apply).
    pub(crate) fn from_pixel_list(
        header: &Header,
        columns: &[usize],
        alt: Option<char>,
    ) -> Result<Wcs> {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        // Translate the column-indexed keywords into an equivalent image header,
        // mapping column number `cN` → axis index `i+1`.
        let mut h = Header::new();
        h.set("WCSAXES", columns.len() as i64);
        for (i, &c) in columns.iter().enumerate() {
            let ax = i + 1;
            if let Some(t) = header.get_text(key!("TCTYP{c}{a}").as_str())? {
                h.set(key!("CTYPE{ax}").as_str(), t);
            }
            for (root, dst) in [
                ("TCRPX", "CRPIX"),
                ("TCRVL", "CRVAL"),
                ("TCDLT", "CDELT"),
                ("TCROT", "CROTA"),
            ] {
                if let Some(v) = header.get_real(key!("{root}{c}{a}").as_str())? {
                    h.set(key!("{dst}{ax}").as_str(), v);
                }
            }
            if let Some(t) = header.get_text(key!("TCUNI{c}{a}").as_str())? {
                h.set(key!("CUNIT{ax}").as_str(), t);
            }
            for m in 0..=20 {
                if let Some(v) = header.get_real(key!("TPV{c}_{m}{a}").as_str())? {
                    h.set(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear-transform matrices: TPCn_ka / TCDn_ka, indexed by column pair.
        for (i, &ci) in columns.iter().enumerate() {
            for (j, &cj) in columns.iter().enumerate() {
                if let Some(v) = header.get_real(key!("TPC{ci}_{cj}{a}").as_str())? {
                    h.set(key!("PC{}_{}", i + 1, j + 1).as_str(), v);
                }
                if let Some(v) = header.get_real(key!("TCD{ci}_{cj}{a}").as_str())? {
                    h.set(key!("CD{}_{}", i + 1, j + 1).as_str(), v);
                }
            }
        }
        if let Some(v) = header.get_real(key!("LONP{a}").as_str())? {
            h.set("LONPOLE", v);
        }
        if let Some(v) = header.get_real(key!("LATP{a}").as_str())? {
            h.set("LATPOLE", v);
        }
        Wcs::from_header(&h, None)
    }

    /// Build a WCS for an image stored in a binary-table **vector cell** (§8,
    /// Table 22): `column` is the 1-based table column whose cells hold a
    /// multidimensional array. Reads the axis-and-column-indexed keyword family —
    /// `iCTYPn`/`iCRVLn`/`iCDLTn`/`jCRPXn`/`iCROTn`/`iCUNIn`, the `ijPCn`/`ijCDn`
    /// matrices, and `iPVn_ma` (or abbreviated `iVn_ma`) parameters, where `i`/`j`
    /// are the array axis and `n` the column — then evaluates it through the same
    /// pipeline as image WCS. The rank is taken from `WCAXna`, else inferred from
    /// the highest axis index present.
    pub(crate) fn from_array_column(
        header: &Header,
        column: usize,
        alt: Option<char>,
    ) -> Result<Wcs> {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        let naxis = match header.get_integer(key!("WCAX{column}{a}").as_str())? {
            Some(value) => axis_count(value, "WCAXn")?,
            None => (1..=99)
                .rev()
                .find(|&i| {
                    header.get(key!("{i}CTYP{column}{a}").as_str()).is_some()
                        || ["CRVL", "CDLT", "CRPX"]
                            .iter()
                            .any(|r| header.get(key!("{i}{r}{column}{a}").as_str()).is_some())
                })
                .unwrap_or(0),
        };
        if naxis == 0 {
            return Err(FitsError::MissingKeyword { name: "iCTYPn" });
        }
        let mut h = Header::new();
        h.set("WCSAXES", naxis as i64);
        for ax in 1..=naxis {
            if let Some(t) = header.get_text(key!("{ax}CTYP{column}{a}").as_str())? {
                h.set(key!("CTYPE{ax}").as_str(), t);
            }
            if let Some(t) = header.get_text(key!("{ax}CUNI{column}{a}").as_str())? {
                h.set(key!("CUNIT{ax}").as_str(), t);
            }
            for (root, dst) in [
                ("CRPX", "CRPIX"),
                ("CRVL", "CRVAL"),
                ("CDLT", "CDELT"),
                ("CROT", "CROTA"),
            ] {
                if let Some(v) = header.get_real(key!("{ax}{root}{column}{a}").as_str())? {
                    h.set(key!("{dst}{ax}").as_str(), v);
                }
            }
            // PVi_m arrives as `iPVn_ma`, or the abbreviated `iVn_ma`.
            for m in 0..=20 {
                if let Some(v) = first_real(
                    header,
                    key!("{ax}PV{column}_{m}{a}").as_str(),
                    key!("{ax}V{column}_{m}{a}").as_str(),
                )? {
                    h.set(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear-transform matrices: `ijPCn` / `ijCDn`, indexed by axis pair.
        for i in 1..=naxis {
            for j in 1..=naxis {
                if let Some(v) = header.get_real(key!("{i}{j}PC{column}{a}").as_str())? {
                    h.set(key!("PC{i}_{j}").as_str(), v);
                }
                if let Some(v) = header.get_real(key!("{i}{j}CD{column}{a}").as_str())? {
                    h.set(key!("CD{i}_{j}").as_str(), v);
                }
            }
        }
        Wcs::from_header(&h, None)
    }

    /// Map 1-based pixel coordinates into caller-owned world-coordinate storage.
    /// Celestial axes return `(α, δ)` in degrees; other axes return `CRVAL + ` the
    /// linear value.
    /// Both slices must contain exactly one value per axis; debug builds assert
    /// this hot-path precondition. On a projection error, the celestial output
    /// pair is set to NaN and the failure is returned.
    pub fn pixel_to_world(&self, pixel: &[f64], world: &mut [f64]) -> Result<()> {
        let naxis = self.axes.len();
        debug_assert_eq!(pixel.len(), naxis, "pixel coordinate count");
        debug_assert_eq!(world.len(), naxis, "world coordinate count");
        for (i, row) in self.matrix.chunks_exact(naxis).enumerate() {
            world[i] = row
                .iter()
                .enumerate()
                .map(|(j, &factor)| factor * (pixel[j] - self.axes[j].crpix))
                .sum();
        }
        let celestial_intermediate = self
            .celestial
            .as_ref()
            .map(|c| [world[c.lng], world[c.lat]]);
        for (value, axis) in world.iter_mut().zip(&self.axes) {
            *value += axis.crval;
        }
        if let Some(c) = &self.celestial {
            let [x, y] = celestial_intermediate.unwrap();
            let native = match c.projection.deproject(x, y) {
                Ok(native) => native,
                Err(error) => {
                    world[c.lng] = f64::NAN;
                    world[c.lat] = f64::NAN;
                    return Err(error);
                }
            };
            let celestial = native_to_celestial(c.pole, native.phi, native.theta);
            world[c.lng] = celestial.ra;
            world[c.lat] = celestial.dec;
        }
        Ok(())
    }

    /// Map world coordinates into caller-owned 1-based pixel-coordinate storage,
    /// the inverse of [`Wcs::pixel_to_world`].
    /// Both slices must contain exactly one value per axis; debug builds assert
    /// this hot-path precondition. On a projection error, every pixel output is
    /// set to NaN because every inverse-matrix row may depend on a celestial axis.
    pub fn world_to_pixel(&self, world: &[f64], pixel: &mut [f64]) -> Result<()> {
        let naxis = self.axes.len();
        debug_assert_eq!(world.len(), naxis, "world coordinate count");
        debug_assert_eq!(pixel.len(), naxis, "pixel coordinate count");
        let celestial_correction = if let Some(c) = self.celestial.as_ref() {
            if !world[c.lng].is_finite()
                || !world[c.lat].is_finite()
                || !(-90.0 - DOMAIN_TOLERANCE..=90.0 + DOMAIN_TOLERANCE).contains(&world[c.lat])
            {
                pixel.fill(f64::NAN);
                return Err(c.projection.domain_error());
            }
            let native = celestial_to_native(c.pole, world[c.lng], world[c.lat]);
            let projected = match c.projection.project(native.phi, native.theta) {
                Ok(projected) => projected,
                Err(error) => {
                    pixel.fill(f64::NAN);
                    return Err(error);
                }
            };
            Some(CelestialCorrection {
                longitude_axis: c.lng,
                latitude_axis: c.lat,
                longitude_delta: projected.x - (world[c.lng] - self.axes[c.lng].crval),
                latitude_delta: projected.y - (world[c.lat] - self.axes[c.lat].crval),
            })
        } else {
            None
        };
        for (i, row) in self.inverse.chunks_exact(naxis).enumerate() {
            let mut offset: f64 = row
                .iter()
                .enumerate()
                .map(|(j, &factor)| factor * (world[j] - self.axes[j].crval))
                .sum();
            if let Some(correction) = celestial_correction {
                offset += row[correction.longitude_axis] * correction.longitude_delta
                    + row[correction.latitude_axis] * correction.latitude_delta;
            }
            pixel[i] = offset + self.axes[i].crpix;
        }
        Ok(())
    }
}

/// Which celestial coordinate an axis carries.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CelestialAxis {
    Longitude,
    Latitude,
}

/// The celestial coordinate an axis carries, from its `CTYPE` head (§8.2): `RA` and
/// the `xLON`/`yzLN` forms are longitudes; `DEC` and `xLAT`/`yzLT` are latitudes;
/// `None` for any non-celestial axis. One classifier shared by [`find_celestial`]
/// and [`nonlinear_unsupported_axes`] so the two cannot drift.
fn celestial_axis(ctype: &str) -> Option<CelestialAxis> {
    let head = ctype.split('-').next().unwrap_or("").trim();
    if head == "RA" || head.ends_with("LON") || (head.len() == 4 && head.ends_with("LN")) {
        Some(CelestialAxis::Longitude)
    } else if head == "DEC" || head.ends_with("LAT") || (head.len() == 4 && head.ends_with("LT")) {
        Some(CelestialAxis::Latitude)
    } else {
        None
    }
}

/// The trailing projection/algorithm code of a `CTYPE` (`RA---TAN` → `TAN`); `None`
/// when there is no hyphen-delimited suffix (a bare `RA`/`GLON`).
fn projection_code(ctype: &str) -> Option<&str> {
    ctype
        .rsplit_once('-')
        .map(|(_, code)| code)
        .filter(|c| !c.is_empty())
}

/// Axis indices (0-based) whose non-linear transform this library does not
/// evaluate: a celestial axis whose 3-letter projection code is unimplemented
/// (quad-cube/HEALPix), or a non-linearly-sampled spectral axis (`TTTT-AAA`,
/// §8.4). Such an axis is taken through the linear stage only (its intermediate
/// world coordinate). The supported projections and a bare spectral type (which
/// is genuinely linear) are not flagged.
fn nonlinear_unsupported_axes(ctype: &[String]) -> Vec<usize> {
    let mut out = Vec::new();
    for (i, t) in ctype.iter().enumerate() {
        if celestial_axis(t).is_some() {
            if let Some(code) = projection_code(t)
                && code.len() == 3
                && Projection::from_code(code).is_none()
            {
                out.push(i);
            }
        } else {
            let head = t.split('-').next().unwrap_or("").trim_end();
            if SPECTRAL_TYPES.contains(&head)
                && t.get(5..).map(str::trim).is_some_and(|s| !s.is_empty())
            {
                out.push(i);
            }
        }
    }
    out
}

/// Degrees per `CUNITia` angle unit; `1.0` for an absent, unknown, or `deg` unit.
fn unit_to_degrees(unit: &str) -> f64 {
    match unit.trim() {
        "arcmin" => 1.0 / 60.0,
        "arcsec" => 1.0 / 3600.0,
        "mas" => 1.0 / 3_600_000.0,
        "rad" => R2D,
        _ => 1.0, // "deg", "", or anything unrecognized
    }
}

/// Locate the celestial longitude/latitude axis pair and their shared projection,
/// or `None` if the header has no complete celestial pair. Errors if the two axes
/// declare *different* projection codes — §8.2 requires them to match, so a
/// mismatch (or one axis projected and the other not) is a malformed header rather
/// than grounds to silently pick one.
fn find_celestial(ctype: &[String]) -> Result<Option<(usize, usize, Projection)>> {
    let mut lng = None;
    let mut lat = None;
    for (i, t) in ctype.iter().enumerate() {
        match celestial_axis(t) {
            Some(CelestialAxis::Longitude) => lng = lng.or(Some(i)),
            Some(CelestialAxis::Latitude) => lat = lat.or(Some(i)),
            None => {}
        }
    }
    let (Some(lng), Some(lat)) = (lng, lat) else {
        return Ok(None);
    };
    if projection_code(&ctype[lng]) != projection_code(&ctype[lat]) {
        return Err(FitsError::ConflictingWcsKeywords {
            detail: "celestial longitude and latitude axes declare different projections",
        });
    }
    Ok(projection_code(&ctype[lng])
        .and_then(Projection::from_code)
        .map(|proj| (lng, lat, proj)))
}

/// Native spherical (φ, θ) → celestial (α, δ), all degrees, given the celestial
/// pole `(α_p, δ_p, φ_p)` (CG 2002 eq. 2).
fn native_to_celestial(pole: CelestialPole, phi: f64, theta: f64) -> CelestialCoordinate {
    let CelestialPole {
        ra: ap,
        dec: dp,
        lonpole: fp,
    } = pole;
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
fn celestial_to_native(pole: CelestialPole, ra: f64, dec: f64) -> NativeCoordinate {
    let CelestialPole {
        ra: ap,
        dec: dp,
        lonpole: fp,
    } = pole;
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

/// Compute the celestial pole `(α_p, δ_p, φ_p)` from the fiducial point
/// `(φ₀, θ₀) → (α₀, δ₀)`, `φ_p` (LONPOLE), and `θ_p` (LATPOLE) (CG 2002 §2.4).
/// Zenithal (`θ₀ = 90°`) reduces to `(α₀, δ₀, φ_p)`.
fn compute_pole(phi0: f64, theta0: f64, a0: f64, d0: f64, phip: f64, thetap: f64) -> CelestialPole {
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

/// Read `PREFIX1..PREFIXn` (with alternate suffix) into a vector, defaulting
/// missing entries.
fn axis_vec(
    header: &Header,
    prefix: &str,
    alt: &str,
    naxis: usize,
    default: f64,
) -> Result<Vec<f64>> {
    (1..=naxis)
        .map(|i| {
            header
                .get_real(key!("{prefix}{i}{alt}").as_str())
                .map(|value| value.unwrap_or(default))
        })
        .collect()
}

fn axis_count(value: i64, name: &'static str) -> Result<usize> {
    let count = usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name })?;
    if !(1..=999).contains(&count) {
        return Err(FitsError::KeywordOutOfRange { name });
    }
    Ok(count)
}

fn first_real(header: &Header, first: &str, second: &str) -> Result<Option<f64>> {
    match header.get_real(first)? {
        Some(value) => Ok(Some(value)),
        None => header.get_real(second),
    }
}

/// Invert a row-major `n×n` matrix by Gauss–Jordan elimination with partial
/// pivoting. Returns `None` if singular.
fn invert(m: &[f64], n: usize) -> Option<Vec<f64>> {
    let mut a = m.to_vec();
    let mut inv = vec![0.0; n * n];
    for i in 0..n {
        inv[i * n + i] = 1.0;
    }
    for col in 0..n {
        // Partial pivot: largest magnitude in this column at or below the diagonal.
        let mut pivot = col;
        for r in (col + 1)..n {
            if a[r * n + col].abs() > a[pivot * n + col].abs() {
                pivot = r;
            }
        }
        if a[pivot * n + col].abs() < 1e-300 {
            return None;
        }
        if pivot != col {
            for k in 0..n {
                a.swap(col * n + k, pivot * n + k);
                inv.swap(col * n + k, pivot * n + k);
            }
        }
        let d = a[col * n + col];
        for k in 0..n {
            a[col * n + k] /= d;
            inv[col * n + k] /= d;
        }
        for r in 0..n {
            if r == col {
                continue;
            }
            let f = a[r * n + col];
            if f != 0.0 {
                for k in 0..n {
                    a[r * n + k] -= f * a[col * n + k];
                    inv[r * n + k] -= f * inv[col * n + k];
                }
            }
        }
    }
    Some(inv)
}

/// Normalize an angle to `[0, 360)` degrees.
fn norm360(a: f64) -> f64 {
    a.rem_euclid(360.0)
}

/// Normalize an angle to `[−180, 180)` degrees.
fn norm180(a: f64) -> f64 {
    (a + 180.0).rem_euclid(360.0) - 180.0
}

#[cfg(test)]
mod tests;
