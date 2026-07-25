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

use std::str::FromStr;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::KeyBuf;
use crate::keyword::key;
use crate::unit;
use crate::wcs::Wcs;

/// JD of the MJD zero point (1858-11-17T00:00 UTC).
const MJD0: f64 = 2_400_000.5;
const SEC_PER_DAY: f64 = 86_400.0;

/// A calendar datetime (proleptic Gregorian, time-scale agnostic). `second` may
/// reach 60.x; [`Datetime::to_jd`] accepts that label only for UTC.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Datetime {
    pub year: i64,
    pub month: u32,
    pub day: u32,
    pub hour: u32,
    pub minute: u32,
    pub second: f64,
}

impl Datetime {
    /// Parse a FITS ISO-8601 datetime: unsigned `YYYY-MM-DD` or signed
    /// `±YYYYY-MM-DD`, optionally followed by `Thh:mm:ss[.sss…]` (§9.1.1). No
    /// component defaulting; the date is required, the time part optional.
    pub fn parse(s: &str) -> Result<Datetime> {
        let invalid = || FitsError::InvalidValue {
            card: format!("DATE '{s}'"),
        };
        let s = s.trim();
        // §9.1.1: no timezone designator is permitted (`Z` or a numeric offset).
        if s.contains(['Z', 'z']) {
            return Err(invalid());
        }
        let (date, time) = match s.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        // `[±C]CCYY-MM-DD`: the sign selects the five-digit extended form.
        let (sign, year_width, rest) = match date.strip_prefix('-') {
            Some(r) => (-1, 5, r),
            None => match date.strip_prefix('+') {
                Some(r) => (1, 5, r),
                None => (1, 4, date),
            },
        };
        let mut dp = rest.split('-');
        let y_str = dp.next().ok_or_else(invalid)?;
        let m_str = dp.next().ok_or_else(invalid)?;
        let d_str = dp.next().ok_or_else(invalid)?;
        if dp.next().is_some() || y_str.len() != year_width || !all_digits(y_str) {
            return Err(invalid());
        }
        let year = sign * y_str.parse::<i64>().map_err(|_| invalid())?;
        let month = parse_fixed(m_str, 2).ok_or_else(invalid)?;
        let day = parse_fixed(d_str, 2).ok_or_else(invalid)?;
        if !(1..=12).contains(&month) || day < 1 || day > days_in_month(year, month) {
            return Err(invalid());
        }

        let (mut hour, mut minute, mut second) = (0u32, 0u32, 0.0f64);
        if let Some(t) = time {
            let mut tp = t.split(':');
            hour = parse_fixed(tp.next().ok_or_else(invalid)?, 2).ok_or_else(invalid)?;
            minute = parse_fixed(tp.next().ok_or_else(invalid)?, 2).ok_or_else(invalid)?;
            second = parse_seconds(tp.next().ok_or_else(invalid)?).ok_or_else(invalid)?;
            if tp.next().is_some() || hour >= 24 || minute >= 60 || !(0.0..61.0).contains(&second) {
                return Err(invalid());
            }
        }
        Ok(Datetime {
            year,
            month,
            day,
            hour,
            minute,
            second,
        })
    }

    /// Julian Date of this datetime in its declared `scale`.
    ///
    /// UTC leap-second labels remain valid FITS datetimes, but converting one to
    /// a continuous coordinate requires an external leap-second realization.
    pub fn to_jd(&self, scale: &TimeScale) -> Result<f64> {
        self.validate(scale)?;
        if self.second >= 60.0 {
            return Err(FitsError::ExternalTimeDataRequired {
                operation: "convert a UTC leap-second label to Julian Date",
            });
        }
        let day_start = calendar_day_start(self.year, self.month, self.day);
        let elapsed = self.hour as f64 * 3600.0 + self.minute as f64 * 60.0 + self.second;
        Ok(day_start + elapsed / SEC_PER_DAY)
    }

    /// Modified Julian Date (`JD − 2400000.5`).
    pub fn to_mjd(&self, scale: &TimeScale) -> Result<f64> {
        Ok(self.to_jd(scale)? - MJD0)
    }

