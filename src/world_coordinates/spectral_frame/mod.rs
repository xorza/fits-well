//! Reference-frame and rest-value metadata for a spectral WCS axis (§8.4).

use crate::alternate::Alternate;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
use crate::world_coordinates::header_read;

/// A spectral reference system: one of FITS Table 27, or another the header names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SpectralReferenceFrame {
    Topocentric,
    Geocentric,
    Barycentric,
    Heliocentric,
    LsrKinematic,
    LsrDynamic,
    Galactocentric,
    LocalGroup,
    CmbDipole,
    Source,
    /// A system Table 27 does not list, as the header names it, blanks trimmed: the
    /// spectral axis reads as it would in one it lists.
    Other(String),
}

/// Resolved reference-frame and rest-value metadata for a spectral WCS axis.
#[derive(Debug, Clone, PartialEq)]
pub struct SpectralFrame {
    /// `SPECSYSa`; `None` when the coordinate frame was not declared.
    pub coordinate: Option<SpectralReferenceFrame>,
    /// `SSYSOBSa`, including its `TOPOCENT` default.
    pub observer: SpectralReferenceFrame,
    /// `RESTFRQa` and `RESTWAVa`, each finite and positive when given.
    pub rest: SpectralRest,
}

impl SpectralReferenceFrame {
    /// The code that names this frame, as `SPECSYSa` and `SSYSOBSa` hold it.
    pub(crate) fn code(&self) -> &str {
        match self {
            Self::Topocentric => "TOPOCENT",
            Self::Geocentric => "GEOCENTR",
            Self::Barycentric => "BARYCENT",
            Self::Heliocentric => "HELIOCEN",
            Self::LsrKinematic => "LSRK",
            Self::LsrDynamic => "LSRD",
            Self::Galactocentric => "GALACTOC",
            Self::LocalGroup => "LOCALGRP",
            Self::CmbDipole => "CMBDIPOL",
            Self::Source => "SOURCE",
            Self::Other(code) => code,
        }
    }

    fn parse(value: &str) -> Self {
        match value.trim() {
            "TOPOCENT" => Self::Topocentric,
            "GEOCENTR" => Self::Geocentric,
            "BARYCENT" => Self::Barycentric,
            "HELIOCEN" => Self::Heliocentric,
            "LSRK" => Self::LsrKinematic,
            "LSRD" => Self::LsrDynamic,
            "GALACTOC" => Self::Galactocentric,
            "LOCALGRP" => Self::LocalGroup,
            "CMBDIPOL" => Self::CmbDipole,
            "SOURCE" => Self::Source,
            value => Self::Other(value.to_string()),
        }
    }
}

impl SpectralFrame {
    /// Resolve `SPECSYSa`/`SSYSOBSa` and the `RESTFRQa`/`RESTWAVa` rest values from
    /// an image header.
    pub(super) fn from_header(header: &Header, alt: Alternate) -> Result<Self> {
        let rest = Self::rest(header, alt)?;
        let coordinate = header_read::text(header, Keyword::fitting(format_args!("SPECSYS{alt}")))?
            .map(SpectralReferenceFrame::parse);
        let observer = header_read::text(header, Keyword::fitting(format_args!("SSYSOBS{alt}")))?
            .map_or(
                SpectralReferenceFrame::Topocentric,
                SpectralReferenceFrame::parse,
            );
        Ok(Self {
            coordinate,
            observer,
            rest,
        })
    }

    fn rest(header: &Header, alt: Alternate) -> Result<SpectralRest> {
        let mut frequency =
            header_read::real(header, Keyword::fitting(format_args!("RESTFRQ{alt}")))?;
        if frequency.is_none() && !alt.is_alternate() {
            frequency = header.get_real(keywords::RESTFREQ)?;
        }
        let wavelength = header_read::real(header, Keyword::fitting(format_args!("RESTWAV{alt}")))?;
        SpectralRest::new(frequency, wavelength)
    }
}

#[cfg(test)]
mod tests;
