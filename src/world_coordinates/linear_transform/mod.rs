//! The linear layer of a WCS (§8.1) and its inverse.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::header_model::RecordView;
use crate::keyword::Keyword;
use crate::world_coordinates::D2R;
use crate::world_coordinates::axis::Axis;
use crate::world_coordinates::first_real;
use crate::world_coordinates::projected_celestial_axes::ProjectedCelestialAxes;
use crate::world_coordinates::read_value;

/// The matrix `A` mapping `(pixel − CRPIX)` to intermediate world coordinates, held
/// beside its LU factors, which solve the inverse direction in place. Both are row-major
/// `naxis²`.
#[derive(Debug, Clone)]
pub(super) struct LinearTransform {
    matrix: Vec<f64>,
    factors: LuFactors,
    naxis: usize,
}

/// The LU factors of a row-major `n×n` matrix `A` with partial pivoting, `P·A = L·U`: `U`
/// on and above the diagonal, the unit-diagonal `L` below it, and `P` as the row each
/// step swapped into place. Solving with them is as accurate as a product with the
/// explicit inverse or better, and it needs no buffer beside the right-hand side
/// (Higham, *Accuracy and Stability of Numerical Algorithms*, §14.1).
#[derive(Debug, Clone)]
struct LuFactors {
    lu: Vec<f64>,
    /// One row for each column, so its length is `n`.
    swaps: Vec<usize>,
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
    keyword: Keyword,
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
fn matrix_cells(header: &Header, root: &str, alt: Alternate, naxis: usize) -> Vec<MatrixCell> {
    let index = |digits: &str| -> Option<usize> {
        let canonical = !digits.is_empty()
            && !digits.starts_with('0')
            && digits.bytes().all(|byte| byte.is_ascii_digit());
        let value: usize = digits.parse().ok().filter(|_| canonical)?;
        (1..=naxis).contains(&value).then_some(value - 1)
    };
    let mut cells: Vec<MatrixCell> = header
        .iter()
        .filter_map(|record| {
            let RecordView::Valued { keyword, .. } = record else {
                return None;
            };
            let rest = keyword
                .as_str()
                .strip_prefix(root)?
                .strip_suffix(alt.as_str())?;
            let (row, column) = rest.split_once('_')?;
            Some(MatrixCell {
                keyword: *keyword,
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
        alt: Alternate,
        naxis: usize,
        cdelt: &[f64],
        celestial_axes: Option<ProjectedCelestialAxes>,
    ) -> Result<Self> {
        let cd = matrix_cells(header, "CD", alt, naxis);
        let pc = matrix_cells(header, "PC", alt, naxis);
        let has_cd = !cd.is_empty();
        let has_pc = !pc.is_empty();
        let has_crota = (1..=naxis)
            .any(|i| read_value(header, Keyword::fitting(format_args!("CROTA{i}{alt}"))).is_some());
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
                values[cell.index(naxis)] = header.get_real(cell.keyword)?.unwrap_or(0.0);
            }
            return Ok(Self { values, naxis });
        }
        for (i, &scale) in cdelt.iter().enumerate() {
            values[i * naxis + i] = scale;
        }
        for cell in pc {
            values[cell.index(naxis)] =
                cdelt[cell.row] * header.get_real(cell.keyword)?.unwrap_or(0.0);
        }
        // Legacy CROTA: rotate the celestial 2-axis sub-block (only when no PC was
        // given, per the convention that CROTA and PC are exclusive).
        if !has_pc && let Some(axes) = celestial_axes {
            let lng = axes.longitude;
            let lat = axes.latitude;
            let rho = first_real(
                header,
                Keyword::fitting(format_args!("CROTA{}{alt}", lat + 1)),
                Keyword::fitting(format_args!("CROTA{}{alt}", lng + 1)),
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
        Ok(Self { values, naxis })
    }

    /// Scale row `i` by `axis_scales[i]` and factor.
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
        let factors =
            LuFactors::new(&self.values, self.naxis).ok_or_else(|| FitsError::InvalidWcs {
                detail: "singular WCS transform matrix".to_string(),
            })?;
        Ok(LinearTransform {
            matrix: self.values,
            factors,
            naxis: self.naxis,
        })
    }
}

impl LinearTransform {
    /// Write the intermediate world coordinate of the complete 1-based `pixel`
    /// coordinate to `out`.
    pub(super) fn intermediate(&self, pixel: &[f64], axes: &[Axis], out: &mut [f64]) {
        debug_assert_eq!(out.len(), self.naxis);
        for (row, value) in self.matrix.chunks_exact(self.naxis).zip(out) {
            *value = offset_row(row, pixel, axes);
        }
    }

    /// One axis's intermediate world coordinate, without evaluating the others.
    pub(super) fn intermediate_axis(&self, axis: usize, pixel: &[f64], axes: &[Axis]) -> f64 {
        let row = &self.matrix[axis * self.naxis..(axis + 1) * self.naxis];
        offset_row(row, pixel, axes)
    }

    /// Replace the complete intermediate world coordinate in `coordinates` with its
    /// 1-based pixel coordinate, the inverse of [`LinearTransform::intermediate`].
    pub(super) fn pixel(&self, coordinates: &mut [f64], axes: &[Axis]) {
        self.factors.solve(coordinates);
        for (value, axis) in coordinates.iter_mut().zip(axes) {
            *value += axis.crpix;
        }
    }
}

/// One matrix row applied to `(pixel − CRPIX)`.
fn offset_row(row: &[f64], pixel: &[f64], axes: &[Axis]) -> f64 {
    row.iter()
        .zip(pixel)
        .zip(axes)
        .map(|((&coefficient, &value), axis)| coefficient * (value - axis.crpix))
        .sum()
}

impl LuFactors {
    /// Factor `m` by Gaussian elimination with partial pivoting. Returns `None` if `m` is
    /// singular: when a pivot is no larger than the rounding elimination leaves in an
    /// entry, about `n·ε` of the largest one (Higham, §9.3, for the typical growth of
    /// partial pivoting). The test is relative, so a matrix in tiny units is not
    /// singular.
    fn new(m: &[f64], n: usize) -> Option<Self> {
        let singular = n as f64 * f64::EPSILON * m.iter().fold(0.0f64, |max, &x| max.max(x.abs()));
        let mut lu = m.to_vec();
        let mut swaps = Vec::with_capacity(n);
        for col in 0..n {
            let mut pivot = col;
            for r in (col + 1)..n {
                if lu[r * n + col].abs() > lu[pivot * n + col].abs() {
                    pivot = r;
                }
            }
            if lu[pivot * n + col].abs() <= singular {
                return None;
            }
            if pivot != col {
                for k in 0..n {
                    lu.swap(col * n + k, pivot * n + k);
                }
            }
            swaps.push(pivot);
            let d = lu[col * n + col];
            for r in (col + 1)..n {
                let f = lu[r * n + col] / d;
                lu[r * n + col] = f;
                if f != 0.0 {
                    for k in (col + 1)..n {
                        lu[r * n + k] -= f * lu[col * n + k];
                    }
                }
            }
        }
        Some(Self { lu, swaps })
    }

    /// Replace `b` with the `x` that solves `A·x = b`.
    fn solve(&self, b: &mut [f64]) {
        let n = self.swaps.len();
        debug_assert_eq!(b.len(), n);
        for (col, &pivot) in self.swaps.iter().enumerate() {
            b.swap(col, pivot);
        }
        for i in 1..n {
            let row = &self.lu[i * n..i * n + i];
            b[i] -= row.iter().zip(&b[..i]).map(|(&l, &y)| l * y).sum::<f64>();
        }
        for i in (0..n).rev() {
            let row = &self.lu[i * n..(i + 1) * n];
            let tail: f64 = row[i + 1..]
                .iter()
                .zip(&b[i + 1..])
                .map(|(&u, &x)| u * x)
                .sum();
            b[i] = (b[i] - tail) / row[i];
        }
    }
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
