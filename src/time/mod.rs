//! Typed time coordinates (§9) — behind the `time` feature.
//!
//! Covers the computational core: ISO-8601 datetimes ↔ Julian Date / MJD
//! (proleptic-Gregorian calendar math), `J`/`B` epochs → JD, time-scale
//! conversions ([`TimeScale`]) among `UTC`/`TAI`/`TT`/`GPS`/`TCG`/`TDB`/`TCB`
//! (UTC↔TAI via an embedded leap-second table; TDB via the standard periodic
//! approximation), and a [`FitsTime`] view over a header's time keywords
//! (`TIMESYS`, `MJDREF*`/`JDREF*`/`DATEREF`, `TIMEUNIT`, `TREFPOS`, and the global
//! `DATE-OBS`/`MJD-OBS`/`TSTART`/… set). Validated against `astropy.time`.

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

/// JD of the MJD zero point (1858-11-17T00:00 UTC).
const MJD0: f64 = 2_400_000.5;
/// 1977-01-01T00:00:00 TAI — the defining epoch where TT/TCG/TCB/TDB coincide.
const T1977_JD: f64 = 2_443_144.5;
/// TT − TAI, seconds (exact, by definition).
const TT_TAI: f64 = 32.184;
/// GPS = TAI − 19 s (GPS time is offset from TAI by a fixed 19 s).
const TAI_GPS: f64 = 19.0;
/// TCG rate: `(TCG − TT) = L_G · (TT − 1977.0)` (IAU 2000 Resolution B1.9).
const L_G: f64 = 6.969_290_134e-10;
/// TCB rate: `(TCB − TDB) ≈ L_B · (TDB − 1977.0)` (IAU 2006 Resolution B3).
const L_B: f64 = 1.550_519_768e-8;
const SEC_PER_DAY: f64 = 86_400.0;

/// A calendar datetime (proleptic Gregorian, UTC-agnostic). `second` may reach
/// 60.x to represent a leap second; the JD arithmetic rolls it over naturally.
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
    /// Parse an ISO-8601 `FITS` datetime: `YYYY-MM-DD` or
    /// `YYYY-MM-DDThh:mm:ss[.sss…]` (§9.1.1). No component defaulting; the date is
    /// required, the time part optional (then midnight).
    pub fn parse(s: &str) -> Result<Datetime> {
        let invalid = || FitsError::InvalidValue {
            card: format!("DATE '{s}'"),
        };
        let s = s.trim();
        let (date, time) = match s.split_once('T') {
            Some((d, t)) => (d, Some(t)),
            None => (s, None),
        };
        let mut dp = date.split('-');
        // A leading '-' (negative year) splits into an empty first field.
        let (neg, y_str) = match dp.next() {
            Some("") => (true, dp.next().ok_or_else(invalid)?),
            Some(y) => (false, y),
            None => return Err(invalid()),
        };
        let year: i64 = y_str.parse().map_err(|_| invalid())?;
        let year = if neg { -year } else { year };
        let month: u32 = dp
            .next()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        let day: u32 = dp
            .next()
            .ok_or_else(invalid)?
            .parse()
            .map_err(|_| invalid())?;
        if dp.next().is_some() || !(1..=12).contains(&month) || !(1..=31).contains(&day) {
            return Err(invalid());
        }

        let (mut hour, mut minute, mut second) = (0u32, 0u32, 0.0f64);
        if let Some(t) = time {
            let mut tp = t.split(':');
            hour = tp
                .next()
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
            minute = tp
                .next()
                .ok_or_else(invalid)?
                .parse()
                .map_err(|_| invalid())?;
            if let Some(sec) = tp.next() {
                second = sec.parse().map_err(|_| invalid())?;
            }
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

    /// Julian Date of this datetime, interpreting the fields in their own time
    /// scale (no scale conversion is applied here).
    pub fn to_jd(&self) -> f64 {
        let day_fraction =
            (self.hour as f64 * 3600.0 + self.minute as f64 * 60.0 + self.second) / SEC_PER_DAY;
        gregorian_to_jdn(self.year, self.month as i64, self.day as i64) as f64 - 0.5 + day_fraction
    }

    /// Modified Julian Date (`JD − 2400000.5`).
    pub fn to_mjd(&self) -> f64 {
        self.to_jd() - MJD0
    }

    /// Build a datetime from a JD (inverse of [`Datetime::to_jd`]). A single `f64`
    /// JD at present epochs (~2.46e6) resolves to ~0.1 ms, so the recovered second
    /// carries that much rounding — fine for display, not for sub-ms timing.
    pub fn from_jd(jd: f64) -> Datetime {
        // Split into integer day (JDN at noon) and the fraction past midnight.
        let z = (jd + 0.5).floor();
        let frac = (jd + 0.5) - z;
        let (year, month, day) = jdn_to_gregorian(z as i64);
        let mut secs = frac * SEC_PER_DAY;
        let hour = (secs / 3600.0).floor();
        secs -= hour * 3600.0;
        let minute = (secs / 60.0).floor();
        secs -= minute * 60.0;
        Datetime {
            year,
            month,
            day,
            hour: hour as u32,
            minute: minute as u32,
            second: secs,
        }
    }
}

/// A reference epoch: Julian (`J2000.0`) or Besselian (`B1950.0`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Epoch {
    /// Julian epoch in years (e.g. `2000.0`).
    Julian(f64),
    /// Besselian epoch in years (e.g. `1950.0`).
    Besselian(f64),
}

