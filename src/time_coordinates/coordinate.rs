//! [`Coordinate`]: an absolute time with its scale.

use crate::error::Result;
use crate::header_model::{Header, keywords};
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::scale::{Scale, ScaleKind};

/// An absolute time coordinate represented by MJD and its declared scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Coordinate {
    /// Modified Julian Date in [`Coordinate::scale`].
    pub mjd: f64,
    /// Time scale associated with the MJD.
    pub scale: Scale,
}

/// A reference epoch from the numeric `JEPOCH` or `BEPOCH` keyword.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Epoch {
    Julian(f64),
    Besselian(f64),
}

impl Epoch {
    /// J2000.0 is JD 2451545.0 with 365.25-day years, B1900.0 is JD 2415020.31352
    /// with 365.242198781-day years (Lieske 1979, as ERFA's `epb2jd`), each written
    /// as an MJD so the sum rounds at the MJD's precision.
    pub(crate) fn to_mjd(self) -> f64 {
        match self {
            Self::Julian(y) => 51_544.5 + (y - 2000.0) * 365.25,
            Self::Besselian(y) => 15_019.813_52 + (y - 1900.0) * 365.242_198_781,
        }
    }
}

impl Coordinate {
    /// The observation time: `MJD-OBS`, else `DATE-OBS`, in the declared `TIMESYS`
    /// scale; else the [`Coordinate::epoch`] in its implied scale (§9.5).
    /// `None` when the header gives none of them.
    pub fn observation(header: &Header) -> Result<Option<Self>> {
        if let Some(mjd) = header.get_real(keywords::MJD_OBS)? {
            return Ok(Some(Self {
                mjd,
                scale: Scale::declared(header)?,
            }));
        }
        if let Some(value) = header.get_text(keywords::DATE_OBS)? {
            let scale = Scale::declared(header)?;
            return Ok(Some(Self {
                mjd: value.parse::<Datetime>()?.to_mjd(&scale)?,
                scale,
            }));
        }
        Self::epoch(header)
    }

    /// The Julian (`JEPOCH`, implied scale TDB) or Besselian (`BEPOCH`, implied
    /// scale ET ≈ TT) epoch keyword, if present (§9.1.2, §9.5). `JEPOCH` wins if
    /// both appear.
    pub fn epoch(header: &Header) -> Result<Option<Self>> {
        if let Some(j) = header.get_real(keywords::JEPOCH)? {
            return Ok(Some(Self {
                mjd: Epoch::Julian(j).to_mjd(),
                scale: Scale::known(ScaleKind::Tdb),
            }));
        }
        let Some(b) = header.get_real(keywords::BEPOCH)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            mjd: Epoch::Besselian(b).to_mjd(),
            scale: Scale::known(ScaleKind::Tt),
        }))
    }
}
