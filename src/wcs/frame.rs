//! Celestial reference frames (`RADESYS`/`EQUINOX`, §8.2) and the rotations
//! between them.
//!
//! Transforms route through ICRS unit vectors. Covered, all matched against
//! astropy: ICRS, FK5 at any equinox (IAU-2000 frame bias + IAU-1976 precession),
//! Galactic, and FK4 B1950 (frame rotation + E-terms of aberration). FK4 at other
//! equinoxes (needs Newcomb pre-precession) errors with
//! [`FitsError::UnsupportedFrame`].

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

const D2R: f64 = std::f64::consts::PI / 180.0;
const R2D: f64 = 180.0 / std::f64::consts::PI;
/// Arcseconds → radians.
const AS2R: f64 = D2R / 3600.0;

/// A celestial reference frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Frame {
    /// International Celestial Reference System (the modern default).
    Icrs,
    /// FK5 equatorial at the given equinox (Julian year, e.g. `2000.0`).
    Fk5 { equinox: f64 },
    /// FK4 equatorial at the given equinox (Besselian year, e.g. `1950.0`).
    Fk4 { equinox: f64 },
    /// Galactic coordinates.
    Galactic,
}

impl Frame {
    /// Parse `RADESYS`/`EQUINOX` from a header (with optional alternate suffix).
    /// `RADESYS` defaults to `FK4` for `EQUINOX < 1984`, else `FK5`, else `ICRS`
    /// when neither is present (§8.2 / the pre-1984 convention).
    pub fn from_header(header: &Header, alt: Option<char>) -> Frame {
        let a = alt.map(|c| c.to_string()).unwrap_or_default();
        let equinox = header
            .get_real(&format!("EQUINOX{a}"))
            .or_else(|| header.get_real("EPOCH")); // legacy EPOCH keyword
        let radesys = header.get_text(&format!("RADESYS{a}")).map(str::trim);
        match radesys {
            Some("ICRS") => Frame::Icrs,
            Some("FK5") => Frame::Fk5 {
                equinox: equinox.unwrap_or(2000.0),
            },
            Some("FK4") | Some("FK4-NO-E") => Frame::Fk4 {
                equinox: equinox.unwrap_or(1950.0),
            },
            Some("GALACTIC") => Frame::Galactic,
            _ => match equinox {
                Some(e) if e < 1984.0 => Frame::Fk4 { equinox: e },
                Some(e) => Frame::Fk5 { equinox: e },
                None => Frame::Icrs,
            },
        }
    }

    /// Transform `(lon, lat)` in degrees from this frame to `to`. Errors with
    /// [`FitsError::UnsupportedFrame`] for FK4 at an equinox other than B1950
    /// (other equinoxes need Newcomb pre-precession).
    pub fn transform(self, lon: f64, lat: f64, to: Frame) -> Result<(f64, f64)> {
        let v_icrs = self.to_icrs_vec(unit_vector(lon, lat))?;
        Ok(vector_to_lonlat(to.icrs_to_frame_vec(v_icrs)?))
    }

    /// A direction vector in this frame → the same direction in ICRS.
    fn to_icrs_vec(self, v: [f64; 3]) -> Result<[f64; 3]> {
        match self {
            Frame::Fk4 { equinox } => {
                if (equinox - 1950.0).abs() > 1e-6 {
                    return Err(FitsError::UnsupportedFrame);
                }
                // FK4 B1950 → FK5 J2000 (remove E-terms, then rotate) → ICRS.
                let fk5 = FK4_TO_FK5.mul_vec(remove_eterms(v));
                Ok(FK5_FROM_ICRS.transpose().mul_vec(fk5))
            }
            _ => {
                let m = self.matrix().ok_or(FitsError::UnsupportedFrame)?;
                Ok(m.transpose().mul_vec(v))
            }
        }
    }

    /// A direction vector in ICRS → the same direction in this frame.
    fn icrs_to_frame_vec(self, v: [f64; 3]) -> Result<[f64; 3]> {
        match self {
            Frame::Fk4 { equinox } => {
                if (equinox - 1950.0).abs() > 1e-6 {
                    return Err(FitsError::UnsupportedFrame);
                }
                let fk5 = FK5_FROM_ICRS.mul_vec(v);
                Ok(add_eterms(FK4_TO_FK5.transpose().mul_vec(fk5)))
            }
            _ => Ok(self.matrix().ok_or(FitsError::UnsupportedFrame)?.mul_vec(v)),
        }
    }

    /// Linear rotation matrix `M` with `v_frame = M · v_icrs` (non-FK4 frames).
    fn matrix(self) -> Option<Mat3> {
        match self {
            Frame::Icrs => Some(Mat3::identity()),
            // FK5(eq) ← ICRS = precession(eq) · frame-bias.
            Frame::Fk5 { equinox } => Some(precession_fk5(equinox).mul(&FK5_FROM_ICRS)),
            Frame::Galactic => Some(GALACTIC),
            Frame::Fk4 { .. } => None,
        }
    }
}

/// FK4 E-terms of aberration (the constant part), as a direction-vector offset.
const ETERMS: [f64; 3] = [-1.625_57e-6, -0.319_19e-6, -0.138_43e-6];

/// Remove the E-terms of aberration from an FK4 direction vector (SLALIB-style).
fn remove_eterms(r: [f64; 3]) -> [f64; 3] {
    let w = r[0] * ETERMS[0] + r[1] * ETERMS[1] + r[2] * ETERMS[2];
    std::array::from_fn(|i| r[i] - ETERMS[i] + w * r[i])
}

