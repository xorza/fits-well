//! Reference-frame and rest-value metadata for a spectral WCS axis (§8.4).

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::wcs::axis::spectral_rest::SpectralRest;

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
    /// `RESTFRQa` in Hz.
    pub rest_frequency_hz: Option<f64>,
    /// `RESTWAVa` in metres.
    pub rest_wavelength_m: Option<f64>,
}

impl SpectralReferenceFrame {
    fn parse(keyword: &str, value: &str) -> Result<SpectralReferenceFrame> {
        match value.trim() {
            "TOPOCENT" => Ok(SpectralReferenceFrame::Topocentric),
            "GEOCENTR" => Ok(SpectralReferenceFrame::Geocentric),
            "BARYCENT" => Ok(SpectralReferenceFrame::Barycentric),
            "HELIOCEN" => Ok(SpectralReferenceFrame::Heliocentric),
            "LSRK" => Ok(SpectralReferenceFrame::LsrKinematic),
            "LSRD" => Ok(SpectralReferenceFrame::LsrDynamic),
            "GALACTOC" => Ok(SpectralReferenceFrame::Galactocentric),
            "LOCALGRP" => Ok(SpectralReferenceFrame::LocalGroup),
            "CMBDIPOL" => Ok(SpectralReferenceFrame::CmbDipole),
            "SOURCE" => Ok(SpectralReferenceFrame::Source),
            value => Err(FitsError::InvalidValue {
                card: format!("{keyword} {value:?} is not a standard spectral reference frame"),
            }),
        }
    }
}

impl SpectralFrame {
    /// Resolve `SPECSYSa`/`SSYSOBSa` and the `RESTFRQa`/`RESTWAVa` rest values from
    /// an image header.
    pub(super) fn from_header(
        header: &Header,
        alt: Option<char>,
        suffix: &str,
    ) -> Result<SpectralFrame> {
        let rest = SpectralFrame::rest(header, alt, suffix)?;
        let coordinate = header
            .get_text(key!("SPECSYS{suffix}").as_str())?
            .map(|value| SpectralReferenceFrame::parse("SPECSYS", value))
            .transpose()?;
        let observer = header
            .get_text(key!("SSYSOBS{suffix}").as_str())?
            .map(|value| SpectralReferenceFrame::parse("SSYSOBS", value))
            .transpose()?
            .unwrap_or(SpectralReferenceFrame::Topocentric);
        Ok(SpectralFrame {
            coordinate,
            observer,
            rest_frequency_hz: rest.frequency,
            rest_wavelength_m: rest.wavelength,
        })
    }

    fn rest(header: &Header, alt: Option<char>, suffix: &str) -> Result<SpectralRest> {
        let mut frequency = header.get_real(key!("RESTFRQ{suffix}").as_str())?;
        if frequency.is_none() && alt.is_none() {
            frequency = header.get_real("RESTFREQ")?;
        }
        let wavelength = header.get_real(key!("RESTWAV{suffix}").as_str())?;
        SpectralRest::new(frequency, wavelength)
    }
}