impl Epoch {
    /// Parse `J<year>` or `B<year>` (e.g. `'J2000.0'`, `'B1950.0'`).
    pub fn parse(s: &str) -> Result<Epoch> {
        let s = s.trim();
        let invalid = || FitsError::InvalidValue {
            card: format!("epoch '{s}'"),
        };
        let (tag, rest) = s.split_at(
            s.char_indices()
                .next()
                .map(|(_, c)| c.len_utf8())
                .unwrap_or(0),
        );
        let year: f64 = rest.parse().map_err(|_| invalid())?;
        match tag {
            "J" | "j" => Ok(Epoch::Julian(year)),
            "B" | "b" => Ok(Epoch::Besselian(year)),
            _ => Err(invalid()),
        }
    }

    /// Julian Date of the epoch. Julian: `2451545.0 + (y−2000)·365.25`; Besselian:
    /// `2415020.31352 + (y−1900)·365.242198781` (the tropical year).
    pub fn to_jd(self) -> f64 {
        match self {
            Epoch::Julian(y) => 2_451_545.0 + (y - 2000.0) * 365.25,
            Epoch::Besselian(y) => 2_415_020.313_52 + (y - 1900.0) * 365.242_198_781,
        }
    }

    /// Modified Julian Date of the epoch.
    pub fn to_mjd(self) -> f64 {
        self.to_jd() - MJD0
    }
}

/// A FITS time scale (`TIMESYS` / `CTYPEi`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimeScale {
    /// Coordinated Universal Time.
    Utc,
    /// Universal Time (UT1) — treated as UTC here (ΔUT1 needs an external table).
    Ut1,
    /// International Atomic Time.
    Tai,
    /// Terrestrial Time.
    Tt,
    /// Geocentric Coordinate Time.
    Tcg,
    /// Barycentric Dynamical Time.
    Tdb,
    /// Barycentric Coordinate Time.
    Tcb,
    /// GPS time.
    Gps,
    /// An unspecified local clock (`LOCAL`); no conversion is possible.
    Local,
}

