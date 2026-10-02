//! [`FitsTime`]: a header's time coordinate frame.

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::key;
use crate::time_coordinates::datetime::Datetime;
use crate::time_coordinates::time_axis_kind::TimeAxisKind;
use crate::time_coordinates::time_coordinate::TimeCoordinate;
use crate::time_coordinates::time_reference_position::TimeReferencePosition;
use crate::time_coordinates::time_scale::{TimeScale, TimeScaleKind};
use crate::time_coordinates::{MJD0, SEC_PER_DAY};
use crate::unit;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::ctype::Ctype;

/// A header's time coordinate frame (§9): the reference epoch, scale, unit, and
/// the resolved global time keywords.
#[derive(Debug, Clone)]
pub struct FitsTime {
    /// `TIMESYS` time scale (default `UTC`).
    pub scale: TimeScale,
    /// Reference epoch as MJD (from `MJDREF`/`MJDREFI`+`MJDREFF`, `JDREF*`, or
    /// `DATEREF`); `0.0` if none is given.
    pub mjdref: f64,
    /// `TIMEUNIT` (default `'s'`).
    pub timeunit: String,
    /// `TIMEOFFS` (§9.4.1): a uniform additive clock correction in `TIMEUNIT`,
    /// equivalent to shifting the reference time. Default `0.0`.
    pub timeoffs: f64,
    /// `TREFPOS`/`TRPOSn`, including the standard `TOPOCENTER` default.
    pub trefpos: TimeReferencePosition,
}

impl FitsTime {
    /// The time frame `header` declares.
    pub fn from_header(header: &Header) -> Result<FitsTime> {
        FitsTime::parse(header, None)
    }

    /// The time frame for a binary-table `column` (1-based): `TRPOSn` overrides the
    /// global `TREFPOS`, and both default to `TOPOCENTER`.
    pub fn for_column(header: &Header, column: usize) -> Result<FitsTime> {
        FitsTime::parse(header, Some(column))
    }

