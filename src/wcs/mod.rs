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
//! world coordinate) and is listed in [`Wcs::unsupported_axes`], so a file using
//! one still reads, just with that axis not fully decoded.
//!
//! Binary-table WCS (Table 22) is supported for both the pixel-list
//! ([`Header::wcs_pixel_list`](crate::Header::wcs_pixel_list)) and vector-cell
//! ([`Header::wcs_array_column`](crate::Header::wcs_array_column)) forms.
//!
//! Pixel↔world yields celestial coordinates in the frame the file declares
//! (`RADESYS`/`EQUINOX`); converting *between* reference frames is astrometry
//! beyond the FITS standard and is intentionally out of scope.

use std::f64::consts::FRAC_PI_2;
use std::f64::consts::PI;
use std::f64::consts::SQRT_2;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

const R2D: f64 = 180.0 / PI;
const D2R: f64 = PI / 180.0;
const FRAC_PI_4: f64 = std::f64::consts::FRAC_PI_4;

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

impl Projection {
    fn from_code(code: &str) -> Option<Projection> {
        Some(match code {
            "TAN" => Projection::Tan,
            "SIN" => Projection::Sin,
            "ARC" => Projection::Arc,
            "STG" => Projection::Stg,
            "ZEA" => Projection::Zea,
            "CAR" => Projection::Car,
            "CEA" => Projection::Cea,
            "MER" => Projection::Mer,
            "SFL" => Projection::Sfl,
            "AIT" => Projection::Ait,
            "MOL" => Projection::Mol,
            "ZPN" => Projection::Zpn,
            "CYP" => Projection::Cyp,
            "PAR" => Projection::Par,
            "COP" => Projection::Cop,
            "COE" => Projection::Coe,
            "COD" => Projection::Cod,
            "COO" => Projection::Coo,
            "BON" => Projection::Bon,
            "AIR" => Projection::Air,
            "AZP" => Projection::Azp,
            "PCO" => Projection::Pco,
            "SZP" => Projection::Szp,
            _ => return None,
        })
    }

    /// Whether this is a zenithal projection (fiducial point at the native pole,
    /// `θ₀ = 90°`); cylindrical projections have `θ₀ = 0°`.
    fn is_zenithal(self) -> bool {
        matches!(
            self,
            Projection::Tan
                | Projection::Sin
                | Projection::Arc
                | Projection::Stg
                | Projection::Zea
                | Projection::Zpn
                | Projection::Air
        )
    }

    /// Whether this is a conic projection (`θ₀ = θ_a = PVi_1`).
    fn is_conic(self) -> bool {
        matches!(
            self,
            Projection::Cop | Projection::Coe | Projection::Cod | Projection::Coo
        )
    }

    /// The fiducial point `(φ₀, θ₀)` in degrees. Zenithal (incl. the perspective
    /// `AZP`): `(0, 90)`; conics: `(0, θ_a)` where `θ_a = PVi_1`; else `(0, 0)`.
    fn reference_point(self, pv: &[f64]) -> (f64, f64) {
        if self.is_zenithal() || matches!(self, Projection::Azp | Projection::Szp) {
            (0.0, 90.0)
        } else if self.is_conic() {
            (0.0, pv.get(1).copied().unwrap_or(0.0))
        } else {
            (0.0, 0.0)
        }
    }