impl TimeScale {
    /// Parse a `TIMESYS`/`CTYPE` time-scale string (case-insensitive); accepts the
    /// `TDT`/`ET` → `TT` and `IAT` → `TAI` aliases. Unknown values map to `LOCAL`.
    pub fn parse(s: &str) -> TimeScale {
        match s.trim().to_ascii_uppercase().as_str() {
            "UTC" => TimeScale::Utc,
            "UT1" | "UT" => TimeScale::Ut1,
            "TAI" | "IAT" => TimeScale::Tai,
            "TT" | "TDT" | "ET" => TimeScale::Tt,
            "TCG" => TimeScale::Tcg,
            "TDB" => TimeScale::Tdb,
            "TCB" => TimeScale::Tcb,
            "GPS" => TimeScale::Gps,
            _ => TimeScale::Local,
        }
    }

    /// Convert a Julian Date in this scale to a JD in `target`, treating `UT1` as
    /// `UTC` (ΔUT1 = 0). Use [`TimeScale::convert_dut1`] to supply a real ΔUT1.
    pub fn convert(self, jd: f64, target: TimeScale) -> f64 {
        self.convert_dut1(jd, target, 0.0)
    }

    /// Convert a Julian Date with an explicit `dut1 = UT1 − UTC` (seconds, from
    /// IERS) so conversions to/from `UT1` are exact. `Local` passes through.
    pub fn convert_dut1(self, jd: f64, target: TimeScale, dut1: f64) -> f64 {
        if self == target || self == TimeScale::Local || target == TimeScale::Local {
            return jd;
        }
        from_tt(self.to_tt(jd, dut1), target, dut1)
    }

    /// This scale's JD expressed as TT (the common pivot). `dut1 = UT1 − UTC` (s).
    fn to_tt(self, jd: f64, dut1: f64) -> f64 {
        match self {
            TimeScale::Tt => jd,
            TimeScale::Tai => jd + TT_TAI / SEC_PER_DAY,
            TimeScale::Gps => jd + (TT_TAI + TAI_GPS) / SEC_PER_DAY,
            TimeScale::Utc | TimeScale::Local => {
                jd + (TT_TAI + leap_seconds(jd - MJD0)) / SEC_PER_DAY
            }
            // UT1 → UTC (subtract ΔUT1) → TT.
            TimeScale::Ut1 => {
                let utc = jd - dut1 / SEC_PER_DAY;
                utc + (TT_TAI + leap_seconds(utc - MJD0)) / SEC_PER_DAY
            }
            TimeScale::Tcg => jd - L_G * (jd - T1977_JD),
            TimeScale::Tdb => jd - tdb_minus_tt(jd) / SEC_PER_DAY,
            TimeScale::Tcb => {
                let tdb = jd - L_B * (jd - T1977_JD);
                tdb - tdb_minus_tt(tdb) / SEC_PER_DAY
            }
        }
    }
}

/// TT (as a JD) expressed in `target` — the inverse of [`TimeScale::to_tt`].
fn from_tt(tt: f64, target: TimeScale, dut1: f64) -> f64 {
    match target {
        TimeScale::Tt => tt,
        TimeScale::Tai => tt - TT_TAI / SEC_PER_DAY,
        TimeScale::Gps => tt - (TT_TAI + TAI_GPS) / SEC_PER_DAY,
        TimeScale::Utc | TimeScale::Local => {
            let tai = tt - TT_TAI / SEC_PER_DAY;
            // leap is a function of UTC; one lookup at the TAI date suffices away
            // from the ≤1 s boundary ambiguity inherent to UTC.
            tai - leap_seconds(tai - MJD0) / SEC_PER_DAY
        }
        // TT → UTC → UT1 (add ΔUT1).
        TimeScale::Ut1 => {
            let tai = tt - TT_TAI / SEC_PER_DAY;
            let utc = tai - leap_seconds(tai - MJD0) / SEC_PER_DAY;
            utc + dut1 / SEC_PER_DAY
        }
        TimeScale::Tcg => tt + L_G * (tt - T1977_JD),
        TimeScale::Tdb => tt + tdb_minus_tt(tt) / SEC_PER_DAY,
        TimeScale::Tcb => {
            let tdb = tt + tdb_minus_tt(tt) / SEC_PER_DAY;
            tdb + L_B * (tdb - T1977_JD)
        }
    }
}

