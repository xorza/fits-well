//! [`Datetime`]: a FITS ISO-8601 calendar datetime and its Julian Date
//! conversions.

use crate::error::FitsError;
use crate::error::Result;
use crate::time_impl::time_scale::TimeScale;
use crate::time_impl::{MJD0, SEC_PER_DAY};

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
