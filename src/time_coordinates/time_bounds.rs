//! [`TimeBounds`]: a header's observation bounds, durations and time errors.

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::time_scale::TimeScale;

/// The global bound / duration / error time keywords (§9.4, §9.5, §9.7). Start
/// and end are absolute MJD in the declared `TIMESYS` scale; the rest are in
/// `TIMEUNIT`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TimeBounds {
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

impl TimeBounds {
    /// The bounds `header` declares. Errors when `TIMEPIXR` lies outside `[0, 1]`.
    pub fn from_header(header: &Header) -> Result<TimeBounds> {
        let mjd_or_date = |mjd: &str, date: &str| -> Result<Option<f64>> {
            if let Some(value) = header.get_real(mjd)? {
                return Ok(Some(value));
            }
            let Some(value) = header.get_text(date)? else {
                return Ok(None);
            };
            Datetime::parse(value)?
                .to_mjd(&TimeScale::declared(header)?)
                .map(Some)
        };
        let timepixr = header.get_real("TIMEPIXR")?.unwrap_or(0.5);
        if !(0.0..=1.0).contains(&timepixr) {
            return Err(FitsError::KeywordOutOfRange { name: "TIMEPIXR" });
        }
        Ok(TimeBounds {
            beg_mjd: mjd_or_date("MJD-BEG", "DATE-BEG")?,
            end_mjd: mjd_or_date("MJD-END", "DATE-END")?,
            avg_mjd: mjd_or_date("MJD-AVG", "DATE-AVG")?,
            xposure: header.get_real("XPOSURE")?,
            telapse: header.get_real("TELAPSE")?,
            timedel: header.get_real("TIMEDEL")?,
            timepixr,
            timsyer: header.get_real("TIMSYER")?,
            timrder: header.get_real("TIMRDER")?,
        })
    }
}
