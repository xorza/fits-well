//! [`Bounds`]: a header's observation bounds, durations and time errors.

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::scale::Scale;

/// The global bound / duration / error time keywords (§9.4, §9.5, §9.7). Start
/// and end are absolute MJD in the declared `TIMESYS` scale; the rest are in
/// `TIMEUNIT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Bounds {
    /// Observation start: `MJD-BEG`, else `DATE-BEG` → MJD.
    pub beg_mjd: Option<f64>,
    /// Observation end: `MJD-END`, else `DATE-END` → MJD.
    pub end_mjd: Option<f64>,
    /// Observation midpoint: `MJD-AVG`, else `DATE-AVG` → MJD (§9.5, Table 35).
    pub avg_mjd: Option<f64>,
    /// `XPOSURE` — effective exposure time.
    pub xposure: Option<f64>,
    /// `TELAPSE` — total elapsed time.
    pub telapse: Option<f64>,
    /// `TIMEDEL` — time resolution / bin width.
    pub timedel: Option<f64>,
    /// `TIMEPIXR` — pixel position within a bin (0–1, default 0.5).
    pub timepixr: f64,
    /// `TIMSYER` — systematic time error.
    pub timsyer: Option<f64>,
    /// `TIMRDER` — random time error.
    pub timrder: Option<f64>,
}

impl Bounds {
    /// The bounds `header` declares. Errors when `TIMEPIXR` lies outside `[0, 1]`.
    pub fn from_header(header: &Header) -> Result<Self> {
        let mjd_or_date = |mjd: Keyword, date: Keyword| -> Result<Option<f64>> {
            if let Some(value) = header.get_real(mjd)? {
                return Ok(Some(value));
            }
            let Some(value) = header.get_text(date)? else {
                return Ok(None);
            };
            value
                .parse::<Datetime>()?
                .to_mjd(&Scale::declared(header)?)
                .map(Some)
        };
        let timepixr = header.get_real(keywords::TIMEPIXR)?.unwrap_or(0.5);
        if !(0.0..=1.0).contains(&timepixr) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TIMEPIXR,
            });
        }
        Ok(Self {
            beg_mjd: mjd_or_date(keywords::MJD_BEG, keywords::DATE_BEG)?,
            end_mjd: mjd_or_date(keywords::MJD_END, keywords::DATE_END)?,
            avg_mjd: mjd_or_date(keywords::MJD_AVG, keywords::DATE_AVG)?,
            xposure: header.get_real(keywords::XPOSURE)?,
            telapse: header.get_real(keywords::TELAPSE)?,
            timedel: header.get_real(keywords::TIMEDEL)?,
            timepixr,
            timsyer: header.get_real(keywords::TIMSYER)?,
            timrder: header.get_real(keywords::TIMRDER)?,
        })
    }
}