    fn validate(&self, scale: &TimeScale) -> Result<()> {
        let valid_date = (-99999..=99999).contains(&self.year)
            && (1..=12).contains(&self.month)
            && self.day >= 1
            && self.day <= days_in_month(self.year, self.month);
        let valid_time =
            self.hour < 24 && self.minute < 60 && self.second.is_finite() && self.second >= 0.0;
        let valid_second = if self.second < 60.0 {
            true
        } else {
            valid_date
                && self.second < 61.0
                && scale.is_utc()
                && self.hour == 23
                && self.minute == 59
        };
        if valid_date && valid_time && valid_second {
            return Ok(());
        }
        Err(FitsError::InvalidValue {
            card: format!("datetime {self:?} in {scale:?}"),
        })
    }
}

/// True if `s` is non-empty and all ASCII digits.
fn all_digits(s: &str) -> bool {
    !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit())
}

/// Parse a fixed-width all-digit field (§9.1.1 forbids omitted leading zeros, so
/// the length must be exact).
fn parse_fixed(s: &str, width: usize) -> Option<u32> {
    (s.len() == width && all_digits(s))
        .then(|| s.parse().ok())
        .flatten()
}

/// Parse a `ss[.s…]` seconds field: exactly two integer digits, optional fraction.
fn parse_seconds(s: &str) -> Option<f64> {
    let (int, frac) = s.split_once('.').map_or((s, None), |(i, f)| (i, Some(f)));
    if int.len() != 2 || !all_digits(int) || frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    s.parse().ok()
}

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

/// A recognized FITS time-scale meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeScaleKind {
    Utc,
    Ut1,
    Tai,
    Tt,
    Tcg,
    Tdb,
    Tcb,
    Gps,
}

/// A FITS time-scale declaration (`TIMESYS` / `CTYPEi`).
///
/// Standard scales are normalized to their meaning while retaining an optional
/// realization suffix. Any other nonempty code is preserved as a local scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeScale {
    Utc,
    Ut1,
    Tai,
    Tt,
    Tcg,
    Tdb,
    Tcb,
    Gps,
    Realized {
        kind: TimeScaleKind,
        realization: String,
    },
    Local(String),
}

impl TimeScale {
    fn kind(&self) -> Option<TimeScaleKind> {
        match self {
            TimeScale::Utc => Some(TimeScaleKind::Utc),
            TimeScale::Ut1 => Some(TimeScaleKind::Ut1),
            TimeScale::Tai => Some(TimeScaleKind::Tai),
            TimeScale::Tt => Some(TimeScaleKind::Tt),
            TimeScale::Tcg => Some(TimeScaleKind::Tcg),
            TimeScale::Tdb => Some(TimeScaleKind::Tdb),
            TimeScale::Tcb => Some(TimeScaleKind::Tcb),
            TimeScale::Gps => Some(TimeScaleKind::Gps),
            TimeScale::Realized { kind, .. } => Some(*kind),
            TimeScale::Local(_) => None,
        }
    }

    fn is_utc(&self) -> bool {
        self.kind() == Some(TimeScaleKind::Utc)
    }
}

impl FromStr for TimeScale {
    type Err = FitsError;

    fn from_str(s: &str) -> Result<TimeScale> {
        let invalid = || FitsError::InvalidValue {
            card: format!("time scale '{s}'"),
        };
        let value = s.trim();
        let (base, realization) = match value.split_once('(') {
            Some((base, realization))
                if !base.trim().is_empty()
                    && realization.ends_with(')')
                    && realization.len() > 1
                    && !realization[..realization.len() - 1].contains(['(', ')']) =>
            {
                (
                    base.trim(),
                    Some(realization[..realization.len() - 1].to_string()),
                )
            }
            Some(_) => return Err(invalid()),
            None if !value.is_empty() && !value.contains(')') => (value, None),
            None => return Err(invalid()),
        };
        let standard = match base.to_ascii_uppercase().as_str() {
            "UTC" | "GMT" => Some((TimeScale::Utc, TimeScaleKind::Utc)),
            "UT1" | "UT" => Some((TimeScale::Ut1, TimeScaleKind::Ut1)),
            "TAI" | "IAT" => Some((TimeScale::Tai, TimeScaleKind::Tai)),
            "TT" | "TDT" | "ET" => Some((TimeScale::Tt, TimeScaleKind::Tt)),
            "TCG" => Some((TimeScale::Tcg, TimeScaleKind::Tcg)),
            "TDB" => Some((TimeScale::Tdb, TimeScaleKind::Tdb)),
            "TCB" => Some((TimeScale::Tcb, TimeScaleKind::Tcb)),
            "GPS" => Some((TimeScale::Gps, TimeScaleKind::Gps)),
            _ => None,
        };
        match (standard, realization) {
            (Some((scale, _)), None) => Ok(scale),
            (Some((_, kind)), Some(realization)) => Ok(TimeScale::Realized { kind, realization }),
            (None, None) => Ok(TimeScale::Local(base.to_string())),
            (None, Some(_)) => Ok(TimeScale::Local(value.to_string())),
        }
    }
}