    /// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
    /// `pv` holds the latitude axis's `PVi_0…` projection parameters.
    fn deproject(self, x: f64, y: f64, pv: &[f64]) -> (f64, f64) {
        if matches!(self, Projection::Azp) {
            // Tilted zenithal perspective (CG 2002 §5.1.1): undo the γ shear, then
            // solve A·sinθ + B·cosθ = C for θ.
            let (mu, gr) = (pv[1], pv[2] * D2R);
            let yc = y * gr.cos();
            let r = x.hypot(yc) / R2D;
            let phi = x.atan2(-yc);
            let (a, b, c) = (r, r * phi.cos() * gr.tan() - (mu + 1.0), -r * mu);
            let rad = a.hypot(b);
            let psi = b.atan2(a);
            let base = (c / rad).clamp(-1.0, 1.0).asin();
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
            return (phi * R2D, theta * R2D);
        }
        if matches!(self, Projection::Szp) {
            // Slant zenithal perspective (CG 2002 §5.1.2). With the vertex
            // P = (xp, yp, zp), substitute σ = 1 − sinθ and reduce to a quadratic
            // `zp²(2σ − σ²) = A² + B²` with A, B linear in σ.
            let (xp, yp, zp) = szp_vertex(pv);
            let (cx, cy) = (x / R2D, y / R2D);
            // A = a0 + a1·σ, B = b0 + b1·σ.
            let (a0, a1) = (cx * zp, -(cx - xp));
            let (b0, b1) = (-cy * zp, cy - yp);
            let qa = a1 * a1 + b1 * b1 + zp * zp;
            let qb = 2.0 * (a0 * a1 + b0 * b1) - 2.0 * zp * zp;
            let qc = a0 * a0 + b0 * b0;
            let disc = (qb * qb - 4.0 * qa * qc).max(0.0).sqrt();
            let s1 = (-qb - disc) / (2.0 * qa);
            let s2 = (-qb + disc) / (2.0 * qa);
            // σ ∈ [0, 2]; prefer the visible-hemisphere root (smaller σ).
            let sigma = if (0.0..=2.0).contains(&s1) { s1 } else { s2 };
            let theta = (1.0 - sigma).clamp(-1.0, 1.0).asin();
            let (a, b) = (a0 + a1 * sigma, b0 + b1 * sigma);
            let phi = a.atan2(b);
            return (phi * R2D, theta * R2D);
        }
        if self.is_conic() {
            let (c, y0) = self.conic_consts(pv);
            let s = pv[1].signum();
            let r = s * x.hypot(y0 - y);
            let phi = (s * x).atan2(s * (y0 - y)) * R2D / c;
            return (phi, self.conic_theta(r, pv, c));
        }
        if self.is_zenithal() {
            let r = x.hypot(y);
            let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
            // Colatitude ζ (rad) from the radius, per projection.
            let u = r / R2D;
            let zeta = match self {
                Projection::Tan => u.atan(),
                Projection::Sin => u.clamp(-1.0, 1.0).asin(),
                Projection::Arc => u,
                Projection::Zea => 2.0 * (u / 2.0).clamp(-1.0, 1.0).asin(),
                Projection::Stg => 2.0 * (u / 2.0).atan(),
                // ZPN: solve Σ Pₘ ζᵐ = u for ζ (Newton from ζ = u).
                Projection::Zpn => zpn_zeta(u, pv),
                // AIR: solve the transcendental radius for ζ (Newton).
                Projection::Air => air_zeta(u, pv[1]),
                _ => unreachable!(),
            };
            (phi, 90.0 - zeta * R2D)
        } else {
            match self {
                Projection::Car => (x, y),
                // CEA: λ = PVi_1 (default 1); θ = asin(λ·y/(180/π)).
                Projection::Cea => {
                    let lambda = pv.get(1).filter(|&&v| v != 0.0).copied().unwrap_or(1.0);
                    (x, (lambda * y / R2D).clamp(-1.0, 1.0).asin() * R2D)
                }
                Projection::Mer => (x, (2.0 * (y / R2D).exp().atan()) * R2D - 90.0),
                Projection::Sfl => (x / (y * D2R).cos(), y),
                // Hammer–Aitoff inverse (CG 2002 eq. 51).
                Projection::Ait => {
                    let (u, v) = (x * D2R, y * D2R);
                    let z2 = (1.0 - (u / 4.0).powi(2) - (v / 2.0).powi(2)).max(0.0);
                    let z = z2.sqrt();
                    let phi = 2.0 * (z * u / 2.0).atan2(2.0 * z2 - 1.0) * R2D;
                    let theta = (v * z).clamp(-1.0, 1.0).asin() * R2D;
                    (phi, theta)
                }
                // Mollweide inverse (CG 2002 eq. 55).
                Projection::Mol => {
                    let s2 = SQRT_2;
                    let gamma = (y / (s2 * R2D)).clamp(-1.0, 1.0).asin();
                    let theta = ((2.0 * gamma + (2.0 * gamma).sin()) / PI).asin() * R2D;
                    let phi = PI * x / (2.0 * s2 * gamma.cos());
                    (phi, theta)
                }
                // CYP inverse: φ = x/λ; θ from η = (y/(180/π))/(μ+λ).
                Projection::Cyp => {
                    let (mu, lambda) = (
                        pv[1],
                        pv.get(2).filter(|&&v| v != 0.0).copied().unwrap_or(1.0),
                    );
                    let eta = (y / R2D) / (mu + lambda);
                    let theta = eta.atan2(1.0)
                        + (eta * mu / (1.0 + eta * eta).sqrt())
                            .clamp(-1.0, 1.0)
                            .asin();
                    (x / lambda, theta * R2D)
                }
                // PAR inverse (CG 2002 eq. 49).
                Projection::Par => {
                    let theta = 3.0 * (y / 180.0).clamp(-1.0, 1.0).asin();
                    (x / (2.0 * (2.0 * theta / 3.0).cos() - 1.0), theta * R2D)
                }
                // Polyconic inverse (CG 2002 §5.6.1): Newton on
                // f(θ) = X² + (Y−θ)² − 2(Y−θ)cotθ = 0, then recover φ.
                Projection::Pco => {
                    let (xr, yr) = (x * D2R, y * D2R);
                    if yr.abs() < 1e-12 {
                        return (x, 0.0);
                    }
                    let mut th = yr;
                    for _ in 0..100 {
                        let d = yr - th;
                        let cot = 1.0 / th.tan();
                        let f = xr * xr + d * d - 2.0 * d * cot;
                        let fp = -2.0 * d + 2.0 * cot + 2.0 * d / th.sin().powi(2);
                        let step = f / fp;
                        th -= step;
                        if step.abs() < 1e-13 {
                            break;
                        }
                    }
                    let d = yr - th;
                    let tanth = th.tan();
                    let omega = (xr * tanth).atan2(1.0 - d * tanth);
                    (omega / th.sin() * R2D, th * R2D)
                }
                // Bonne's pseudoconic inverse (CG 2002 §5.5.1), θ₁ = PVi_1.
                Projection::Bon => {
                    // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0
                    // (avoiding the `1/tan 0` singularity below).
                    if pv[1] == 0.0 {
                        return (x / (y * D2R).cos(), y);
                    }
                    let t1 = pv[1] * D2R;
                    let y0 = t1 + 1.0 / t1.tan();
                    let s = pv[1].signum();
                    let yc = y0 - y * D2R;
                    let r = s * (x * D2R).hypot(yc);
                    let tr = y0 - r;
                    let aphi = (s * x * D2R).atan2(s * yc);
                    (aphi * r / tr.cos() * R2D, tr * R2D)
                }
                _ => unreachable!(),
            }
        }
    }

