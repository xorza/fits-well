//! [`Sip`]: the Simple Imaging Polynomial (Shupe et al. 2005), a prior distortion of the
//! first two pixel axes.

use crate::fits_error::FitsError;
use crate::fits_error::KeywordDefect;
use crate::fits_error::Result;
use crate::header_model::Header;
use crate::header_model::record_name::RecordName;
use crate::keyword::Keyword;
use crate::world_coordinates::distortion::{MAX_STEPS, converged};

/// The highest order: wcslib reads one digit for `p` and `q` in `A_p_q` (`wcspih.l`).
const MAX_ORDER: usize = 9;

/// The SIP polynomials of a WCS. `(u, v)` are the pixel offsets from `CRPIX`; the
/// forward direction gives `(U, V) = (u + f(u, v), v + g(u, v))` from `A_p_q` and
/// `B_p_q`, which the matrix then maps. `AP_p_q`/`BP_p_q`, an approximate fit of the
/// inverse, only start the Newton solve of the forward polynomials, as wcslib uses them
/// (`dis.h`, "SIP"), so a round trip gives back the pixel.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Sip {
    a: Polynomial,
    b: Polynomial,
    /// `AP` and `BP`, when the header carries them.
    approximate_inverse: Option<[Polynomial; 2]>,
}

/// `Σ c_pq · u^p · v^q` over `p + q ≤ order`.
#[derive(Debug, Clone, PartialEq)]
struct Polynomial {
    order: usize,
    /// `c_pq` at `p · (order + 1) + q`, zero past `p + q = order`.
    coefficients: Vec<f64>,
}

impl Sip {
    /// The SIP polynomials `header` declares, or `None` when it declares none. A header
    /// with one of `A_ORDER` and `B_ORDER`, or one of `AP_ORDER` and `BP_ORDER`, without
    /// the other is `MissingKeyword`; an order past 9 is `InvalidKeywordValue`.
    pub(crate) fn from_header(header: &Header) -> Result<Option<Self>> {
        let Some([a, b]) = Polynomial::pair(header, ["A", "B"])? else {
            return Ok(None);
        };
        Ok(Some(Self {
            a,
            b,
            approximate_inverse: Polynomial::pair(header, ["AP", "BP"])?,
        }))
    }

    /// Write the polynomials as `A_ORDER`, `A_p_q`, `B_ORDER`, `B_p_q`, and `AP`/`BP`
    /// when they were read, each zero term omitted.
    pub(crate) fn write(&self, header: &mut Header) -> Result<()> {
        self.a.write(header, "A")?;
        self.b.write(header, "B")?;
        if let Some([ap, bp]) = &self.approximate_inverse {
            ap.write(header, "AP")?;
            bp.write(header, "BP")?;
        }
        Ok(())
    }

    /// The distorted offsets `(U, V)` of the pixel offsets `(u, v)`.
    pub(crate) fn forward(&self, u: f64, v: f64) -> [f64; 2] {
        [u + self.a.value(u, v), v + self.b.value(u, v)]
    }

    /// The pixel offsets `(u, v)` whose distorted offsets are `(U, V)`: Newton steps on
    /// the forward polynomials, with their exact Jacobian, until each residual is within
    /// wcslib's 1e-13 (`distortion::converged`). `WcsNoConvergence` after 30 steps, or
    /// where the Jacobian is singular, as past a fold of the polynomial.
    pub(crate) fn inverse(&self, target_u: f64, target_v: f64) -> Result<[f64; 2]> {
        let [mut u, mut v] = match &self.approximate_inverse {
            Some([ap, bp]) => [
                target_u + ap.value(target_u, target_v),
                target_v + bp.value(target_u, target_v),
            ],
            None => [target_u, target_v],
        };
        for step in 0..=MAX_STEPS {
            let [forward_u, forward_v] = self.forward(u, v);
            let (residual_u, residual_v) = (forward_u - target_u, forward_v - target_v);
            if converged(residual_u, target_u) && converged(residual_v, target_v) {
                return Ok([u, v]);
            }
            if step == MAX_STEPS {
                break;
            }
            let [fu, fv] = self.a.gradient(u, v);
            let [gu, gv] = self.b.gradient(u, v);
            let (j00, j01, j10, j11) = (1.0 + fu, fv, gu, 1.0 + gv);
            let determinant = j00 * j11 - j01 * j10;
            if determinant == 0.0 || !determinant.is_finite() {
                break;
            }
            u -= (residual_u * j11 - residual_v * j01) / determinant;
            v -= (j00 * residual_v - j10 * residual_u) / determinant;
        }
        Err(FitsError::WcsNoConvergence { algorithm: "SIP" })
    }
}

