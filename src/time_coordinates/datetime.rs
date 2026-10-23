//! [`Datetime`]: a FITS ISO-8601 calendar datetime and its MJD.

use crate::error::FitsError;
use crate::error::Result;
use crate::time_coordinates::SEC_PER_DAY;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use std::str::FromStr;

/// A calendar datetime (proleptic Gregorian, time-scale agnostic). `second` may
/// reach 60.x; [`Datetime::to_mjd`] accepts that label only for UTC.
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
    /// The MJD of this datetime in its declared `scale`: the date's day, and the time
    /// of day rounded once into the fraction.
    ///
    /// UTC leap-second labels remain valid FITS datetimes, but converting one to
    /// a continuous coordinate requires an external leap-second realization.
    pub fn to_mjd(&self, scale: &Scale) -> Result<Mjd> {
        self.validate(scale)?;
        if self.second >= 60.0 {
            return Err(FitsError::ExternalTimeDataRequired {
                operation: "convert a UTC leap-second label to MJD",
            });
        }
        // The JDN counts from noon and the MJD from the midnight 2400000.5 days later.
        let day =
            gregorian_to_jdn(self.year, i64::from(self.month), i64::from(self.day)) - 2_400_001;
        Ok(Mjd::new(day, self.day_fraction()))
    }

    fn day_fraction(&self) -> f64 {
        (f64::from(self.hour) * 3600.0 + f64::from(self.minute) * 60.0 + self.second) / SEC_PER_DAY
    }

    fn validate(&self, scale: &Scale) -> Result<()> {
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
                && scale.kind() == Some(ScaleKind::Utc)
                && self.hour == 23
                && self.minute == 59
        };
        if valid_date && valid_time && valid_second {
            return Ok(());
        }
        Err(FitsError::InvalidTime {
            detail: format!("datetime {self:?} in {scale:?}"),
        })
    }
}

/// Parse a FITS ISO-8601 datetime: unsigned `YYYY-MM-DD` or signed
/// `±YYYYY-MM-DD`, optionally followed by `Thh:mm:ss[.sss…]` (§9.1.1). No
/// component defaulting; the date is required, the time part optional.
impl FromStr for Datetime {
    type Err = FitsError;

    fn from_str(s: &str) -> Result<Self> {
        let invalid = || FitsError::InvalidTime {
            detail: format!("DATE '{s}'"),
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
        Ok(Self {
            year,
            month,
            day,
            hour,
            minute,
            second,
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
    if s.len() != width || !all_digits(s) {
        return None;
    }
    s.parse().ok()
}

/// Parse a `ss[.s…]` seconds field: exactly two integer digits, optional fraction.
fn parse_seconds(s: &str) -> Option<f64> {
    let (int, frac) = s.split_once('.').map_or((s, None), |(i, f)| (i, Some(f)));
    if int.len() != 2 || !all_digits(int) || frac.is_some_and(|f| !all_digits(f)) {
        return None;
    }
    s.parse().ok()
}

/// Whether `year` is a leap year in the proleptic Gregorian calendar.
const fn is_leap_year(year: i64) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

/// Number of days in `month` (1–12) of `year`; `0` for an out-of-range month.
const fn days_in_month(year: i64, month: u32) -> u32 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

/// Julian Day Number at noon of a proleptic-Gregorian calendar date (the standard
/// integer formula).
const fn gregorian_to_jdn(year: i64, month: i64, day: i64) -> i64 {
    let a = (14 - month).div_euclid(12);
    let y = year + 4800 - a;
    let m = month + 12 * a - 3;
    day + (153 * m + 2).div_euclid(5) + 365 * y + y.div_euclid(4) - y.div_euclid(100)
        + y.div_euclid(400)
        - 32045
}
