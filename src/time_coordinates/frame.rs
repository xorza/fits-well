//! [`Frame`]: a header's time coordinate frame.

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::time_coordinates::coordinate::Coordinate;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::reference_position::ReferencePosition;
use crate::time_coordinates::scale::Scale;
use crate::time_coordinates::time_axis_kind::TimeAxisKind;
use crate::time_coordinates::unit::Unit;
use crate::time_coordinates::{MJD0, SEC_PER_DAY};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::ctype::Ctype;

/// A header's time coordinate frame (§9): the reference epoch, scale, unit, and
/// the resolved global time keywords.
#[derive(Debug, Clone)]
pub struct Frame {
    /// `TIMESYS` time scale (default `UTC`).
    pub scale: Scale,
    /// Reference epoch as MJD (from `MJDREF`/`MJDREFI`+`MJDREFF`, `JDREF*`, or
    /// `DATEREF`); `0.0` if none is given.
    pub reference_mjd: f64,
    /// `TIMEUNIT` (default `'s'`).
    pub unit: Unit,
    /// `TIMEOFFS` (§9.4.1): a uniform additive clock correction in `TIMEUNIT`,
    /// equivalent to shifting the reference time. Default `0.0`.
    pub clock_offset: f64,
    /// `TREFPOS`/`TRPOSn`, including the standard `TOPOCENTER` default.
    pub reference_position: ReferencePosition,
}

impl Frame {
    /// The time frame `header` declares.
    pub fn from_header(header: &Header) -> Result<Self> {
        Self::parse(header, None)
    }

    /// The time frame for a binary-table `column` (1-based): `TRPOSn` overrides the
    /// global `TREFPOS`, and both default to `TOPOCENTER`.
    pub fn from_column(header: &Header, column: usize) -> Result<Self> {
        Self::parse(header, Some(column))
    }

    fn parse(header: &Header, column: Option<usize>) -> Result<Self> {
        if column == Some(0) {
            return Err(FitsError::OneBasedIndexRequired {
                indexed: Indexed::Column,
            });
        }
        let scale = Scale::declared(header)?;
        let unit = header
            .get_text(keywords::TIMEUNIT)?
            .unwrap_or("s")
            .parse::<Unit>()?;
        let column_trefpos = match column {
            Some(column) => header.get_text(keywords::TRPOS.indexed(column)?)?,
            None => None,
        };
        let reference_position = match column_trefpos {
            Some(value) => ReferencePosition::parse(value),
            None => ReferencePosition::parse(
                header.get_text(keywords::TREFPOS)?.unwrap_or("TOPOCENTER"),
            ),
        };
        let reference_mjd = reference_mjd(header, &scale)?;
        let fits_time = Self {
            scale,
            reference_mjd,
            unit,
            clock_offset: header.get_real(keywords::TIMEOFFS)?.unwrap_or(0.0),
            reference_position,
        };
        Ok(fits_time)
    }

    /// `TIMEUNIT` expressed in seconds. Standard SI prefixes are accepted and
    /// tropical/Besselian years are evaluated at `MJDREF`.
    pub fn unit_seconds(&self) -> Result<f64> {
        self.unit.seconds(self.reference_mjd, &self.scale)
    }

    /// Resolve a time value measured *relative* to `MJDREF` (e.g. `TSTART`,
    /// `TSTOP`), in `TIMEUNIT`, to an absolute MJD in the frame's own scale. The
    /// `TIMEOFFS` clock correction (§9.4.1) is added before scaling.
    pub fn relative_to_mjd(&self, value: f64) -> Result<f64> {
        self.relative_to_mjd_in(value, self.unit, &self.scale)
    }

    fn relative_to_mjd_in(&self, value: f64, unit: Unit, scale: &Scale) -> Result<f64> {
        Ok(self.reference_mjd
            + (value + self.clock_offset) * unit.seconds(self.reference_mjd, scale)? / SEC_PER_DAY)
    }

    /// Evaluate a 1-based time `axis` through its complete WCS row and coordinate
    /// algorithm, then convert it to MJD. `pixel` must contain one coordinate per
    /// WCS axis.
    pub fn time_axis_mjd(&self, wcs: &Wcs, axis: usize, pixel: &[f64]) -> Result<Coordinate> {
        let zero_based = axis
            .checked_sub(1)
            .ok_or(FitsError::OneBasedIndexRequired {
                indexed: Indexed::WcsAxis,
            })?;
        let metadata =
            wcs.metadata()
                .axes
                .get(zero_based)
                .ok_or_else(|| FitsError::IndexOutOfBounds {
                    indexed: Indexed::WcsAxis,
                    index: axis,
                    len: wcs.metadata().axes.len(),
                })?;
        if TimeAxisKind::from_ctype(&metadata.ctype) != Some(TimeAxisKind::Time) {
            return Err(FitsError::NotATimeAxis { axis });
        }
        let head = Ctype::parse(&metadata.ctype).head;
        let scale = if head.eq_ignore_ascii_case("TIME") {
            self.scale.clone()
        } else {
            head.parse::<Scale>()?
        };
        let world = wcs.axis_world(zero_based, pixel)?;
        let unit = if world.cunit.trim().is_empty() {
            self.unit
        } else {
            world.cunit.parse::<Unit>()?
        };
        Ok(Coordinate {
            mjd: self.relative_to_mjd_in(world.value, unit, &scale)?,
            scale,
        })
    }
}

/// The reference epoch as MJD: `MJDREF` (or `MJDREFI`+`MJDREFF`), else `JDREF`
/// (or `JDREFI`+`JDREFF`), else `DATEREF`, else `0.0`.
fn reference_mjd(header: &Header, scale: &Scale) -> Result<f64> {
    if let Some(mjd) = resolve_split_ref(
        header,
        keywords::MJDREF,
        keywords::MJDREFI,
        keywords::MJDREFF,
        0.0,
    )? {
        return Ok(mjd);
    }
    if let Some(mjd) = resolve_split_ref(
        header,
        keywords::JDREF,
        keywords::JDREFI,
        keywords::JDREFF,
        MJD0,
    )? {
        return Ok(mjd);
    }
    let Some(value) = header.get_text(keywords::DATEREF)? else {
        return Ok(0.0);
    };
    value.parse::<Datetime>()?.to_mjd(scale)
}

/// Resolve a reference epoch from its single (`MJDREF`) and split-precision
/// (`MJDREFI`+`MJDREFF`) keywords. Per §9.2.2 a *full* integer+fractional split
/// takes precedence over the single value; otherwise the single value is used,
/// falling back to a lone split part. `offset` is taken from the integer part
/// before the fraction is added, so a JD split becomes an MJD that rounds at the
/// MJD's precision, not the JD's.
fn resolve_split_ref(
    header: &Header,
    single: Keyword,
    int: Keyword,
    frac: Keyword,
    offset: f64,
) -> Result<Option<f64>> {
    let i = header.get_real(int)?;
    let f = header.get_real(frac)?;
    Ok(match (i, f) {
        (Some(i), Some(f)) => Some((i - offset) + f),
        _ => header
            .get_real(single)?
            .map(|value| value - offset)
            .or_else(|| match (i, f) {
                (None, None) => None,
                _ => Some((i.unwrap_or(0.0) - offset) + f.unwrap_or(0.0)),
            }),
    })
}
