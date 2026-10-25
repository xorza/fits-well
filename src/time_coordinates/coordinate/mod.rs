//! [`Coordinate`]: an absolute time with its scale.

use crate::fits_error::FitsError;
use crate::fits_error::KeywordDefect;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::mjd::{Expansion, Mjd};
use crate::time_coordinates::scale::{Scale, ScaleKind};

/// An absolute time coordinate represented by MJD and its declared scale.
#[derive(Debug, Clone, PartialEq)]
pub struct Coordinate {
    /// Modified Julian Date in [`Coordinate::scale`].
    pub mjd: Mjd,
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
    /// with 365.242198781-day years (Lieske 1979, as ERFA's `epb2jd`). `None` for a
    /// year outside [`Mjd`]'s range.
    pub(crate) fn to_mjd(self) -> Option<Mjd> {
        match self {
            Self::Julian(y) => {
                Mjd::new(51_544, 0.5).plus(Expansion::two_product(y - 2000.0, 365.25))
            }
            Self::Besselian(y) => {
                Mjd::new(15_019, 0.813_52).plus(Expansion::two_product(y - 1900.0, 365.242_198_781))
            }
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
                mjd: Mjd::from_days(mjd, 0.0).ok_or(FitsError::InvalidKeywordValue {
                    keyword: keywords::MJD_OBS,
                    defect: KeywordDefect::Range,
                })?,
                scale: Scale::declared(header)?,
            }));
        }
        if let Some(datetime) = Datetime::from_keyword(header, keywords::DATE_OBS)? {
            return Ok(Some(Self {
                mjd: datetime.to_mjd()?,
                scale: Scale::declared(header)?,
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
                mjd: Epoch::Julian(j)
                    .to_mjd()
                    .ok_or(FitsError::InvalidKeywordValue {
                        keyword: keywords::JEPOCH,
                        defect: KeywordDefect::Range,
                    })?,
                scale: Scale::known(ScaleKind::Tdb),
            }));
        }
        let Some(b) = header.get_real(keywords::BEPOCH)? else {
            return Ok(None);
        };
        Ok(Some(Self {
            mjd: Epoch::Besselian(b)
                .to_mjd()
                .ok_or(FitsError::InvalidKeywordValue {
                    keyword: keywords::BEPOCH,
                    defect: KeywordDefect::Range,
                })?,
            scale: Scale::known(ScaleKind::Tt),
        }))
    }
}

#[cfg(test)]
mod tests;