    /// Project native `(φ, θ)` (deg) to intermediate world `(x, y)` (deg).
    fn project(self, phi: f64, theta: f64, pv: &[f64]) -> (f64, f64) {
        if matches!(self, Projection::Azp) {
            let (mu, gr) = (pv[1], pv[2] * D2R);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let denom = (mu + tr.sin()) + tr.cos() * pr.cos() * gr.tan();
            let r = R2D * (mu + 1.0) * tr.cos() / denom;
            return (r * pr.sin(), -r * pr.cos() / gr.cos());
        }
        if matches!(self, Projection::Szp) {
            let (xp, yp, zp) = szp_vertex(pv);
            let (tr, pr) = (theta * D2R, phi * D2R);
            let sigma = 1.0 - tr.sin();
            let denom = zp - sigma;
            let x = R2D * (zp * tr.cos() * pr.sin() - xp * sigma) / denom;
            let y = R2D * (-zp * tr.cos() * pr.cos() - yp * sigma) / denom;
            return (x, y);
        }
        if self.is_conic() {
            let (c, y0) = self.conic_consts(pv);
            let r = self.conic_radius(theta, pv);
            let cp = (c * phi) * D2R;
            return (r * cp.sin(), y0 - r * cp.cos());
        }
        if self.is_zenithal() {
            let zeta = (90.0 - theta) * D2R;
            let r = match self {
                Projection::Tan => R2D * zeta.tan(),
                Projection::Sin => R2D * zeta.sin(),
                Projection::Arc => R2D * zeta,
                Projection::Zea => 2.0 * R2D * (zeta / 2.0).sin(),
                Projection::Stg => 2.0 * R2D * (zeta / 2.0).tan(),
                Projection::Zpn => R2D * zpn_poly(zeta, pv),
                Projection::Air => R2D * air_radius_u(zeta, pv[1]),
                _ => unreachable!(),
            };
            let p = phi * D2R;
            (r * p.sin(), -r * p.cos())
        } else {
            let t = theta * D2R;
            match self {
                Projection::Car => (phi, theta),
                Projection::Cea => {
                    let lambda = pv.get(1).filter(|&&v| v != 0.0).copied().unwrap_or(1.0);
                    (phi, R2D * t.sin() / lambda)
                }
                Projection::Mer => (phi, R2D * ((45.0 + theta / 2.0) * D2R).tan().ln()),
                Projection::Sfl => (phi * t.cos(), theta),
                Projection::Ait => {
                    let pr = phi * D2R;
                    let gamma = R2D * (2.0 / (1.0 + t.cos() * (pr / 2.0).cos())).sqrt();
                    (2.0 * gamma * t.cos() * (pr / 2.0).sin(), gamma * t.sin())
                }
                Projection::Mol => {
                    // Solve 2γ + sin2γ = π·sinθ for γ (Newton).
                    let s2 = SQRT_2;
                    let target = PI * t.sin();
                    let mut g = t; // initial guess
                    for _ in 0..100 {
                        let f = 2.0 * g + (2.0 * g).sin() - target;
                        let d = 2.0 + 2.0 * (2.0 * g).cos();
                        let step = f / d;
                        g -= step;
                        if step.abs() < 1e-14 {
                            break;
                        }
                    }
                    ((2.0 * s2 / PI) * phi * g.cos(), s2 * R2D * g.sin())
                }
                Projection::Cyp => {
                    let (mu, lambda) = (
                        pv[1],
                        pv.get(2).filter(|&&v| v != 0.0).copied().unwrap_or(1.0),
                    );
                    (lambda * phi, R2D * (mu + lambda) * t.sin() / (mu + t.cos()))
                }
                Projection::Par => (
                    phi * (2.0 * (2.0 * t / 3.0).cos() - 1.0),
                    180.0 * (t / 3.0).sin(),
                ),
                Projection::Bon => {
                    // §5.5.1: BON degenerates to the sinusoidal SFL at θ₁ = 0.
                    if pv[1] == 0.0 {
                        return (phi * t.cos(), theta);
                    }
                    let t1 = pv[1] * D2R;
                    let y0 = t1 + 1.0 / t1.tan();
                    let r = y0 - t;
                    let aphi = phi * D2R * t.cos() / r;
                    (R2D * r * aphi.sin(), R2D * (y0 - r * aphi.cos()))
                }
                Projection::Pco => {
                    if theta.abs() < 1e-12 {
                        return (phi, 0.0);
                    }
                    let omega = phi * D2R * t.sin();
                    let cot = 1.0 / t.tan();
                    (
                        R2D * cot * omega.sin(),
                        theta + R2D * cot * (1.0 - omega.cos()),
                    )
                }
                _ => unreachable!(),
            }
        }
    }

