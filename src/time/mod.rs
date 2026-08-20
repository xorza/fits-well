//! Typed time coordinates (§9).
//!
//! Covers FITS ISO-8601 datetimes → Julian Date / MJD (strict year forms and
//! proleptic-Gregorian math), `J`/`B` epochs → JD, declared time-scale metadata,
//! and a [`FitsTime`] view over a header's time keywords
//! (`TIMESYS`, `MJDREF*`/`JDREF*`/`DATEREF`, `TIMEUNIT`, resolved
//! `TREFPOS`/`TRPOSn`, all image/table PHASE forms, and the global
//! `DATE-OBS`/`MJD-OBS`/`TSTART`/… set). Conversion between declared time
//! frames requires external ephemeris and Earth-orientation data and is outside
//! this crate.

pub(crate) mod datetime;
pub(crate) mod phase_axis;
pub(crate) mod time_reference_position;
pub(crate) mod time_scale;

use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::time_impl::datetime::Datetime;
use crate::time_impl::phase_axis::PhaseAxis;
use crate::time_impl::time_reference_position::TimeReferencePosition;
use crate::time_impl::time_scale::{TimeScale, TimeScaleKind};
use crate::unit;
use crate::wcs::Wcs;

/// JD of the MJD zero point (1858-11-17T00:00 UTC).
const MJD0: f64 = 2_400_000.5;
const SEC_PER_DAY: f64 = 86_400.0;

/// A reference epoch from the numeric `JEPOCH` or `BEPOCH` keyword.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Epoch {
    Julian(f64),
    Besselian(f64),
}

impl Epoch {
    fn to_jd(self) -> f64 {
        match self {
            Epoch::Julian(y) => 2_451_545.0 + (y - 2000.0) * 365.25,
            Epoch::Besselian(y) => 2_415_020.313_52 + (y - 1900.0) * 365.242_198_781,
        }
    }

    fn to_mjd(self) -> f64 {
        self.to_jd() - MJD0
    }
}

/// An absolute time coordinate represented by MJD and its declared scale.
#[derive(Debug, Clone, PartialEq)]
pub struct TimeCoordinate {
    /// Modified Julian Date in [`TimeCoordinate::scale`].
    pub mjd: f64,
    /// Time scale associated with the MJD.
    pub scale: TimeScale,
}

