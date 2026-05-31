//! Celestial reference frames (`RADESYS`/`EQUINOX`, §8.2) and the rotations
//! between them.
//!
//! Each frame is related to ICRS by a 3×3 rotation `M` (`v_frame = M · v_icrs`);
//! a transform is `v_to = M_to · M_fromᵀ · v_from`. Covered: ICRS, FK5 at any
//! equinox (IAU-1976 precession), and Galactic (the Hipparcos matrix). ICRS and
//! FK5 J2000 are treated as aligned — the ~25 mas frame bias is omitted. FK4
//! (B1950) is parsed but its E-term transform is not yet implemented.

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
    /// [`FitsError::UnsupportedFrame`] if either frame's rotation is unimplemented
    /// (FK4 needs the E-term model).
    pub fn transform(self, lon: f64, lat: f64, to: Frame) -> Result<(f64, f64)> {
        let m_from = self.to_icrs_matrix().ok_or(FitsError::UnsupportedFrame)?;
        let m_to = to.to_icrs_matrix().ok_or(FitsError::UnsupportedFrame)?;
        let v_icrs = m_from.transpose().mul_vec(unit_vector(lon, lat));
        Ok(vector_to_lonlat(m_to.mul_vec(v_icrs)))
    }

    /// Rotation matrix `M` with `v_frame = M · v_icrs`. `None` for frames whose
    /// transform is not yet implemented (FK4).
    fn to_icrs_matrix(self) -> Option<Mat3> {
        match self {
            Frame::Icrs => Some(Mat3::identity()),
            Frame::Fk5 { equinox } => Some(precession_fk5(equinox)),
            Frame::Galactic => Some(GALACTIC),
            Frame::Fk4 { .. } => None,
        }
    }
}

/// IAU-1976 precession (Lieske) matrix from FK5 J2000 to `equinox` (Julian year):
/// `v_equinox = P · v_J2000`, with the standard `R₃(−z)·R₂(θ)·R₃(−ζ)`.
fn precession_fk5(equinox: f64) -> Mat3 {
    let t = (equinox - 2000.0) / 100.0; // Julian centuries from J2000
    let zeta = (2306.2181 * t + 0.30188 * t * t + 0.017998 * t * t * t) * AS2R;
    let z = (2306.2181 * t + 1.09468 * t * t + 0.018203 * t * t * t) * AS2R;
    let theta = (2004.3109 * t - 0.42665 * t * t - 0.041833 * t * t * t) * AS2R;
    r3(-z).mul(&r2(theta)).mul(&r3(-zeta))
}

/// ICRS → Galactic rotation (Hipparcos/IAU): NGP at ICRS (192.85948°, 27.12825°),
/// galactic longitude of the north celestial pole 122.93192°.
const GALACTIC: Mat3 = Mat3([
    [
        -0.054_875_560_416_215_4,
        -0.873_437_090_234_885,
        -0.483_835_015_548_713_2,
    ],
    [
        0.494_109_427_875_583_7,
        -0.444_829_629_960_011_2,
        0.746_982_244_497_219,
    ],
    [
        -0.867_666_149_019_004_7,
        -0.198_076_373_431_201_5,
        0.455_983_776_175_066_9,
    ],
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