/// Add the E-terms back (iterative inverse of [`remove_eterms`]).
fn add_eterms(r: [f64; 3]) -> [f64; 3] {
    let mut s = r;
    for _ in 0..3 {
        let w = s[0] * ETERMS[0] + s[1] * ETERMS[1] + s[2] * ETERMS[2];
        s = std::array::from_fn(|i| r[i] + ETERMS[i] - w * s[i]);
    }
    s
}

/// FK4 (B1950) → FK5 (J2000) position rotation (precession + frame rotation;
/// Murray 1989, the position block of the SLA FK425 6×6).
const FK4_TO_FK5: Mat3 = Mat3([
    [0.999_925_678_2, -0.011_182_061_1, -0.004_857_947_7],
    [0.011_182_061_0, 0.999_937_478_4, -0.000_027_176_5],
    [0.004_857_947_9, -0.000_027_147_4, 0.999_988_199_7],
]);

/// IAU-1976 precession (Lieske) matrix from FK5 J2000 to `equinox` (Julian year):
/// `v_equinox = P · v_J2000`, with the standard `R₃(−z)·R₂(θ)·R₃(−ζ)`. This is the
/// FITS-WCS FK5 model (bit-identical to `erfa.pmat76`); astropy applies the newer
/// IAU-2006 model to FK5, which differs by ~tens of mas over a few decades.
fn precession_fk5(equinox: f64) -> Mat3 {
    let t = (equinox - 2000.0) / 100.0; // Julian centuries from J2000
    let zeta = (2306.2181 * t + 0.30188 * t * t + 0.017998 * t * t * t) * AS2R;
    let z = (2306.2181 * t + 1.09468 * t * t + 0.018203 * t * t * t) * AS2R;
    let theta = (2004.3109 * t - 0.42665 * t * t - 0.041833 * t * t * t) * AS2R;
    r3(-z).mul(&r2(theta)).mul(&r3(-zeta))
}

/// `v_galactic = GALACTIC · v_icrs` — the ICRS→Galactic rotation (matches astropy).
const GALACTIC: Mat3 = Mat3([
    [
        -5.487_565_771_259e-2,
        -8.734_370_519_556e-1,
        -4.838_350_736_167e-1,
    ],
    [
        4.941_094_371_927e-1,
        -4.448_297_212_233e-1,
        7.469_821_839_867e-1,
    ],
    [
        -8.676_661_375_597e-1,
        -1.980_763_372_730e-1,
        4.559_838_136_873e-1,
    ],
]);

/// `v_fk5(J2000) = FK5_FROM_ICRS · v_icrs` — the IAU-2000 frame bias (~25 mas),
/// extracted from astropy; the unit diagonal omits the negligible ~1e-15 terms.
const FK5_FROM_ICRS: Mat3 = Mat3([
    [1.0, -1.110_223_329e-7, -4.411_804_498e-8],
    [1.110_223_374e-7, 1.0, 9.647_792_251e-8],
    [4.411_803_432e-8, -9.647_792_744e-8, 1.0],
]);

/// A 3×3 row-major rotation matrix.
#[derive(Debug, Clone, Copy)]
struct Mat3([[f64; 3]; 3]);

impl Mat3 {
    fn identity() -> Mat3 {
        Mat3([[1.0, 0.0, 0.0], [0.0, 1.0, 0.0], [0.0, 0.0, 1.0]])
    }

    fn mul(&self, o: &Mat3) -> Mat3 {
        let mut m = [[0.0; 3]; 3];
        for (i, row) in m.iter_mut().enumerate() {
            for (j, cell) in row.iter_mut().enumerate() {
                *cell = (0..3).map(|k| self.0[i][k] * o.0[k][j]).sum();
            }
        }
        Mat3(m)
    }

    fn mul_vec(&self, v: [f64; 3]) -> [f64; 3] {
        std::array::from_fn(|i| (0..3).map(|k| self.0[i][k] * v[k]).sum())
    }

    fn transpose(&self) -> Mat3 {
        Mat3(std::array::from_fn(|i| {
            std::array::from_fn(|j| self.0[j][i])
        }))
    }
}

/// Rotation about the y-axis (R₂).
fn r2(a: f64) -> Mat3 {
    let (s, c) = a.sin_cos();
    Mat3([[c, 0.0, -s], [0.0, 1.0, 0.0], [s, 0.0, c]])
}

/// Rotation about the z-axis (R₃).
fn r3(a: f64) -> Mat3 {
    let (s, c) = a.sin_cos();
    Mat3([[c, s, 0.0], [-s, c, 0.0], [0.0, 0.0, 1.0]])
}

/// `(lon, lat)` in degrees → a unit direction vector.
fn unit_vector(lon: f64, lat: f64) -> [f64; 3] {
    let (lo, la) = (lon * D2R, lat * D2R);
    [la.cos() * lo.cos(), la.cos() * lo.sin(), la.sin()]
}

/// A direction vector → `(lon, lat)` in degrees, `lon ∈ [0, 360)`.
fn vector_to_lonlat(v: [f64; 3]) -> (f64, f64) {
    let lon = v[1].atan2(v[0]) * R2D;
    let lat = v[2].clamp(-1.0, 1.0).asin() * R2D;
    (lon.rem_euclid(360.0), lat)
}