    fn parse(header: &Header, column: Option<usize>) -> Result<FitsTime> {
        if column == Some(0) {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        let scale = TimeScale::declared(header)?;
        let timeunit = header.get_text("TIMEUNIT")?.unwrap_or("s").to_string();
        let column_trefpos = match column {
            Some(column) => header.get_text(key!("TRPOS{column}").as_str())?,
            None => None,
        };
        let trefpos = match column_trefpos {
            Some(value) => TimeReferencePosition::parse(value),
            None => {
                TimeReferencePosition::parse(header.get_text("TREFPOS")?.unwrap_or("TOPOCENTER"))
            }
        };
        let mjdref = reference_mjd(header, &scale)?;
        let fits_time = FitsTime {
            scale,
            mjdref,
            timeunit,
            timeoffs: header.get_real("TIMEOFFS")?.unwrap_or(0.0),
            trefpos,
        };
        Ok(fits_time)
    }

    /// `TIMEUNIT` expressed in seconds. Standard SI prefixes are accepted and
    /// tropical/Besselian years are evaluated at `MJDREF`.
    pub fn unit_seconds(&self) -> Result<f64> {
        time_unit_seconds(&self.timeunit, self.mjdref, &self.scale)
    }

    /// Resolve a time value measured *relative* to `MJDREF` (e.g. `TSTART`,
    /// `TSTOP`), in `TIMEUNIT`, to an absolute MJD in the frame's own scale. The
    /// `TIMEOFFS` clock correction (§9.4.1) is added before scaling.
    pub fn relative_to_mjd(&self, value: f64) -> Result<f64> {
        self.relative_to_mjd_in(value, &self.timeunit, &self.scale)
    }

    fn relative_to_mjd_in(&self, value: f64, unit: &str, scale: &TimeScale) -> Result<f64> {
        Ok(self.mjdref
            + (value + self.timeoffs) * time_unit_seconds(unit, self.mjdref, scale)? / SEC_PER_DAY)
    }

    /// Evaluate a 1-based time `axis` through its complete WCS row and coordinate
    /// algorithm, then convert it to MJD. `pixel` must contain one coordinate per
    /// WCS axis.
    pub fn time_axis_mjd(
        &self,
        wcs: &Wcs,
        axis: usize,
        pixel: &[f64],
    ) -> Result<Option<TimeCoordinate>> {
        let zero_based = axis
            .checked_sub(1)
            .ok_or(FitsError::OneBasedIndexRequired { kind: "WCS axis" })?;
        let metadata = wcs
            .view()
            .axes
            .get(zero_based)
            .ok_or(FitsError::IndexOutOfBounds {
                indexed: Indexed::WcsAxis,
                index: axis,
                len: wcs.view().axes.len(),
            })?;
        if TimeAxisKind::from_ctype(&metadata.ctype) != Some(TimeAxisKind::Time) {
            return Ok(None);
        }
        let head = Ctype::parse(&metadata.ctype).head;
        let scale = if head.eq_ignore_ascii_case("TIME") {
            self.scale.clone()
        } else {
            head.parse::<TimeScale>()?
        };
        let world = wcs.axis_world(zero_based, pixel)?;
        let unit = if world.cunit.trim().is_empty() {
            &self.timeunit
        } else {
            world.cunit
        };
        Ok(Some(TimeCoordinate {
            mjd: self.relative_to_mjd_in(world.value, unit, &scale)?,
            scale,
        }))
    }
}

/// Seconds per `unit`: the [`unit::TIME`] units, or the tropical (`ta`) and Besselian
/// (`Ba`) years, whose length depends on the epoch and takes no prefix.
fn time_unit_seconds(unit: &str, reference_mjd: f64, scale: &TimeScale) -> Result<f64> {
    let invalid = || FitsError::InvalidUnit {
        unit: unit.to_string(),
        expected: "a time unit",
    };
    if let Some(seconds) = unit::resolve(unit, unit::TIME) {
        return Ok(seconds);
    }
    let scaled = unit::split_numeric_multiplier(unit).ok_or_else(invalid)?;
    let days = match scaled.base {
        "ta" => tropical_year_days(reference_mjd, scale)?,
        "Ba" => besselian_year_days(reference_mjd, scale)?,
        _ => return Err(invalid()),
    };
    Ok(scaled.factor * days * SEC_PER_DAY)
}

fn tropical_year_days(reference_mjd: f64, scale: &TimeScale) -> Result<f64> {
    if scale.kind() != Some(TimeScaleKind::Tdb) {
        return Err(FitsError::ExternalTimeDataRequired {
            operation: "evaluate a tropical year outside the TDB frame",
        });
    }
    let centuries = (reference_mjd + MJD0 - 2_451_545.0) / 36_525.0;
    Ok(
        365.242_190_402_112_4 - 0.000_006_152_513_49 * centuries - 6.0921e-10 * centuries.powi(2)
            + 2.6525e-10 * centuries.powi(3),
    )
}

fn besselian_year_days(reference_mjd: f64, scale: &TimeScale) -> Result<f64> {
    if scale.kind() != Some(TimeScaleKind::Tt) {
        return Err(FitsError::ExternalTimeDataRequired {
            operation: "evaluate a Besselian year outside the TT/ET frame",
        });
    }
    let centuries = (reference_mjd + MJD0 - 2_415_020.0) / 36_525.0;
    Ok(365.242_198_781_7 - 0.000_007_854_23 * centuries)
}

/// The reference epoch as MJD: `MJDREF` (or `MJDREFI`+`MJDREFF`), else `JDREF`
/// (or `JDREFI`+`JDREFF`), else `DATEREF`, else `0.0`.
fn reference_mjd(header: &Header, scale: &TimeScale) -> Result<f64> {
    if let Some(mjd) = resolve_split_ref(header, "MJDREF", "MJDREFI", "MJDREFF")? {
        return Ok(mjd);
    }
    if let Some(jd) = resolve_split_ref(header, "JDREF", "JDREFI", "JDREFF")? {
        return Ok(jd - MJD0);
    }
    let Some(value) = header.get_text("DATEREF")? else {
        return Ok(0.0);
    };
    Datetime::parse(value)?.to_mjd(scale)
}

/// Resolve a reference epoch from its single (`MJDREF`) and split-precision
/// (`MJDREFI`+`MJDREFF`) keywords. Per §9.2.2 a *full* integer+fractional split
/// takes precedence over the single value; otherwise the single value is used,
/// falling back to a lone split part.
fn resolve_split_ref(header: &Header, single: &str, int: &str, frac: &str) -> Result<Option<f64>> {
    let i = header.get_real(int)?;
    let f = header.get_real(frac)?;
    Ok(match (i, f) {
        (Some(i), Some(f)) => Some(i + f),
        _ => header.get_real(single)?.or_else(|| match (i, f) {
            (None, None) => None,
            _ => Some(i.unwrap_or(0.0) + f.unwrap_or(0.0)),
        }),
    })
}
