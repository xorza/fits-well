//! [`TimeCoordinate`]: an absolute time with its scale.

use crate::error::Result;
use crate::header_model::Header;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::time_scale::{TimeScale, TimeScaleKind};

/// An absolute time coordinate represented by MJD and its declared scale.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeCoordinate {
    /// Modified Julian Date in [`TimeCoordinate::scale`].
    pub mjd: f64,
    /// Time scale associated with the MJD.
    pub scale: TimeScale,
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
            Epoch::Julian(y) => 51_544.5 + (y - 2000.0) * 365.25,
            Epoch::Besselian(y) => 15_019.813_52 + (y - 1900.0) * 365.242_198_781,
        }
    }
}

impl TimeCoordinate {
    /// The observation time: `MJD-OBS`, else `DATE-OBS`, in the declared `TIMESYS`
    /// scale; else the [`TimeCoordinate::epoch`] in its implied scale (§9.5).
    /// `None` when the header gives none of them.
    pub fn observation(header: &Header) -> Result<Option<TimeCoordinate>> {
        if let Some(mjd) = header.get_real("MJD-OBS")? {
            return Ok(Some(TimeCoordinate {
                mjd,
                scale: TimeScale::declared(header)?,
            }));
        }
        if let Some(value) = header.get_text("DATE-OBS")? {
            let scale = TimeScale::declared(header)?;
            return Ok(Some(TimeCoordinate {
                mjd: Datetime::parse(value)?.to_mjd(&scale)?,
                scale,
            }));
        }
        TimeCoordinate::epoch(header)
    }

    /// The Julian (`JEPOCH`, implied scale TDB) or Besselian (`BEPOCH`, implied
    /// scale ET ≈ TT) epoch keyword, if present (§9.1.2, §9.5). `JEPOCH` wins if
    /// both appear.
    pub fn epoch(header: &Header) -> Result<Option<TimeCoordinate>> {
        if let Some(j) = header.get_real("JEPOCH")? {
            return Ok(Some(TimeCoordinate {
                mjd: Epoch::Julian(j).to_mjd(),
                scale: TimeScale::known(TimeScaleKind::Tdb),
            }));
        }
        let Some(b) = header.get_real("BEPOCH")? else {
            return Ok(None);
        };
        Ok(Some(TimeCoordinate {
            mjd: Epoch::Besselian(b).to_mjd(),
            scale: TimeScale::known(TimeScaleKind::Tt),
        }))
    }
}