    /// Conic constants `(C, Y0)` (CG 2002 §3.4): `θ_a = PVi_1`, `η = PVi_2` (deg).
    fn conic_consts(self, pv: &[f64]) -> (f64, f64) {
        let (ta, eta) = (pv[1] * D2R, pv[2] * D2R);
        let (t1, t2) = (ta - eta, ta + eta);
        match self {
            Projection::Cop => {
                let c = ta.sin();
                (c, R2D * eta.cos() / ta.tan())
            }
            Projection::Coe => {
                let (s1, s2) = (t1.sin(), t2.sin());
                let c = (s1 + s2) / 2.0;
                let y0 = R2D / c * (1.0 + s1 * s2 - 2.0 * c * ta.sin()).max(0.0).sqrt();
                (c, y0)
            }
            Projection::Cod => {
                // Equidistant: C = sinθ_a·sinη/η; R = Y0 + (θ_a − θ) deg, with
                // Y0 = (180/π)·(η/tanη)·cotθ_a (η→0 ⇒ η/tanη→1).
                let (c, k) = if eta.abs() < 1e-12 {
                    (ta.sin(), 1.0)
                } else {
                    (ta.sin() * eta.sin() / eta, eta / eta.tan())
                };
                (c, R2D * k / ta.tan())
            }
            Projection::Coo => {
                let c = if eta.abs() < 1e-12 {
                    ta.sin()
                } else {
                    (t2.cos() / t1.cos()).ln()
                        / ((FRAC_PI_4 - t2 / 2.0).tan() / (FRAC_PI_4 - t1 / 2.0).tan()).ln()
                };
                let psi = R2D * t1.cos() / (c * (FRAC_PI_4 - t1 / 2.0).tan().powf(c));
                (c, psi * (FRAC_PI_4 - ta / 2.0).tan().powf(c))
            }
            _ => unreachable!(),
        }
    }

