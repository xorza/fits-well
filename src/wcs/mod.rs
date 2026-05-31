//! Typed World Coordinate System (§8) — behind the `wcs` feature.
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
//! v1 covers the linear layer (`PC`+`CDELT` or `CD`, with general matrix
//! inversion for the reverse direction) and the zenithal celestial projections
//! `TAN`/`SIN`/`ARC`; non-celestial axes pass through linearly. Validated against
//! `astropy.wcs` (wcslib).

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

const R2D: f64 = 180.0 / std::f64::consts::PI;
const D2R: f64 = std::f64::consts::PI / 180.0;

/// A celestial (zenithal) projection algorithm — the 3-letter `CTYPE` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `TAN` — gnomonic.
    Tan,
    /// `SIN` — orthographic/slant.
    Sin,
    /// `ARC` — zenithal equidistant.
    Arc,
}

impl Projection {
    fn from_code(code: &str) -> Option<Projection> {
        match code {
            "TAN" => Some(Projection::Tan),
            "SIN" => Some(Projection::Sin),
            "ARC" => Some(Projection::Arc),
            _ => None,
        }
    }

    /// Projection-plane radius `r` (deg) from native latitude `θ` (deg).
    fn r_of_theta(self, theta_deg: f64) -> f64 {
        let t = theta_deg * D2R;
        match self {
            Projection::Tan => R2D / t.tan(),
            Projection::Sin => R2D * t.cos(),
            Projection::Arc => 90.0 - theta_deg,
        }
    }
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
    /// `LONPOLE` (native longitude of the celestial pole), degrees.
    lonpole: f64,
    /// The (longitude axis, latitude axis, projection) when a celestial pair is
    /// present; `None` for an all-linear system.
    celestial: Option<Celestial>,
}

#[derive(Debug, Clone, Copy)]
struct Celestial {
    lng: usize,
    lat: usize,
    proj: Projection,
}

impl Wcs {
    /// Parse the primary WCS (`alt = None`) or an alternate description
    /// (`alt = Some('A'..='Z')`) from `header`.
    pub fn from_header(header: &Header, alt: Option<char>) -> Result<Wcs> {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        let naxis = header
            .get_integer(&format!("WCSAXES{a}"))
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
                    .get_text(&format!("CTYPE{i}{a}"))
                    .unwrap_or("")
                    .to_string()
            })
            .collect();
        let crval = axis_vec(header, "CRVAL", &a, naxis, 0.0);
        let crpix = axis_vec(header, "CRPIX", &a, naxis, 0.0);
        let cdelt = axis_vec(header, "CDELT", &a, naxis, 1.0);

        // Build the linear transform A. CD takes precedence over PC; CDELT folds
        // into A only in the PC form (CD already includes the scale).
        let has_cd = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get_real(&format!("CD{i}_{j}{a}")).is_some()));
        let mut matrix = vec![0.0; naxis * naxis];
        for i in 0..naxis {
            for j in 0..naxis {
                let (idx, jdx) = (i + 1, j + 1);
                if has_cd {
                    matrix[i * naxis + j] =
                        header.get_real(&format!("CD{idx}_{jdx}{a}")).unwrap_or(0.0);
                } else {
                    let pc = header
                        .get_real(&format!("PC{idx}_{jdx}{a}"))
                        .unwrap_or(if i == j { 1.0 } else { 0.0 });
                    matrix[i * naxis + j] = cdelt[i] * pc;
                }
            }
        }
        let inverse = invert(&matrix, naxis).ok_or(FitsError::InvalidValue {
            card: "singular WCS transform matrix".to_string(),
        })?;

        let celestial = find_celestial(&ctype);
        let default_lonpole =
            celestial.map_or(0.0, |c| if crval[c.lat] < 90.0 { 180.0 } else { 0.0 });
        let lonpole = header
            .get_real(&format!("LONPOLE{a}"))
            .unwrap_or(default_lonpole);

        Ok(Wcs {
            naxis,
            ctype,
            crval,
            crpix,
            matrix,
            inverse,
            lonpole,
            celestial,
        })
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
        if let Some(c) = self.celestial {
            let (phi, theta) = deproject(c.proj, inter[c.lng], inter[c.lat]);
            let (ra, dec) = self.native_to_celestial(phi, theta);
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
        if let Some(c) = self.celestial {
            let (phi, theta) = self.celestial_to_native(world[c.lng], world[c.lat]);
            let (x, y) = project(c.proj, phi, theta);
            inter[c.lng] = x;
            inter[c.lat] = y;
        }
        // Invert the linear transform, then add back CRPIX.
        let offset = matvec(&self.inverse, &inter, self.naxis);
        (0..self.naxis).map(|i| offset[i] + self.crpix[i]).collect()
    }

    /// Native spherical (φ, θ) → celestial (α, δ), all degrees (CG 2002 eq. 2).
    fn native_to_celestial(&self, phi: f64, theta: f64) -> (f64, f64) {
        let c = self.celestial.expect("celestial system");
        let (ap, dp, fp) = (self.crval[c.lng], self.crval[c.lat], self.lonpole);
        let (tr, dpr, dphi) = (theta * D2R, dp * D2R, (phi - fp) * D2R);
        let sin_d = tr.sin() * dpr.sin() + tr.cos() * dpr.cos() * dphi.cos();
        let dec = sin_d.clamp(-1.0, 1.0).asin() * R2D;
        let y = -tr.cos() * dphi.sin();
        let x = tr.sin() * dpr.cos() - tr.cos() * dpr.sin() * dphi.cos();
        let ra = ap + y.atan2(x) * R2D;
        (norm360(ra), dec)
    }

    /// Celestial (α, δ) → native spherical (φ, θ), all degrees (CG 2002 eq. 5).
    fn celestial_to_native(&self, ra: f64, dec: f64) -> (f64, f64) {
        let c = self.celestial.expect("celestial system");
        let (ap, dp, fp) = (self.crval[c.lng], self.crval[c.lat], self.lonpole);
        let (dr, dpr, dalpha) = (dec * D2R, dp * D2R, (ra - ap) * D2R);
        let sin_t = dr.sin() * dpr.sin() + dr.cos() * dpr.cos() * dalpha.cos();
        let theta = sin_t.clamp(-1.0, 1.0).asin() * R2D;
        let y = -dr.cos() * dalpha.sin();
        let x = dr.sin() * dpr.cos() - dr.cos() * dpr.sin() * dalpha.cos();
        let phi = fp + y.atan2(x) * R2D;
        (norm180(phi), theta)
    }
}

