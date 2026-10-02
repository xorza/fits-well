//! Per-axis source metadata for a parsed WCS.

use crate::wcs::spectral_frame::SpectralFrame;

/// Immutable source metadata for one WCS axis.
#[derive(Debug, Clone, PartialEq)]
pub struct WcsAxis {
    /// The `CTYPEi` string.
    pub ctype: String,
    /// `CUNITi` as declared.
    pub cunit: String,
    /// `CRVALi` — world coordinate at the reference pixel, in [`Self::cunit`].
    pub crval: f64,
    /// `CRPIXi` — reference pixel (1-based).
    pub crpix: f64,
    /// Spectral reference metadata for a spectral axis.
    pub spectral_frame: Option<SpectralFrame>,
}
