//! [`Bounds`]: a header's observation bounds, durations and time errors.

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::time_coordinates::coordinate::Coordinate;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::frame::Frame;
use crate::time_coordinates::mjd::Mjd;

/// The global bound, duration and error time keywords (§9.4, §9.5, §9.7) of a header
/// in its [`Frame`]. Each accessor reads its own keywords, so a bad one fails alone.
/// The bounds are absolute times in the frame's `TIMESYS`, and the durations and errors
/// are in its `TIMEUNIT`.
#[derive(Debug, Clone, Copy)]
pub struct Bounds<'a> {
    header: &'a Header,
    frame: &'a Frame,
}

impl<'a> Bounds<'a> {
    /// The bounds of `header`, whose time frame is `frame`.
    pub const fn new(header: &'a Header, frame: &'a Frame) -> Self {
        Self { header, frame }
    }

    /// The observation start: `MJD-BEG`, else `DATE-BEG`.
    pub fn start(&self) -> Result<Option<Coordinate>> {
        self.mjd_or_date(keywords::MJD_BEG, keywords::DATE_BEG)
    }

    /// The observation end: `MJD-END`, else `DATE-END`.
    pub fn end(&self) -> Result<Option<Coordinate>> {
        self.mjd_or_date(keywords::MJD_END, keywords::DATE_END)
    }

    /// The observation midpoint: `MJD-AVG`, else `DATE-AVG` (§9.5, Table 35).
    pub fn mid(&self) -> Result<Option<Coordinate>> {
        self.mjd_or_date(keywords::MJD_AVG, keywords::DATE_AVG)
    }

    /// `XPOSURE`: the effective exposure time.
    pub fn exposure(&self) -> Result<Option<f64>> {
        self.header.get_real(keywords::XPOSURE)
    }

    /// `TELAPSE`: the total elapsed time.
    pub fn elapsed(&self) -> Result<Option<f64>> {
        self.header.get_real(keywords::TELAPSE)
    }

    /// `TIMEDEL`: the time resolution, or the bin width.
    pub fn resolution(&self) -> Result<Option<f64>> {
        self.header.get_real(keywords::TIMEDEL)
    }

    /// `TIMEPIXR`: the position of a time stamp in its bin, 0 to 1, 0.5 by default.
    /// Errors with [`FitsError::KeywordOutOfRange`] for a value outside `[0, 1]`.
    pub fn pixel_position(&self) -> Result<f64> {
        let position = self.header.get_real(keywords::TIMEPIXR)?.unwrap_or(0.5);
        if !(0.0..=1.0).contains(&position) {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::TIMEPIXR,
            });
        }
        Ok(position)
    }

    /// `TIMSYER`: the systematic time error.
    pub fn systematic_error(&self) -> Result<Option<f64>> {
        self.header.get_real(keywords::TIMSYER)
    }

    /// `TIMRDER`: the random time error.
    pub fn random_error(&self) -> Result<Option<f64>> {
        self.header.get_real(keywords::TIMRDER)
    }

    fn mjd_or_date(&self, mjd: Keyword, date: Keyword) -> Result<Option<Coordinate>> {
        let mjd = if let Some(value) = self.header.get_real(mjd)? {
            Mjd::from_days(value, 0.0).ok_or(FitsError::KeywordOutOfRange { keyword: mjd })?
        } else if let Some(datetime) = Datetime::from_keyword(self.header, date)? {
            datetime.to_mjd()?
        } else {
            return Ok(None);
        };
        Ok(Some(Coordinate {
            mjd,
            scale: self.frame.scale.clone(),
        }))
    }
}

#[cfg(test)]
mod tests;
