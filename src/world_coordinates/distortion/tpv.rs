//! [`Tpv`]: the TPV polynomial (fits.gsfc.nasa.gov/registry/tpvwcs), a sequent
//! distortion of the celestial intermediate coordinates.

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::Keyword;
use crate::world_coordinates::distortion::{MAX_STEPS, converged};
use crate::world_coordinates::read_real;

/// The registered terms, `PVi_0` to `PVi_39`.
const TERM_COUNT: usize = 40;

/// One registered term in its first and second variables `(x, y)`, or a power of
/// `r = √(x² + y²)`.
#[derive(Debug, Clone, Copy)]
enum Term {
    Monomial { x: usize, y: usize },
    Radial(i32),
}

/// The registered order: the constant, then each degree's monomials from `x^d` to `y^d`,
/// with `r`, `r³`, `r⁵` and `r⁷` after degrees 1, 3, 5 and 7.
const TERMS: [Term; TERM_COUNT] = {
    let mut terms = [Term::Radial(0); TERM_COUNT];
    let mut index = 0;
    let mut degree = 0;
    while degree <= 7 {
        let mut y = 0;
        while y <= degree {
            terms[index] = Term::Monomial { x: degree - y, y };
            index += 1;
            y += 1;
        }
        if degree % 2 == 1 {
            terms[index] = Term::Radial(degree as i32);
            index += 1;
        }
        degree += 1;
    }
    terms
};

/// The two TPV polynomials of a celestial pair: `ξ = P₁(x, y)` from the longitude axis's
/// `PV`, and `η = P₂(y, x)` from the latitude axis's, its variables swapped, as wcslib
/// maps them (`wcs.c`, `wcsset`). `x` and `y` are the intermediate coordinates in
/// degrees. An absent term is 0, except `PVi_1`, which is 1, so a header with no `PV`
/// is plain TAN, as the registration states.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Tpv {
    xi: [f64; TERM_COUNT],
    eta: [f64; TERM_COUNT],
}

impl Tpv {
    /// The polynomials the `PV` of the longitude axis `lng` and the latitude axis `lat`
    /// (0-based) declare in the description `alt`.
    pub(crate) fn from_header(
        header: &Header,
        alt: Alternate,
        lng: usize,
        lat: usize,
    ) -> Result<Self> {
        let read = |axis: usize| -> Result<[f64; TERM_COUNT]> {
            let mut coefficients = [0.0; TERM_COUNT];
            coefficients[1] = 1.0;
            for (m, coefficient) in coefficients.iter_mut().enumerate() {
                if let Some(value) = read_real(
                    header,
                    Keyword::fitting(format_args!("PV{}_{m}{alt}", axis + 1)),
                )? {
                    *coefficient = value;
                }
            }
            Ok(coefficients)
        };
        Ok(Self {
            xi: read(lng)?,
            eta: read(lat)?,
        })
    }

    /// The corrected coordinates `(ξ, η)` of the intermediate coordinates `(x, y)`.
    pub(crate) fn forward(&self, x: f64, y: f64) -> [f64; 2] {
        [evaluate(&self.xi, x, y)[0], evaluate(&self.eta, y, x)[0]]
    }

    /// The intermediate coordinates `(x, y)` whose corrected coordinates are `(ξ, η)`:
    /// Newton steps from `(ξ, η)`, with the exact Jacobian, under the SIP stop rule.
    pub(crate) fn inverse(&self, target_xi: f64, target_eta: f64) -> Result<[f64; 2]> {
        let (mut x, mut y) = (target_xi, target_eta);
        for step in 0..=MAX_STEPS {
            let [xi, xi_x, xi_y] = evaluate(&self.xi, x, y);
            let [eta, eta_y, eta_x] = evaluate(&self.eta, y, x);
            let (residual_xi, residual_eta) = (xi - target_xi, eta - target_eta);
            if converged(residual_xi, target_xi) && converged(residual_eta, target_eta) {
                return Ok([x, y]);
            }
            let determinant = xi_x * eta_y - xi_y * eta_x;
            if step == MAX_STEPS || determinant == 0.0 || !determinant.is_finite() {
                break;
            }
            x -= (residual_xi * eta_y - residual_eta * xi_y) / determinant;
            y -= (xi_x * residual_eta - eta_x * residual_xi) / determinant;
        }
        Err(FitsError::WcsNoConvergence { algorithm: "TPV" })
    }
}

/// One polynomial's value at `(x, y)` and its partial derivatives in `x` and `y`.
fn evaluate(coefficients: &[f64; TERM_COUNT], x: f64, y: f64) -> [f64; 3] {
    let mut xp = [1.0; 8];
    let mut yp = [1.0; 8];
    for i in 1..8 {
        xp[i] = xp[i - 1] * x;
        yp[i] = yp[i - 1] * y;
    }
    let r = x.hypot(y);
    let (mut value, mut dx, mut dy) = (0.0, 0.0, 0.0);
    for (&c, term) in coefficients.iter().zip(TERMS) {
        if c == 0.0 {
            continue;
        }
        match term {
            Term::Monomial { x: p, y: q } => {
                value += c * xp[p] * yp[q];
                if p > 0 {
                    dx += c * p as f64 * xp[p - 1] * yp[q];
                }
                if q > 0 {
                    dy += c * q as f64 * xp[p] * yp[q - 1];
                }
            }
            Term::Radial(k) => {
                value += c * r.powi(k);
                // ∂(r^k)/∂x = k·r^(k−2)·x, which tends to k·x/r at r = 0 for k = 1;
                // 0 there by symmetry of the limit.
                if r > 0.0 {
                    let scale = c * f64::from(k) * r.powi(k - 2);
                    dx += scale * x;
                    dy += scale * y;
                }
            }
        }
    }
    [value, dx, dy]
}

#[cfg(test)]
mod tests {
    use crate::world_coordinates::distortion::tpv::{TERMS, Term};

    /// The registered layout: PV1_4 is x², PV1_8 x²y, PV1_11 r³, PV1_23 r⁵, PV1_30 y⁶,
    /// PV1_31 x⁷ and PV1_39 r⁷.
    #[test]
    fn the_terms_follow_the_registered_order() {
        let monomial = |index: usize| match TERMS[index] {
            Term::Monomial { x, y } => Some((x, y)),
            Term::Radial(_) => None,
        };
        let radial = |index: usize| match TERMS[index] {
            Term::Radial(k) => Some(k),
            Term::Monomial { .. } => None,
        };
        assert_eq!(monomial(0), Some((0, 0)));
        assert_eq!(monomial(1), Some((1, 0)));
        assert_eq!(monomial(2), Some((0, 1)));
        assert_eq!(radial(3), Some(1));
        assert_eq!(monomial(4), Some((2, 0)));
        assert_eq!(monomial(8), Some((2, 1)));
        assert_eq!(radial(11), Some(3));
        assert_eq!(monomial(12), Some((4, 0)));
        assert_eq!(radial(23), Some(5));
        assert_eq!(monomial(30), Some((0, 6)));
        assert_eq!(monomial(31), Some((7, 0)));
        assert_eq!(monomial(38), Some((0, 7)));
        assert_eq!(radial(39), Some(7));
    }
}