fn calendar_day_start(year: i64, month: u32, day: u32) -> f64 {
    gregorian_to_jdn(year, month as i64, day as i64) as f64 - 0.5
}

/// Julian Day Number at noon of a proleptic-Gregorian calendar date (the standard
/// integer formula).
/// Whether `year` is a leap year in the proleptic Gregorian calendar.
fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Number of days in `month` (1–12) of `year`; `0` for an out-of-range month.
fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn gregorian_to_jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month).div_euclid(12);
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2).div_euclid(5) + 365 * y + y.div_euclid(4) - y.div_euclid(100)
        + y.div_euclid(400)
        - 32045
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

/// The §9.6 `'PHASE'` axis folding parameters: the zero-phase reference time
/// `CZPHSia` and the period `CPERIia`, in `TIMEUNIT` relative to the time
/// reference (the `TSTART` convention).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PhaseAxis {
    pub zero_phase: f64,
    /// A constant non-zero `CPERI` value; `None` means the period is undefined or
    /// varies with time.
    pub period: Option<f64>,
}

#[derive(Debug)]
struct PhaseAxisKeywords {
    ctype: KeyBuf,
    zero_phase: KeyBuf,
    period: KeyBuf,
}

impl PhaseAxisKeywords {
    fn image(axis: usize, alt: Option<char>) -> PhaseAxisKeywords {
        let suffix = alt.map(|value| value.to_string()).unwrap_or_default();
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

/// The spatial location at which a FITS time coordinate is valid.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeReferencePosition {
    Topocenter,
    Geocenter,
    Barycenter,
    Relocatable,
    Custom,
    Heliocenter,
    GalacticCenter,
    EarthMoonBarycenter,
    Mercury,
    Venus,
    Mars,
    Jupiter,
    Saturn,
    Uranus,
    Neptune,
    /// A non-standard value retained exactly as declared.
    Other(String),
}

impl TimeReferencePosition {
    fn parse(value: &str) -> TimeReferencePosition {
        match value {
            value if value.starts_with("TOP") => TimeReferencePosition::Topocenter,
            value if value.starts_with("GEO") => TimeReferencePosition::Geocenter,
            value if value.starts_with("BAR") => TimeReferencePosition::Barycenter,
            value if value.starts_with("REL") => TimeReferencePosition::Relocatable,
            value if value.starts_with("CUS") => TimeReferencePosition::Custom,
            value if value.starts_with("HEL") => TimeReferencePosition::Heliocenter,
            value if value.starts_with("GAL") => TimeReferencePosition::GalacticCenter,
            value if value.starts_with("EMB") => TimeReferencePosition::EarthMoonBarycenter,
            value if value.starts_with("MER") => TimeReferencePosition::Mercury,
            value if value.starts_with("VEN") => TimeReferencePosition::Venus,
            value if value.starts_with("MAR") => TimeReferencePosition::Mars,
            value if value.starts_with("JUP") => TimeReferencePosition::Jupiter,
            value if value.starts_with("SAT") => TimeReferencePosition::Saturn,
            value if value.starts_with("URA") => TimeReferencePosition::Uranus,
            value if value.starts_with("NEP") => TimeReferencePosition::Neptune,
            value => TimeReferencePosition::Other(value.to_string()),
        }
    }
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
        let metadata =
            wcs.view()
                .axes
                .get(zero_based)
                .ok_or(FitsError::WcsAxisIndexOutOfBounds {
                    axis,
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

#[cfg(test)]
mod tests;