/// `TDB − TT` in seconds — the standard periodic approximation (~10 µs accuracy):
/// `0.001658·sin g + 0.000014·sin 2g`, `g = 357.53° + 0.9856003°·(JD_TT − J2000)`.
fn tdb_minus_tt(jd_tt: f64) -> f64 {
    let g = (357.53 + 0.985_600_3 * (jd_tt - 2_451_545.0)).to_radians();
    0.001_658 * g.sin() + 0.000_014 * (2.0 * g).sin()
}

/// `TAI − UTC` in seconds for a given UTC MJD: the integer leap-second count from
/// the IERS table (1972–2017). Clamped to the table ends outside that range.
fn leap_seconds(mjd: f64) -> f64 {
    // (year, month, day, TAI−UTC) at each step, 1972 onward.
    const TABLE: &[(i64, i64, i64, f64)] = &[
        (1972, 1, 1, 10.0),
        (1972, 7, 1, 11.0),
        (1973, 1, 1, 12.0),
        (1974, 1, 1, 13.0),
        (1975, 1, 1, 14.0),
        (1976, 1, 1, 15.0),
        (1977, 1, 1, 16.0),
        (1978, 1, 1, 17.0),
        (1979, 1, 1, 18.0),
        (1980, 1, 1, 19.0),
        (1981, 7, 1, 20.0),
        (1982, 7, 1, 21.0),
        (1983, 7, 1, 22.0),
        (1985, 7, 1, 23.0),
        (1988, 1, 1, 24.0),
        (1990, 1, 1, 25.0),
        (1991, 1, 1, 26.0),
        (1992, 7, 1, 27.0),
        (1993, 7, 1, 28.0),
        (1994, 7, 1, 29.0),
        (1996, 1, 1, 30.0),
        (1997, 7, 1, 31.0),
        (1999, 1, 1, 32.0),
        (2006, 1, 1, 33.0),
        (2009, 1, 1, 34.0),
        (2012, 7, 1, 35.0),
        (2015, 7, 1, 36.0),
        (2017, 1, 1, 37.0),
    ];
    let mut leap = TABLE[0].3;
    for &(y, m, d, l) in TABLE {
        let threshold = gregorian_to_jdn(y, m, d) as f64 - 0.5 - MJD0;
        if mjd >= threshold {
            leap = l;
        } else {
            break;
        }
    }
    leap
}

/// Julian Day Number at noon of a proleptic-Gregorian calendar date (the standard
/// integer formula).
fn gregorian_to_jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month) / 12;
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2) / 5 + 365 * y + y / 4 - y / 100 + y / 400 - 32045
}

/// Inverse of [`gregorian_to_jdn`]: JDN → `(year, month, day)`.
fn jdn_to_gregorian(jdn: i64) -> (i64, u32, u32) {
    let a = jdn + 32044;
    let b = (4 * a + 3) / 146097;
    let c = a - (146097 * b) / 4;
    let d = (4 * c + 3) / 1461;
    let e = c - (1461 * d) / 4;
    let m = (5 * e + 2) / 153;
    let day = e - (153 * m + 2) / 5 + 1;
    let month = m + 3 - 12 * (m / 10);
    let year = 100 * b + d - 4800 + m / 10;
    (year, month as u32, day as u32)
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
    /// `TREFPOS` (reference position, e.g. `'TOPOCENTER'`), if present.
    pub trefpos: Option<String>,
}

impl FitsTime {
    /// Parse the time frame from a header.
    pub fn from_header(header: &Header) -> FitsTime {
        let scale = header
            .get_text("TIMESYS")
            .map(TimeScale::parse)
            .unwrap_or(TimeScale::Utc);
        let timeunit = header.get_text("TIMEUNIT").unwrap_or("s").to_string();
        let trefpos = header.get_text("TREFPOS").map(str::to_string);
        FitsTime {
            scale,
            mjdref: reference_mjd(header),
            timeunit,
            trefpos,
        }
    }