    /// Conic radius `R_θ` (deg) for a native latitude `θ` (deg).
    fn conic_radius(self, theta: f64, pv: &[f64]) -> f64 {
        let (c, y0) = self.conic_consts(pv);
        let (ta, eta, t) = (pv[1] * D2R, pv[2] * D2R, theta * D2R);
        let (t1, t2) = (ta - eta, ta + eta);
        match self {
            Projection::Cop => R2D * eta.cos() * (1.0 / ta.tan() - (t - ta).tan()),
            Projection::Coe => {
                let (s1, s2) = (t1.sin(), t2.sin());
                R2D / c * (1.0 + s1 * s2 - 2.0 * c * t.sin()).max(0.0).sqrt()
            }
            Projection::Cod => y0 + (pv[1] - theta),
            Projection::Coo => {
                let psi = R2D * t1.cos() / (c * (FRAC_PI_4 - t1 / 2.0).tan().powf(c));
                psi * (FRAC_PI_4 - t / 2.0).tan().powf(c)
            }
            _ => unreachable!(),
        }
    }

    /// Native latitude `θ` (deg) for a conic radius `R_θ` (deg).
    fn conic_theta(self, r: f64, pv: &[f64], c: f64) -> f64 {
        let (ta, eta) = (pv[1] * D2R, pv[2] * D2R);
        let (t1, t2) = (ta - eta, ta + eta);
        match self {
            Projection::Cop => {
                let tan = 1.0 / ta.tan() - r / (R2D * eta.cos());
                pv[1] + tan.atan() * R2D
            }
            Projection::Coe => {
                let (s1, s2) = (t1.sin(), t2.sin());
                let sin_t = (1.0 + s1 * s2 - (r * c / R2D).powi(2)) / (2.0 * c);
                sin_t.clamp(-1.0, 1.0).asin() * R2D
            }
            Projection::Cod => {
                let y0 = self.conic_consts(pv).1;
                pv[1] - (r - y0)
            }
            Projection::Coo => {
                let psi = R2D * t1.cos() / (c * (FRAC_PI_4 - t1 / 2.0).tan().powf(c));
                90.0 - 2.0 * (r / psi).powf(1.0 / c).atan() * R2D
            }
            _ => unreachable!(),
        }
    }
}