impl Polynomial {
    /// The two polynomials whose orders `{root}_ORDER` name, or `None` when the header
    /// names neither.
    fn pair(header: &Header, roots: [&str; 2]) -> Result<Option<[Self; 2]>> {
        let [first, second] = roots.map(|root| Self::read(header, root));
        match (first?, second?) {
            (None, None) => Ok(None),
            (Some(first), Some(second)) => Ok(Some([first, second])),
            (Some(_), None) => Err(missing_order(roots[1])),
            (None, Some(_)) => Err(missing_order(roots[0])),
        }
    }

    /// The polynomial of `{root}_ORDER` and `{root}_p_q`, absent terms zero.
    fn read(header: &Header, root: &str) -> Result<Option<Self>> {
        let order_keyword = order_keyword(root);
        let Some(order) = header.get_integer(order_keyword)? else {
            return Ok(None);
        };
        let order = usize::try_from(order)
            .ok()
            .filter(|&order| order <= MAX_ORDER)
            .ok_or(FitsError::InvalidKeywordValue {
                keyword: order_keyword,
                defect: KeywordDefect::Range,
            })?;
        let width = order + 1;
        let mut coefficients = vec![0.0; width * width];
        for p in 0..=order {
            for q in 0..=order - p {
                let keyword = Keyword::try_from(format_args!("{root}_{p}_{q}"))?;
                coefficients[p * width + q] = header.get_real(keyword)?.unwrap_or(0.0);
            }
        }
        Ok(Some(Self {
            order,
            coefficients,
        }))
    }

    fn write(&self, header: &mut Header, root: &str) -> Result<()> {
        header.set(
            order_keyword(root),
            i64::try_from(self.order).expect("an order of at most 9"),
        )?;
        let width = self.order + 1;
        for p in 0..=self.order {
            for q in 0..=self.order - p {
                let c = self.coefficients[p * width + q];
                if c != 0.0 {
                    header.set(Keyword::try_from(format_args!("{root}_{p}_{q}"))?, c)?;
                }
            }
        }
        Ok(())
    }

    /// The powers `x⁰ … x^order`.
    fn powers(&self, x: f64) -> [f64; MAX_ORDER + 1] {
        let mut powers = [1.0; MAX_ORDER + 1];
        for i in 1..=self.order {
            powers[i] = powers[i - 1] * x;
        }
        powers
    }

    fn value(&self, u: f64, v: f64) -> f64 {
        let (up, vp) = (self.powers(u), self.powers(v));
        let width = self.order + 1;
        let mut sum = 0.0;
        for (p, &u_p) in up.iter().enumerate().take(width) {
            for (q, &v_q) in vp.iter().enumerate().take(width - p) {
                sum += self.coefficients[p * width + q] * u_p * v_q;
            }
        }
        sum
    }

    /// `(∂/∂u, ∂/∂v)` of the polynomial.
    fn gradient(&self, u: f64, v: f64) -> [f64; 2] {
        let (up, vp) = (self.powers(u), self.powers(v));
        let width = self.order + 1;
        let (mut du, mut dv) = (0.0, 0.0);
        for p in 0..=self.order {
            for q in 0..=self.order - p {
                let c = self.coefficients[p * width + q];
                if p > 0 {
                    du += p as f64 * c * up[p - 1] * vp[q];
                }
                if q > 0 {
                    dv += q as f64 * c * up[p] * vp[q - 1];
                }
            }
        }
        [du, dv]
    }
}

fn order_keyword(root: &str) -> Keyword {
    Keyword::try_from(format_args!("{root}_ORDER")).expect("a SIP root names a keyword")
}

fn missing_order(root: &str) -> FitsError {
    FitsError::MissingKeyword {
        name: RecordName::Keyword(order_keyword(root)),
    }
}