    /// `TIMEUNIT` expressed in seconds (`s`, `d`/day, `a`/`yr` Julian year).
    pub fn unit_seconds(&self) -> f64 {
        match self.timeunit.trim() {
            "d" | "day" => SEC_PER_DAY,
            "a" | "yr" | "y" => 365.25 * SEC_PER_DAY,
            _ => 1.0, // seconds (the default)
        }
    }

    /// Resolve a time value measured *relative* to `MJDREF` (e.g. `TSTART`,
    /// `TSTOP`), in `TIMEUNIT`, to an absolute MJD in the frame's own scale.
    pub fn relative_to_mjd(&self, value: f64) -> f64 {
        self.mjdref + value * self.unit_seconds() / SEC_PER_DAY
    }

    /// The observation MJD from `MJD-OBS`, else `DATE-OBS`, else `None`.
    pub fn obs_mjd(&self, header: &Header) -> Option<f64> {
        if let Some(mjd) = header.get_real("MJD-OBS") {
            return Some(mjd);
        }
        header
            .get_text("DATE-OBS")
            .and_then(|s| Datetime::parse(s).ok())
            .map(|d| d.to_mjd())
    }

    /// If WCS axis `axis` (1-based) is a time axis (`CTYPEi = 'TIME'` or a
    /// time-scale name, §9.2.3), convert a 1-based pixel coordinate along it to an
    /// absolute MJD in the frame's scale: the linear axis value (elapsed time in
    /// `TIMEUNIT` from `MJDREF`) plus the reference. `None` if not a time axis.
    pub fn time_axis_mjd(&self, header: &Header, axis: usize, pixel: f64) -> Option<f64> {
        let ctype = header.get_text(&format!("CTYPE{axis}"))?;
        if !is_time_ctype(ctype) {
            return None;
        }
        let crpix = header.get_real(&format!("CRPIX{axis}")).unwrap_or(0.0);
        let crval = header.get_real(&format!("CRVAL{axis}")).unwrap_or(0.0);
        let cdelt = header.get_real(&format!("CDELT{axis}")).unwrap_or(1.0);
        Some(self.relative_to_mjd(crval + cdelt * (pixel - crpix)))
    }
}

/// True if a `CTYPE` denotes a time axis: `'TIME'` or a recognized time-scale
/// name (`'UTC'`, `'TT'`, …).
pub fn is_time_ctype(ctype: &str) -> bool {
    let head = ctype.split('-').next().unwrap_or("").trim();
    head == "TIME" || !matches!(TimeScale::parse(head), TimeScale::Local)
}

/// The reference epoch as MJD: `MJDREF` (or `MJDREFI`+`MJDREFF`), else `JDREF`
/// (or `JDREFI`+`JDREFF`), else `DATEREF`, else `0.0`.
fn reference_mjd(header: &Header) -> f64 {
    if let Some(mjd) = header.get_real("MJDREF") {
        return mjd;
    }
    if header.get_real("MJDREFI").is_some() || header.get_real("MJDREFF").is_some() {
        return header.get_real("MJDREFI").unwrap_or(0.0)
            + header.get_real("MJDREFF").unwrap_or(0.0);
    }
    if let Some(jd) = header.get_real("JDREF") {
        return jd - MJD0;
    }
    if header.get_real("JDREFI").is_some() || header.get_real("JDREFF").is_some() {
        return header.get_real("JDREFI").unwrap_or(0.0) + header.get_real("JDREFF").unwrap_or(0.0)
            - MJD0;
    }
    header
        .get_text("DATEREF")
        .and_then(|s| Datetime::parse(s).ok())
        .map(|d| d.to_mjd())
        .unwrap_or(0.0)
}

#[cfg(test)]
mod tests;
