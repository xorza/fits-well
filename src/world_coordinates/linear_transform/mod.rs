//! The linear layer of a WCS (§8.1) and its inverse.

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::AltSuffix;
use crate::keyword::key;
use crate::world_coordinates::D2R;
use crate::world_coordinates::first_real;
use crate::world_coordinates::projected_celestial_axes::ProjectedCelestialAxes;
use crate::world_coordinates::wcs_axis::WcsAxis;

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
/// The `CD`/`PC`/`CROTA` conventions are resolved and validated as the header is
/// read; the scaling and the inversion wait for the axis parse to report each axis's
/// unit factor.
#[derive(Debug)]
pub(super) struct LinearMatrix {
    values: Vec<f64>,
    naxis: usize,
}

/// One `PCi_ja`/`CDi_ja` keyword the header carries, with its zero-based cell.
#[derive(Debug)]
struct MatrixCell {
    keyword: String,
    row: usize,
    column: usize,
}

impl MatrixCell {
    const fn index(&self, naxis: usize) -> usize {
        self.row * naxis + self.column
    }
}

/// The `{root}i_j{alt}` keywords the header carries with `1 ≤ i, j ≤ naxis`, found in one pass
/// over its cards. Probing all `naxis²` names instead costs a million lookups for a header that
/// claims `WCSAXES = 999`. An index is matched exactly as a keyword name spells it: decimal
/// digits with no leading zero, so `PC01_02` is a different keyword, as it is to a reader that
/// asks for `PC1_2` by name.
fn matrix_cells(header: &Header, root: &str, alt: AltSuffix, naxis: usize) -> Vec<MatrixCell> {
    let index = |digits: &str| -> Option<usize> {
        let canonical = !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit());
        let value: usize = digits.parse().ok().filter(|_| canonical)?;
        (1..=naxis).contains(&value).then_some(value - 1)
    };
    let mut cells: Vec<MatrixCell> = header
        .iter()
        .filter(|entry| entry.value.is_some())
        .filter_map(|entry| {
            let rest = entry
                .keyword
                .strip_prefix(root)?
                .strip_suffix(alt.as_str())?;
            let (row, column) = rest.split_once('_')?;
            Some(MatrixCell {
                keyword: entry.keyword.to_owned(),
                row: index(row)?,
                column: index(column)?,
            })
        })
        .collect();
    // A repeated keyword names one cell; the header's own lookup reads its first card.
    cells.sort_by_key(|cell| (cell.row, cell.column));
    cells.dedup_by_key(|cell| (cell.row, cell.column));
    cells
}

impl LinearMatrix {
    /// Read the header's linear keywords.
    ///
    /// Precedence (§8.1): `CDi_j` if present, else `PCi_j × CDELTi`, else the legacy
    /// `CROTAi` rotation of the celestial pair, else a bare `CDELT` diagonal. `PC` with
    /// `CD`, or `PC` with `CROTA`, is rejected rather than resolved. A `CROTAi` beside a
    /// `CD` matrix is ignored, as wcslib ignores it: legacy headers often keep a
    /// redundant `CROTA2` next to the matrix that supersedes it.
    pub(super) fn from_header(
        header: &Header,
        a: AltSuffix,
        naxis: usize,
        cdelt: &[f64],
        celestial_axes: Option<ProjectedCelestialAxes>,
    ) -> Result<LinearMatrix> {
        let cd = matrix_cells(header, "CD", a, naxis);
        let pc = matrix_cells(header, "PC", a, naxis);
        let has_cd = !cd.is_empty();
        let has_pc = !pc.is_empty();
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
            for cell in cd {
                values[cell.index(naxis)] = header.get_real(&cell.keyword)?.unwrap_or(0.0);
            }
            return Ok(LinearMatrix { values, naxis });
        }
        for (i, &scale) in cdelt.iter().enumerate() {
            values[i * naxis + i] = scale;
        }
        for cell in pc {
            values[cell.index(naxis)] =
                cdelt[cell.row] * header.get_real(&cell.keyword)?.unwrap_or(0.0);
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
        let inverse = invert(&self.values, self.naxis).ok_or(FitsError::InvalidWcs {
            detail: "singular WCS transform matrix".to_string(),
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
/// pivoting. Returns `None` if singular: when a pivot is no larger than the rounding
/// elimination leaves in an entry, about `n·ε` of the largest one (Higham, *Accuracy
/// and Stability of Numerical Algorithms*, §9.3, for the typical growth of partial
/// pivoting). The test is relative, so a matrix in tiny units is not singular.
fn invert(m: &[f64], n: usize) -> Option<Vec<f64>> {
    let singular = n as f64 * f64::EPSILON * m.iter().fold(0.0f64, |max, &x| max.max(x.abs()));
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
        if a[pivot * n + col].abs() <= singular {
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
    use crate::world_coordinates::linear_transform::LinearTransform;

    /// The resolved linear matrix, row-major, for tests that assert which of the
    /// `CD`/`PC`/`CROTA` conventions a header resolved to.
    pub(crate) fn matrix(transform: &LinearTransform) -> &[f64] {
        &transform.matrix
    }
}

#[cfg(test)]
mod tests;
