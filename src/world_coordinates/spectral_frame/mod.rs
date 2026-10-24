//! Reference-frame and rest-value metadata for a spectral WCS axis (§8.4).

use crate::alternate::Alternate;
use crate::error::FitsError;
use crate::error::Result;
use crate::error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::world_coordinates::axis_transform::spectral_rest::SpectralRest;
use crate::world_coordinates::header_read;

/// A standard spectral reference system from FITS Table 27.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
}

/// Resolved reference-frame and rest-value metadata for a spectral WCS axis.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralFrame {
    /// `SPECSYSa`; `None` when the coordinate frame was not declared.
    pub coordinate: Option<SpectralReferenceFrame>,
    /// `SSYSOBSa`, including its `TOPOCENT` default.
    pub observer: SpectralReferenceFrame,
    /// `RESTFRQa` and `RESTWAVa`, each finite and positive when given.
    pub rest: SpectralRest,
}

impl SpectralReferenceFrame {
    fn parse(keyword: Keyword, value: &str) -> Result<Self> {
        match value.trim() {
            "TOPOCENT" => Ok(Self::Topocentric),
            "GEOCENTR" => Ok(Self::Geocentric),
            "BARYCENT" => Ok(Self::Barycentric),
            "HELIOCEN" => Ok(Self::Heliocentric),
            "LSRK" => Ok(Self::LsrKinematic),
            "LSRD" => Ok(Self::LsrDynamic),
            "GALACTOC" => Ok(Self::Galactocentric),
            "LOCALGRP" => Ok(Self::LocalGroup),
            "CMBDIPOL" => Ok(Self::CmbDipole),
            "SOURCE" => Ok(Self::Source),
            value => Err(FitsError::InvalidWcs {
                defect: WcsDefect::UnknownSpectralFrame {
                    keyword,
                    value: value.to_string(),
                },
            }),
        }
    }
}

impl SpectralFrame {
    /// Resolve `SPECSYSa`/`SSYSOBSa` and the `RESTFRQa`/`RESTWAVa` rest values from
    /// an image header.
    pub(super) fn from_header(header: &Header, alt: Alternate) -> Result<Self> {
        let rest = Self::rest(header, alt)?;
        let coordinate = header_read::text(header, Keyword::fitting(format_args!("SPECSYS{alt}")))?
            .map(|value| SpectralReferenceFrame::parse(keywords::SPECSYS, value))
            .transpose()?;
        let observer = header_read::text(header, Keyword::fitting(format_args!("SSYSOBS{alt}")))?
            .map(|value| SpectralReferenceFrame::parse(keywords::SSYSOBS, value))
            .transpose()?
            .unwrap_or(SpectralReferenceFrame::Topocentric);
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
