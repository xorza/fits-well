//! The linear layer of a WCS (§8.1) and its inverse.

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::key;
use crate::wcs::D2R;
use crate::wcs::first_real;
use crate::wcs::projected_celestial_axes::ProjectedCelestialAxes;
use crate::wcs::wcs_axis::WcsAxis;

/// The matrix `A` mapping `(pixel − CRPIX)` to intermediate world coordinates, held
/// beside its inverse so both directions stay consistent by construction. Both are
/// row-major `naxis²`.
#[derive(Debug, Clone)]
pub(super) struct LinearTransform {
    matrix: Vec<f64>,
    inverse: Vec<f64>,
    naxis: usize,
}

/// The matrix as the header spells it, before the per-axis unit factors are known.
/// The `CD`/`PC`/`CROTA` conventions are mutually exclusive, so they are resolved
/// and validated as the header is read; the scaling and the inversion wait for the
/// axis parse to report each axis's unit factor.
#[derive(Debug)]
pub(super) struct LinearMatrix {
    values: Vec<f64>,
    naxis: usize,
}

impl LinearMatrix {
    /// Read the header's linear keywords.
    ///
    /// Precedence (§8.1): `CDi_j` if present, else `PCi_j × CDELTi`, else the legacy
    /// `CROTAi` rotation of the celestial pair, else a bare `CDELT` diagonal. A header
    /// mixing the conventions is rejected rather than silently resolved.
    pub(super) fn from_header(
        header: &Header,
        a: AltSuffix,
        naxis: usize,
        cdelt: &[f64],
        celestial_axes: Option<ProjectedCelestialAxes>,
    ) -> Result<LinearMatrix> {
        let has_cd = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get(key!("CD{i}_{j}{a}").as_str()).is_some()));
        let has_pc = (1..=naxis)
            .any(|i| (1..=naxis).any(|j| header.get(key!("PC{i}_{j}{a}").as_str()).is_some()));
        let has_crota = (1..=naxis).any(|i| header.get(key!("CROTA{i}{a}").as_str()).is_some());
        if has_cd && has_pc {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "PC and CD conventions overlap",
            });
        }
        if has_pc && has_crota {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "PC and CROTA conventions overlap",
            });
        }
        let mut values = vec![0.0; naxis * naxis];
        if has_cd {
            for i in 0..naxis {
                for j in 0..naxis {
                    values[i * naxis + j] = header
                        .get_real(key!("CD{}_{}{a}", i + 1, j + 1).as_str())?
                        .unwrap_or(0.0);
                }
            }
            return Ok(LinearMatrix { values, naxis });
        }
        for i in 0..naxis {
            for j in 0..naxis {
                let pc = header
                    .get_real(key!("PC{}_{}{a}", i + 1, j + 1).as_str())?
                    .unwrap_or(if i == j { 1.0 } else { 0.0 });
                values[i * naxis + j] = cdelt[i] * pc;
            }
        }
        // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC was
        // given, per the convention that CROTA and PC are exclusive).
        if !has_pc && let Some(axes) = celestial_axes {
            let lng = axes.longitude;
            let lat = axes.latitude;
            let rho = first_real(
                header,
                key!("CROTA{}{a}", lat + 1).as_str(),
                key!("CROTA{}{a}", lng + 1).as_str(),
            )?
            .unwrap_or(0.0);
            if rho != 0.0 {
                let (c, s) = ((rho * D2R).cos(), (rho * D2R).sin());
                values[lng * naxis + lng] = cdelt[lng] * c;
                values[lng * naxis + lat] = -cdelt[lat] * s;
                values[lat * naxis + lng] = cdelt[lng] * s;
                values[lat * naxis + lat] = cdelt[lat] * c;
            }
        }
        Ok(LinearMatrix { values, naxis })
    }

    /// Scale row `i` by `axis_scales[i]` and invert.
    ///
    /// §8.2: `CRVAL`/`CDELT` are in `CUNITia` units, but the transforms run in degrees
    /// (celestial) or the Table-25 default (spectral), so each axis's whole matrix row
    /// carries its unit factor. The inverse is computed from the scaled matrix, so both
    /// directions stay consistent.
    pub(super) fn scaled(mut self, axis_scales: &[f64]) -> Result<LinearTransform> {
        debug_assert_eq!(axis_scales.len(), self.naxis);
        for (axis, &scale) in axis_scales.iter().enumerate() {
            for column in 0..self.naxis {
                self.values[axis * self.naxis + column] *= scale;
            }
        }
        let inverse = invert(&self.values, self.naxis).ok_or(FitsError::InvalidValue {
            card: "singular WCS transform matrix".to_string(),
        })?;
        Ok(LinearTransform {
            matrix: self.values,
            inverse,
            naxis: self.naxis,
        })
    }
}

impl LinearTransform {
    /// Intermediate world coordinates for a complete 1-based `pixel` coordinate.
    pub(super) fn intermediate(&self, pixel: &[f64], axes: &[WcsAxis]) -> Vec<f64> {
        self.matrix
            .chunks_exact(self.naxis)
            .map(|row| offset_row(row, pixel, axes))
            .collect()
    }

    /// One axis's intermediate world coordinate, without evaluating the others.
    pub(super) fn intermediate_axis(&self, axis: usize, pixel: &[f64], axes: &[WcsAxis]) -> f64 {
        let row = &self.matrix[axis * self.naxis..(axis + 1) * self.naxis];
        offset_row(row, pixel, axes)
    }

    /// The 1-based pixel coordinate for a complete intermediate world coordinate —
    /// the inverse of [`LinearTransform::intermediate`].
    pub(super) fn pixel(&self, intermediate: &[f64], axes: &[WcsAxis]) -> Vec<f64> {
        self.inverse
            .chunks_exact(self.naxis)
            .zip(axes)
            .map(|(row, axis)| {
                let offset: f64 = row.iter().zip(intermediate).map(|(&a, &b)| a * b).sum();
                offset + axis.crpix
            })
            .collect()
    }
}

/// One matrix row applied to `(pixel − CRPIX)`.
fn offset_row(row: &[f64], pixel: &[f64], axes: &[WcsAxis]) -> f64 {
    row.iter()
        .zip(pixel)
        .zip(axes)
        .map(|((&coefficient, &value), axis)| coefficient * (value - axis.crpix))
        .sum()
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

#[cfg(test)]
pub(crate) mod internals {
    use crate::wcs::linear_transform::LinearTransform;

    /// The resolved linear matrix, row-major, for tests that assert which of the
    /// `CD`/`PC`/`CROTA` conventions a header resolved to.
    pub(crate) fn matrix(transform: &LinearTransform) -> &[f64] {
        &transform.matrix
    }
}

#[cfg(test)]
mod tests;