/// The global bound / duration / error time keywords (§9.4, §9.5, §9.7), as read
/// by [`Header::time_bounds`](crate::header::Header::time_bounds). Start/end are absolute
/// MJD; the rest are in `TIMEUNIT`.
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
    /// Parse the time frame from a header. The public entry point is
    /// [`Header::time`](crate::header::Header::time), which forwards here.
    pub(crate) fn from_header(header: &Header) -> Result<FitsTime> {
        FitsTime::from_header_for_column(header, None)
    }

    pub(crate) fn from_header_for_column(
        header: &Header,
        column: Option<usize>,
    ) -> Result<FitsTime> {
        if column == Some(0) {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        let scale = declared_time_scale(header)?;
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

    /// The observation MJD from `MJD-OBS`, else `DATE-OBS`, else `None`. Reads only
    /// the header (not the parsed frame). The public entry point is
    /// [`Header::obs_mjd`](crate::header::Header::obs_mjd), which forwards here.
    pub(crate) fn obs_mjd(header: &Header) -> Result<Option<f64>> {
        if let Some(mjd) = header.get_real("MJD-OBS")? {
            return Ok(Some(mjd));
        }
        if let Some(value) = header.get_text("DATE-OBS")? {
            let scale = declared_time_scale(header)?;
            return Datetime::parse(value)?.to_mjd(&scale).map(Some);
        }
        // §9.5: with no DATE-OBS/MJD-OBS, JEPOCH/BEPOCH stand in for the observation time.
        Ok(Self::epoch(header)?.map(|epoch| epoch.mjd))
    }

    /// The Julian (`JEPOCH`, implied scale TDB) or Besselian (`BEPOCH`, implied
    /// scale ET ≈ TT) epoch keyword as a [`TimeCoordinate`], if present (§9.1.2, §9.5).
    /// `JEPOCH` wins if both appear. Reads only the header, so it takes no `self`.
    pub(crate) fn epoch(header: &Header) -> Result<Option<TimeCoordinate>> {
        if let Some(j) = header.get_real("JEPOCH")? {
            return Ok(Some(TimeCoordinate {
                mjd: Epoch::Julian(j).to_mjd(),
                scale: TimeScale::Tdb,
            }));
        }
        let Some(b) = header.get_real("BEPOCH")? else {
            return Ok(None);
        };
        Ok(Some(TimeCoordinate {
            mjd: Epoch::Besselian(b).to_mjd(),
            scale: TimeScale::Tt, // ET ≈ TT
        }))
    }

    /// The global bound / duration / error time keywords (§9.4, §9.5, §9.7). The
    /// start/end are resolved to absolute MJD (`MJD-BEG`/`-END`, else `DATE-BEG`/
    /// `-END`); durations and errors are returned as stored, in `TIMEUNIT`. Reads
    /// only the header, so it takes no `self`.
    pub(crate) fn bounds(header: &Header) -> Result<TimeBounds> {
        let mjd_or_date = |mjd: &str, date: &str| -> Result<Option<f64>> {
            if let Some(value) = header.get_real(mjd)? {
                return Ok(Some(value));
            }
            let Some(value) = header.get_text(date)? else {
                return Ok(None);
            };
            let scale = declared_time_scale(header)?;
            Datetime::parse(value)?.to_mjd(&scale).map(Some)
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
        let head = metadata.ctype.split('-').next().unwrap_or("").trim();
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

    pub(crate) fn phase_axis(
        header: &Header,
        axis: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        FitsTime::phase_axis_from_keywords(header, PhaseAxisKeywords::image(axis, alt))
    }

    pub(crate) fn phase_axis_pixel_list(
        header: &Header,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        FitsTime::phase_axis_from_keywords(header, PhaseAxisKeywords::pixel_list(column, alt))
    }

    pub(crate) fn phase_axis_array_column(
        header: &Header,
        axis: usize,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        if axis == 0 {
            return Err(FitsError::OneBasedIndexRequired { kind: "WCS axis" });
        }
        if column == 0 {
            return Err(FitsError::OneBasedIndexRequired {
                kind: "table column",
            });
        }
        FitsTime::phase_axis_from_keywords(
            header,
            PhaseAxisKeywords::array_column(axis, column, alt),
        )
    }

    fn phase_axis_from_keywords(
        header: &Header,
        keywords: PhaseAxisKeywords,
    ) -> Result<Option<PhaseAxis>> {
        let Some(ctype) = header.get_text(keywords.ctype.as_str())? else {
            return Ok(None);
        };
        if TimeAxisKind::from_ctype(ctype) != Some(TimeAxisKind::Phase) {
            return Ok(None);
        }
        let zero_phase = header
            .get_real(keywords.zero_phase.as_str())?
            .ok_or_else(|| FitsError::InvalidValue {
                card: format!("PHASE axis requires {}", keywords.zero_phase.as_str()),
            })?;
        if !zero_phase.is_finite() {
            return Err(FitsError::InvalidValue {
                card: format!("{} must be finite", keywords.zero_phase.as_str()),
            });
        }
        let period = header.get_real(keywords.period.as_str())?;
        if period.is_some_and(|value| !value.is_finite()) {
            return Err(FitsError::InvalidValue {
                card: format!("{} must be finite", keywords.period.as_str()),
            });
        }
        Ok(Some(PhaseAxis {
            zero_phase,
            period: period.filter(|value| *value != 0.0),
        }))
    }
}

/// A time-related WCS axis type (§9.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TimeAxisKind {
    /// `'TIME'` or a time-scale name — an absolute time axis (→ MJD).
    Time,
    /// `'PHASE'` — phase folded on a period (`CPERIia`, zero `CZPHSia`).
    Phase,
    /// `'TIMELAG'` — a correlation/cross-spectral time lag.
    Timelag,
    /// `'FREQUENCY'` — a frequency axis.
    Frequency,
}

impl TimeAxisKind {
    /// Classify a `CTYPE` as a time-related axis (§9.6), or `None` if it is not one.
    fn from_ctype(ctype: &str) -> Option<TimeAxisKind> {
        let head = ctype.split('-').next().unwrap_or("").trim();
        if head.eq_ignore_ascii_case("TIME") {
            return Some(TimeAxisKind::Time);
        }
        match head {
            "PHASE" => Some(TimeAxisKind::Phase),
            "TIMELAG" => Some(TimeAxisKind::Timelag),
            "FREQUENCY" => Some(TimeAxisKind::Frequency),
            _ => head
                .parse::<TimeScale>()
                .ok()
                .and_then(|scale| scale.kind())
                .map(|_| TimeAxisKind::Time),
        }
    }
}

fn time_unit_seconds(unit: &str, reference_mjd: f64, scale: &TimeScale) -> Result<f64> {
    let scaled = unit::split_numeric_multiplier(unit).ok_or_else(|| FitsError::InvalidValue {
        card: format!("time unit '{}'", unit.trim()),
    })?;
    let unit = scaled.base;
    if let Some(seconds) = base_time_unit_seconds(unit, reference_mjd, scale)? {
        return Ok(scaled.factor * seconds);
    }
    for (prefix, factor) in unit::SI_PREFIXES {
        if let Some(base) = unit.strip_prefix(prefix)
            && let Some(seconds) = base_time_unit_seconds(base, reference_mjd, scale)?
        {
            return Ok(scaled.factor * factor * seconds);
        }
    }
    Err(FitsError::InvalidValue {
        card: format!("time unit '{unit}'"),
    })
}

fn base_time_unit_seconds(
    unit: &str,
    reference_mjd: f64,
    scale: &TimeScale,
) -> Result<Option<f64>> {
    Ok(Some(match unit {
        "s" => 1.0,
        "min" => 60.0,
        "h" => 3600.0,
        "d" => SEC_PER_DAY,
        "a" | "yr" => 365.25 * SEC_PER_DAY,
        "cy" => 36_525.0 * SEC_PER_DAY,
        "ta" => tropical_year_days(reference_mjd, scale)? * SEC_PER_DAY,
        "Ba" => besselian_year_days(reference_mjd, scale)? * SEC_PER_DAY,
        _ => return Ok(None),
    }))
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

fn declared_time_scale(header: &Header) -> Result<TimeScale> {
    header
        .get_text("TIMESYS")?
        .map(str::parse::<TimeScale>)
        .transpose()
        .map(|scale| scale.unwrap_or(TimeScale::Utc))
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

#[derive(Debug)]
struct PhaseAxisKeywords {
    ctype: KeyBuf,
    zero_phase: KeyBuf,
    period: KeyBuf,
}

impl PhaseAxisKeywords {
    fn image(axis: usize, alt: Option<char>) -> PhaseAxisKeywords {
        let suffix = AltSuffix::new(alt);
        PhaseAxisKeywords {
            ctype: key!("CTYPE{axis}{suffix}"),
            zero_phase: key!("CZPHS{axis}{suffix}"),
            period: key!("CPERI{axis}{suffix}"),
        }
    }

    fn pixel_list(column: usize, alt: Option<char>) -> PhaseAxisKeywords {
        match alt {
            Some(alt) => PhaseAxisKeywords {
                ctype: key!("TCTY{column}{alt}"),
                zero_phase: key!("TCZP{column}{alt}"),
                period: key!("TCPR{column}{alt}"),
            },
            None => PhaseAxisKeywords {
                ctype: key!("TCTYP{column}"),
                zero_phase: key!("TCZPH{column}"),
                period: key!("TCPER{column}"),
            },
        }
    }

    fn array_column(axis: usize, column: usize, alt: Option<char>) -> PhaseAxisKeywords {
        match alt {
            Some(alt) => PhaseAxisKeywords {
                ctype: key!("{axis}CTY{column}{alt}"),
                zero_phase: key!("{axis}CZP{column}{alt}"),
                period: key!("{axis}CPR{column}{alt}"),
            },
            None => PhaseAxisKeywords {
                ctype: key!("{axis}CTYP{column}"),
                zero_phase: key!("{axis}CZPH{column}"),
                period: key!("{axis}CPER{column}"),
            },
        }
    }
}

#[cfg(test)]
mod tests;
