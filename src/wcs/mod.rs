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
            _ => return None,
        })
    }

    /// Whether this is a zenithal projection (fiducial point at the native pole,
    /// `θ₀ = 90°`); cylindrical projections have `θ₀ = 0°`.
    fn is_zenithal(self) -> bool {
        matches!(
            self,
            Projection::Tan | Projection::Sin | Projection::Arc | Projection::Stg | Projection::Zea
        )
    }

    /// The fiducial point `(φ₀, θ₀)` in degrees — `(0, 90)` zenithal, `(0, 0)` else.
    fn reference_point(self) -> (f64, f64) {
        if self.is_zenithal() { (0.0, 90.0) } else { (0.0, 0.0) }
    }

    /// Deproject intermediate world `(x, y)` (deg) to native `(φ, θ)` (deg).
    fn deproject(self, x: f64, y: f64) -> (f64, f64) {
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
                _ => unreachable!(),
            };
            (phi, 90.0 - zeta * R2D)
        } else {
            match self {
                Projection::Car => (x, y),
                Projection::Cea => (x, (y / R2D).clamp(-1.0, 1.0).asin() * R2D),
                Projection::Mer => (x, (2.0 * (y / R2D).exp().atan()) * R2D - 90.0),
                Projection::Sfl => (x / (y * D2R).cos(), y),
                _ => unreachable!(),
            }
        }
    }

    /// Project native `(φ, θ)` (deg) to intermediate world `(x, y)` (deg).
    fn project(self, phi: f64, theta: f64) -> (f64, f64) {
        if self.is_zenithal() {
            let zeta = (90.0 - theta) * D2R;
            let r = match self {
                Projection::Tan => R2D * zeta.tan(),
                Projection::Sin => R2D * zeta.sin(),
                Projection::Arc => R2D * zeta,
                Projection::Zea => 2.0 * R2D * (zeta / 2.0).sin(),
                Projection::Stg => 2.0 * R2D * (zeta / 2.0).tan(),
                _ => unreachable!(),
            };
            let p = phi * D2R;
            (r * p.sin(), -r * p.cos())
        } else {
            let t = theta * D2R;
            match self {
                Projection::Car => (phi, theta),
                Projection::Cea => (phi, R2D * t.sin()),
                Projection::Mer => (phi, R2D * ((45.0 + theta / 2.0) * D2R).tan().ln()),
                Projection::Sfl => (phi * t.cos(), theta),
                _ => unreachable!(),
            }
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
    /// The (longitude axis, latitude axis, projection, celestial pole) when a
    /// celestial pair is present; `None` for an all-linear system.
    celestial: Option<Celestial>,
}

#[derive(Debug, Clone, Copy)]
struct Celestial {
    lng: usize,
    lat: usize,
    proj: Projection,
    /// Celestial pole `(α_p, δ_p)` and native longitude of the pole `φ_p`
    /// (LONPOLE), all degrees, computed from the fiducial point.
    pole: (f64, f64, f64),
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

        // Build the linear transform A. Precedence: CD, then PC×CDELT, then the
        // legacy CROTA rotation, then a bare CDELT diagonal.
        let has_cd = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get_real(&format!("CD{i}_{j}{a}")).is_some()));
        let has_pc = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get_real(&format!("PC{i}_{j}{a}")).is_some()));
        let mut matrix = vec![0.0; naxis * naxis];
        if has_cd {
            for i in 0..naxis {
                for j in 0..naxis {
                    matrix[i * naxis + j] = header
                        .get_real(&format!("CD{}_{}{a}", i + 1, j + 1))
                        .unwrap_or(0.0);
                }
            }
        } else {
            for i in 0..naxis {
                for j in 0..naxis {
                    let pc = header
                        .get_real(&format!("PC{}_{}{a}", i + 1, j + 1))
                        .unwrap_or(if i == j { 1.0 } else { 0.0 });
                    matrix[i * naxis + j] = cdelt[i] * pc;
                }
            }
            // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC
            // was given, per the convention that CROTA and PC are exclusive).
            if !has_pc
                && let Some((lng, lat, _)) = find_celestial(&ctype)
            {
                let rho = header
                    .get_real(&format!("CROTA{}{a}", lat + 1))
                    .or_else(|| header.get_real(&format!("CROTA{}{a}", lng + 1)))
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
        let inverse = invert(&matrix, naxis).ok_or(FitsError::InvalidValue {
            card: "singular WCS transform matrix".to_string(),
        })?;

        let celestial = find_celestial(&ctype).map(|(lng, lat, proj)| {
            let (phi0, theta0) = proj.reference_point();
            let (alpha0, delta0) = (crval[lng], crval[lat]);
            // LONPOLE default: φ0 if δ0 ≥ θ0, else φ0 + 180° (§8.3).
            let phip = header
                .get_real(&format!("LONPOLE{a}"))
                .unwrap_or(if delta0 >= theta0 { phi0 } else { phi0 + 180.0 });
            // LATPOLE default 90° (disambiguates the two δ_p solutions).
            let thetap = header.get_real(&format!("LATPOLE{a}")).unwrap_or(90.0);
            let pole = compute_pole(phi0, theta0, alpha0, delta0, phip, thetap);
            Celestial {
                lng,
                lat,
                proj,
                pole,
            }
        });

        Ok(Wcs {
            naxis,
            ctype,
            crval,
            crpix,
            matrix,
            inverse,
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
            let (phi, theta) = c.proj.deproject(inter[c.lng], inter[c.lat]);
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
        if let Some(c) = self.celestial {
            let (phi, theta) = celestial_to_native(c.pole, world[c.lng], world[c.lat]);
            let (x, y) = c.proj.project(phi, theta);
            inter[c.lng] = x;
            inter[c.lat] = y;
        }
        // Invert the linear transform, then add back CRPIX.
        let offset = matvec(&self.inverse, &inter, self.naxis);
        (0..self.naxis).map(|i| offset[i] + self.crpix[i]).collect()
    }
}

/// Identify the longitude/latitude axis pair and projection from `CTYPE`s.
fn find_celestial(ctype: &[String]) -> Option<(usize, usize, Projection)> {
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
        (Some(lng), Some(lat), Some(proj)) => Some((lng, lat, proj)),
        _ => None,
    }
}

/// Native spherical (φ, θ) → celestial (α, δ), all degrees, given the celestial
/// pole `(α_p, δ_p, φ_p)` (CG 2002 eq. 2).
fn native_to_celestial(pole: (f64, f64, f64), phi: f64, theta: f64) -> (f64, f64) {
    let (ap, dp, fp) = pole;
    let (tr, dpr, dphi) = (theta * D2R, dp * D2R, (phi - fp) * D2R);
    let sin_d = tr.sin() * dpr.sin() + tr.cos() * dpr.cos() * dphi.cos();
    let dec = sin_d.clamp(-1.0, 1.0).asin() * R2D;
    let y = -tr.cos() * dphi.sin();
    let x = tr.sin() * dpr.cos() - tr.cos() * dpr.sin() * dphi.cos();
    (norm360(ap + y.atan2(x) * R2D), dec)
}

/// Celestial (α, δ) → native spherical (φ, θ), all degrees (CG 2002 eq. 5).
fn celestial_to_native(pole: (f64, f64, f64), ra: f64, dec: f64) -> (f64, f64) {
    let (ap, dp, fp) = pole;
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
fn compute_pole(phi0: f64, theta0: f64, a0: f64, d0: f64, phip: f64, thetap: f64) -> (f64, f64, f64) {
    if (theta0 - 90.0).abs() < 1e-12 {
        return (a0, d0, phip);
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
    let in_range = |x: f64| (-std::f64::consts::FRAC_PI_2..=std::f64::consts::FRAC_PI_2).contains(&x);
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
        (false, false) => c1.clamp(-std::f64::consts::FRAC_PI_2, std::f64::consts::FRAC_PI_2),
    };
    let dp = dpr * R2D;
    // α_p from the fiducial constraint (inverting eq. 2 at (φ0, θ0)).
    let fphi = (phi0 - phip) * D2R;
    let y = -t0.cos() * fphi.sin();
    let x = t0.sin() * dpr.cos() - t0.cos() * dpr.sin() * fphi.cos();
    let ap = a0 - y.atan2(x) * R2D;
    (norm360(ap), dp, phip)
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
