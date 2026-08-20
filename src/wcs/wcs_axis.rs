//! Per-axis source metadata for a parsed WCS.

use crate::wcs::spectral_frame::SpectralFrame;

/// Immutable source metadata for one WCS axis.
#[derive(Debug, Clone, PartialEq)]
pub struct WcsAxis {
    /// The `CTYPEi` string.
    pub ctype: String,
    /// Declared `CUNITi`, normalized only where the transform requires standard
    /// projection units.
    pub cunit: String,
    /// `CRVALi` — world coordinate at the reference pixel.
    pub crval: f64,
    /// `CRPIXi` — reference pixel (1-based).
    pub crpix: f64,
    /// Spectral reference metadata for a spectral axis.
    pub spectral_frame: Option<SpectralFrame>,
}