/// Identify the longitude/latitude axis pair and projection from `CTYPE`s.
fn find_celestial(ctype: &[String]) -> Option<Celestial> {
    let mut lng = None;
    let mut lat = None;
    let mut proj = None;
    for (i, t) in ctype.iter().enumerate() {
        let head = t.split('-').next().unwrap_or("");
        let is_lng = head == "RA" || head.ends_with("LON") || head == "LON";
        let is_lat = head == "DEC" || head.ends_with("LAT") || head == "LAT";
        if (is_lng || is_lat)
            && let Some(code) = t.rsplit('-').find(|s| !s.is_empty())
        {
            proj = proj.or_else(|| Projection::from_code(code));
        }
        if is_lng {
            lng = Some(i);
        } else if is_lat {
            lat = Some(i);
        }
    }
    match (lng, lat, proj) {
        (Some(lng), Some(lat), Some(proj)) => Some(Celestial { lng, lat, proj }),
        _ => None,
    }
}

/// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
fn deproject(proj: Projection, x: f64, y: f64) -> (f64, f64) {
    let r = x.hypot(y);
    let phi = if r == 0.0 { 0.0 } else { x.atan2(-y) * R2D };
    let theta = match proj {
        // R = (180/π)·cotθ ⇒ θ = atan2(180/π, R).
        Projection::Tan => R2D.atan2(r) * R2D,
        Projection::Sin => (r / R2D).clamp(-1.0, 1.0).acos() * R2D,
        Projection::Arc => 90.0 - r,
    };
    (phi, theta)
}

/// Project native `(φ, θ)` (deg) to intermediate world `(x, y)` (deg).
fn project(proj: Projection, phi: f64, theta: f64) -> (f64, f64) {
    let r = proj.r_of_theta(theta);
    let pr = phi * D2R;
    (r * pr.sin(), -r * pr.cos())
}

/// Read `PREFIX1..PREFIXn` (with alternate suffix) into a vector, defaulting
/// missing entries.
fn axis_vec(header: &Header, prefix: &str, alt: &str, naxis: usize, default: f64) -> Vec<f64> {
    (1..=naxis)
        .map(|i| {
            header
                .get_real(&format!("{prefix}{i}{alt}"))
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