/// SZP projection vertex `(x_p, y_p, z_p)` from `μ = PVi_1`, `φc = PVi_2`,
/// `θc = PVi_3` (CG 2002 §5.1.2).
fn szp_vertex(pv: &[f64]) -> (f64, f64, f64) {
    let mu = pv[1];
    let (phic, thetac) = (pv[2] * D2R, pv[3] * D2R);
    (
        -mu * thetac.cos() * phic.sin(),
        mu * thetac.cos() * phic.cos(),
        mu * thetac.sin() + 1.0,
    )
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

/// Invert the AIR radius for ζ given `u = R/(180/π)` (Newton).
fn air_zeta(u: f64, theta_b: f64) -> f64 {
    let mut z = u.max(1e-6); // start near ζ ≈ u
    for _ in 0..100 {
        let f = air_radius_u(z, theta_b) - u;
        let d = (air_radius_u(z + 1e-7, theta_b) - air_radius_u(z - 1e-7, theta_b)) / 2e-7;
        if d == 0.0 {
            break;
        }
        let step = f / d;
        z -= step;
        if step.abs() < 1e-13 {
            break;
        }
    }
    z
}

/// ZPN forward polynomial `R/(180/π) = Σ Pₘ ζᵐ` (ζ in radians, `pv[m] = PVi_m`).
fn zpn_poly(zeta: f64, pv: &[f64]) -> f64 {
    pv.iter().rev().fold(0.0, |acc, &p| acc * zeta + p)
}

/// Invert the ZPN polynomial for ζ given `u = R/(180/π)` (Newton from ζ = u).
fn zpn_zeta(u: f64, pv: &[f64]) -> f64 {
    let mut z = u;
    for _ in 0..100 {
        let f = zpn_poly(z, pv) - u;
        // derivative Σ m·Pₘ ζ^(m-1)
        let d: f64 = pv
            .iter()
            .enumerate()
            .skip(1)
            .map(|(m, &p)| m as f64 * p * z.powi(m as i32 - 1))
            .sum();
        if d == 0.0 {
            break;
        }
        let step = f / d;
        z -= step;
        if step.abs() < 1e-14 {
            break;
        }
    }
    z
}

/// A parsed world coordinate system for one (optionally alternate) axis set.
#[derive(Debug, Clone)]
pub struct Wcs {
    /// Number of WCS axes.
    pub naxis: usize,
    /// `CTYPEi` strings.
    pub ctype: Vec<String>,
    /// `CRVALi` — world coordinate at the reference pixel.
    pub crval: Vec<f64>,
    /// `CRPIXi` — reference pixel (1-based).
    pub crpix: Vec<f64>,
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
    pub unsupported_axes: Vec<usize>,
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
    proj: Projection,
    /// The native→celestial pole, computed from the fiducial point.
    pole: CelestialPole,
    /// Latitude-axis `PVi_0…` projection parameters.
    pv: Vec<f64>,
}

impl Wcs {
    /// Parse the primary WCS (`alt = None`) or an alternate description
    /// (`alt = Some('A'..='Z')`) from `header`. The public entry point is
    /// [`Header::wcs`](crate::Header::wcs), which forwards here.
    pub(crate) fn from_header(header: &Header, alt: Option<char>) -> Result<Wcs> {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        let naxis = header
            .get_integer(key!("WCSAXES{a}").as_str())
            .or_else(|| header.get_integer("NAXIS"))
            .ok_or(FitsError::MissingKeyword { name: "WCSAXES" })?
            .max(0) as usize;
        if naxis == 0 {
            return Err(FitsError::InvalidValue {
                card: "WCSAXES = 0".to_string(),
            });
        }

        let ctype: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CTYPE{i}{a}").as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let mut crval = axis_vec(header, "CRVAL", &a, naxis, 0.0);
        let crpix = axis_vec(header, "CRPIX", &a, naxis, 0.0);
        let cdelt = axis_vec(header, "CDELT", &a, naxis, 1.0);
        let cunit: Vec<String> = (1..=naxis)
            .map(|i| {
                header
                    .get_text(key!("CUNIT{i}{a}").as_str())
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
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
            .any(|i| (1..=naxis).any(|j| header.get_real(key!("CD{i}_{j}{a}").as_str()).is_some()));
        let has_pc = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get_real(key!("PC{i}_{j}{a}").as_str()).is_some()));
        let has_crota =
            (1..=naxis).any(|i| header.get_real(key!("CROTA{i}{a}").as_str()).is_some());
        // §8: the PC/CDELT, CD, and legacy CROTA conventions are mutually exclusive.
        if has_cd && has_pc {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "PC and CD both present",
            });
        }
        if has_pc && has_crota {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "CROTA and PC both present",
            });
        }
        let mut matrix = vec![0.0; naxis * naxis];
        if has_cd {
            for i in 0..naxis {
                for j in 0..naxis {
                    matrix[i * naxis + j] = header
                        .get_real(key!("CD{}_{}{a}", i + 1, j + 1).as_str())
                        .unwrap_or(0.0);
                }
            }
        } else {
            for i in 0..naxis {
                for j in 0..naxis {
                    let pc = header
                        .get_real(key!("PC{}_{}{a}", i + 1, j + 1).as_str())
                        .unwrap_or(if i == j { 1.0 } else { 0.0 });
                    matrix[i * naxis + j] = cdelt[i] * pc;
                }
            }
            // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC
            // was given, per the convention that CROTA and PC are exclusive).
            if !has_pc && let Some((lng, lat, _)) = celestial_axes {
                let rho = header
                    .get_real(key!("CROTA{}{a}", lat + 1).as_str())
                    .or_else(|| header.get_real(key!("CROTA{}{a}", lng + 1).as_str()))
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
                let pv: Vec<f64> = (0..=20)
                    .map(|m| {
                        header
                            .get_real(key!("PV{}_{m}{a}", lat + 1).as_str())
                            .unwrap_or(0.0)
                    })
                    .collect();
                // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be
                // non-zero; θ_a = 0 (absent, or explicitly 0) is a degenerate cone
                // (`1/tan 0`). Treat it like an unimplemented projection — flag the
                // axes and skip deprojection so they pass through the linear stage
                // (an intermediate world coordinate) rather than returning NaN.
                if proj.is_conic() && pv[1] == 0.0 {
                    unsupported_axes.push(lng);
                    unsupported_axes.push(lat);
                    unsupported_axes.sort_unstable();
                    None
                } else {
                    // Fiducial point: projection default, overridable by PVi_1a/
                    // PVi_2a on the longitude axis (§8.3).
                    let (mut phi0, mut theta0) = proj.reference_point(&pv);
                    if let Some(v) = header.get_real(key!("PV{}_1{a}", lng + 1).as_str()) {
                        phi0 = v;
                    }
                    if let Some(v) = header.get_real(key!("PV{}_2{a}", lng + 1).as_str()) {
                        theta0 = v;
                    }
                    let (alpha0, delta0) = (crval[lng], crval[lat]);
                    // LONPOLE (= LONPOLEa or PVi_3a): default φ0 if δ0 ≥ θ0, else φ0 + 180°.
                    let phip = header
                        .get_real(key!("LONPOLE{a}").as_str())
                        .or_else(|| header.get_real(key!("PV{}_3{a}", lng + 1).as_str()))
                        .unwrap_or(if delta0 >= theta0 { phi0 } else { phi0 + 180.0 });
                    // LATPOLE (= LATPOLEa or PVi_4a): default 90°.
                    let thetap = header
                        .get_real(key!("LATPOLE{a}").as_str())
                        .or_else(|| header.get_real(key!("PV{}_4{a}", lng + 1).as_str()))
                        .unwrap_or(90.0);
                    let pole = compute_pole(phi0, theta0, alpha0, delta0, phip, thetap);
                    Some(Celestial {
                        lng,
                        lat,
                        proj,
                        pole,
                        pv,
                    })
                }
            }
            None => None,
        };

        Ok(Wcs {
            naxis,
            ctype,
            crval,
            crpix,
            matrix,
            inverse,
            celestial,
            unsupported_axes,
        })
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
            if let Some(t) = header.get_text(key!("TCTYP{c}{a}").as_str()) {
                h.set(key!("CTYPE{ax}").as_str(), t);
            }
            for (root, dst) in [
                ("TCRPX", "CRPIX"),
                ("TCRVL", "CRVAL"),
                ("TCDLT", "CDELT"),
                ("TCROT", "CROTA"),
            ] {
                if let Some(v) = header.get_real(key!("{root}{c}{a}").as_str()) {
                    h.set(key!("{dst}{ax}").as_str(), v);
                }
            }
            if let Some(t) = header.get_text(key!("TCUNI{c}{a}").as_str()) {
                h.set(key!("CUNIT{ax}").as_str(), t);
            }
            for m in 0..=20 {
                if let Some(v) = header.get_real(key!("TPV{c}_{m}{a}").as_str()) {
                    h.set(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear-transform matrices: TPCn_ka / TCDn_ka, indexed by column pair.
        for (i, &ci) in columns.iter().enumerate() {
            for (j, &cj) in columns.iter().enumerate() {
                if let Some(v) = header.get_real(key!("TPC{ci}_{cj}{a}").as_str()) {
                    h.set(key!("PC{}_{}", i + 1, j + 1).as_str(), v);
                }
                if let Some(v) = header.get_real(key!("TCD{ci}_{cj}{a}").as_str()) {
                    h.set(key!("CD{}_{}", i + 1, j + 1).as_str(), v);
                }
            }
        }
        if let Some(v) = header.get_real(key!("LONP{a}").as_str()) {
            h.set("LONPOLE", v);
        }
        if let Some(v) = header.get_real(key!("LATP{a}").as_str()) {
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
        let naxis = header
            .get_integer(key!("WCAX{column}{a}").as_str())
            .map(|v| v.max(0) as usize)
            .filter(|&n| n > 0)
            .unwrap_or_else(|| {
                (1..=99)
                    .rev()
                    .find(|&i| {
                        header
                            .get_text(key!("{i}CTYP{column}{a}").as_str())
                            .is_some()
                            || ["CRVL", "CDLT", "CRPX"].iter().any(|r| {
                                header
                                    .get_real(key!("{i}{r}{column}{a}").as_str())
                                    .is_some()
                            })
                    })
                    .unwrap_or(0)
            });
        if naxis == 0 {
            return Err(FitsError::MissingKeyword { name: "iCTYPn" });
        }
        let mut h = Header::new();
        h.set("WCSAXES", naxis as i64);
        for ax in 1..=naxis {
            if let Some(t) = header.get_text(key!("{ax}CTYP{column}{a}").as_str()) {
                h.set(key!("CTYPE{ax}").as_str(), t);
            }
            if let Some(t) = header.get_text(key!("{ax}CUNI{column}{a}").as_str()) {
                h.set(key!("CUNIT{ax}").as_str(), t);
            }
            for (root, dst) in [
                ("CRPX", "CRPIX"),
                ("CRVL", "CRVAL"),
                ("CDLT", "CDELT"),
                ("CROT", "CROTA"),
            ] {
                if let Some(v) = header.get_real(key!("{ax}{root}{column}{a}").as_str()) {
                    h.set(key!("{dst}{ax}").as_str(), v);
                }
            }
            // PVi_m arrives as `iPVn_ma`, or the abbreviated `iVn_ma`.
            for m in 0..=20 {
                if let Some(v) = header
                    .get_real(key!("{ax}PV{column}_{m}{a}").as_str())
                    .or_else(|| header.get_real(key!("{ax}V{column}_{m}{a}").as_str()))
                {
                    h.set(key!("PV{ax}_{m}").as_str(), v);
                }
            }
        }
        // Linear-transform matrices: `ijPCn` / `ijCDn`, indexed by axis pair.
        for i in 1..=naxis {
            for j in 1..=naxis {
                if let Some(v) = header.get_real(key!("{i}{j}PC{column}{a}").as_str()) {
                    h.set(key!("PC{i}_{j}").as_str(), v);
                }
                if let Some(v) = header.get_real(key!("{i}{j}CD{column}{a}").as_str()) {
                    h.set(key!("CD{i}_{j}").as_str(), v);
                }
            }
        }
        Wcs::from_header(&h, None)
    }

    /// Map 1-based pixel coordinates to world coordinates. Celestial axes return
    /// `(α, δ)` in degrees; other axes return `CRVAL + ` the linear value.
    pub fn pixel_to_world(&self, pixel: &[f64]) -> Vec<f64> {
        assert_eq!(pixel.len(), self.naxis, "pixel coordinate count");
        // Offset, then apply the linear transform → intermediate world coords.
        let offset: Vec<f64> = (0..self.naxis).map(|i| pixel[i] - self.crpix[i]).collect();
        let inter = matvec(&self.matrix, &offset, self.naxis);

        let mut world = vec![0.0; self.naxis];
        for i in 0..self.naxis {
            world[i] = self.crval[i] + inter[i];
        }
        if let Some(c) = &self.celestial {
            let (phi, theta) = c.proj.deproject(inter[c.lng], inter[c.lat], &c.pv);
            let (ra, dec) = native_to_celestial(c.pole, phi, theta);
            world[c.lng] = ra;
            world[c.lat] = dec;
        }
        world
    }

    /// Map world coordinates back to 1-based pixel coordinates (the inverse of
    /// [`Wcs::pixel_to_world`]).
    pub fn world_to_pixel(&self, world: &[f64]) -> Vec<f64> {
        assert_eq!(world.len(), self.naxis, "world coordinate count");
        // Recover the intermediate world coordinates.
        let mut inter = vec![0.0; self.naxis];
        for i in 0..self.naxis {
            inter[i] = world[i] - self.crval[i];
        }
        if let Some(c) = &self.celestial {
            let (phi, theta) = celestial_to_native(c.pole, world[c.lng], world[c.lat]);
            let (x, y) = c.proj.project(phi, theta, &c.pv);
            inter[c.lng] = x;
            inter[c.lat] = y;
        }
        // Invert the linear transform, then add back CRPIX.
        let offset = matvec(&self.inverse, &inter, self.naxis);
        (0..self.naxis).map(|i| offset[i] + self.crpix[i]).collect()
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
fn native_to_celestial(pole: CelestialPole, phi: f64, theta: f64) -> (f64, f64) {
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
    (norm360(ap + y.atan2(x) * R2D), dec)
}

/// Celestial (α, δ) → native spherical (φ, θ), all degrees (CG 2002 eq. 5).
fn celestial_to_native(pole: CelestialPole, ra: f64, dec: f64) -> (f64, f64) {
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
    (norm180(fp + y.atan2(x) * R2D), theta)
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
fn axis_vec(header: &Header, prefix: &str, alt: &str, naxis: usize, default: f64) -> Vec<f64> {
    (1..=naxis)
        .map(|i| {
            header
                .get_real(key!("{prefix}{i}{alt}").as_str())
                .unwrap_or(default)
        })
        .collect()
}

/// Multiply the row-major `n×n` matrix `m` by vector `v`.
fn matvec(m: &[f64], v: &[f64], n: usize) -> Vec<f64> {
    (0..n)
        .map(|i| (0..n).map(|j| m[i * n + j] * v[j]).sum())
        .collect()
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
