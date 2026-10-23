//! WCS distortions (Calabretta et al., WCS Paper IV draft): a correction to the pixel
//! offsets before the matrix (a prior distortion, SIP) or to the intermediate
//! coordinates after it (a sequent distortion, TPV).

pub(crate) mod sip;
pub(crate) mod tpv;

/// A distortion a WCS applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Distortion {
    /// The Simple Imaging Polynomial (Shupe et al. 2005), a prior distortion of the
    /// first two pixel axes, from the `A_p_q`/`B_p_q` keywords.
    Sip,
    /// The TPV polynomial (fits.gsfc.nasa.gov/registry/tpvwcs), a sequent distortion of
    /// the celestial intermediate coordinates, from the celestial axes' `PVi_m`.
    Tpv,
}

impl Distortion {
    /// The distortion a `CTYPEia` distortion code names; `None` for one this crate does
    /// not evaluate.
    pub(crate) fn from_code(code: &str) -> Option<Self> {
        match code {
            "SIP" => Some(Self::Sip),
            "TPV" => Some(Self::Tpv),
            _ => None,
        }
    }
}

/// Whether `residual` is within the tolerance of `target`: 1e-13, absolute below 1 and
/// relative to `|target|` above it, as wcslib's iterative inverse stops (`dis.c`,
/// `disx2p`); an absolute 1e-13 is under the spacing of a double past about 450.
fn converged(residual: f64, target: f64) -> bool {
    residual.abs() <= 1e-13 * target.abs().max(1.0)
}

/// The most Newton steps an inverse takes, wcslib's `disitermax`.
const MAX_STEPS: usize = 30;

#[cfg(test)]
mod tests;
