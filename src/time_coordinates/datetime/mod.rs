//! [`Datetime`]: a FITS ISO-8601 calendar datetime and its MJD.

use crate::error::FitsError;
use crate::error::Result;
use crate::error::TimeConversion;
use crate::header_model::Header;
use crate::keyword::Keyword;
use crate::time_coordinates::mjd::{Expansion, Mjd};
use crate::unit::SECONDS_PER_DAY;
use std::str::FromStr;

/// A calendar datetime (proleptic Gregorian, time-scale agnostic), valid by
/// construction: a date of years −99999 to 99999, and a time of day whose second may be
/// 60 only at 23:59, the label of a UTC leap second, which [`Datetime::to_mjd`] cannot
/// convert without a leap-second table.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Datetime {
    year: i64,
    month: u32,
    day: u32,
    hour: u32,
    minute: u32,
    /// The whole second, 0 to 60.
    second: u32,
    /// The fraction of the second, in [0, 1).
    fraction: f64,
}

impl Datetime {
    /// The datetime `keyword` of `header` holds; `None` when it holds none.
    pub(crate) fn from_keyword(header: &Header, keyword: Keyword) -> Result<Option<Self>> {
        header
            .get_text(keyword)?
            .map(|value| {
                value.parse().map_err(|_| FitsError::InvalidDatetime {
                    keyword: Some(keyword),
                })
            })
            .transpose()
    }

    /// Midnight of the date `year`-`month`-`day`. Errors with
    /// [`FitsError::InvalidDatetime`] for a date that does not exist.
    pub fn date(year: i64, month: u32, day: u32) -> Result<Self> {
        let valid = (-99999..=99999).contains(&year)
            && (1..=12).contains(&month)
            && (1..=days_in_month(year, month)).contains(&day);
        if !valid {
            return Err(FitsError::InvalidDatetime { keyword: None });
        }
        Ok(Self {
            year,
            month,
            day,
            hour: 0,
            minute: 0,
            second: 0,
            fraction: 0.0,
        })
    }

    /// This date at `hour`:`minute`:`second`. Errors with [`FitsError::InvalidDatetime`]
    /// for a time that does not exist: `second` lies in [0, 60), or in [60, 61) at 23:59.
    pub fn at(self, hour: u32, minute: u32, second: f64) -> Result<Self> {
        if !(0.0..61.0).contains(&second) {
            return Err(FitsError::InvalidDatetime { keyword: None });
        }
        let whole = second.floor();
        #[expect(
            clippy::cast_sign_loss,
            reason = "a whole second in [0, 60] converts exactly"
        )]
        let whole_second = whole as u32;
        let second = SplitSecond {
            whole: whole_second,
            fraction: second - whole,
        };
        self.at_split(hour, minute, second)
    }

    fn at_split(self, hour: u32, minute: u32, second: SplitSecond) -> Result<Self> {
        let SplitSecond {
            whole: second,
            fraction,
        } = second;
        debug_assert!((0.0..1.0).contains(&fraction), "a fraction of a second");
        let leap = second == 60 && hour == 23 && minute == 59;
        if hour >= 24 || minute >= 60 || (second >= 60 && !leap) {
            return Err(FitsError::InvalidDatetime { keyword: None });
        }
        Ok(Self {
            hour,
            minute,
            second,
            fraction,
            ..self
        })
    }

    pub const fn year(self) -> i64 {
        self.year
    }

    pub const fn month(self) -> u32 {
        self.month
    }

    pub const fn day(self) -> u32 {
        self.day
    }

    pub const fn hour(self) -> u32 {
        self.hour
    }

    pub const fn minute(self) -> u32 {
        self.minute
    }

    /// The whole second: 60 for a leap second.
    pub const fn second(self) -> u32 {
        self.second
    }

    /// The fraction of the second, in [0, 1).
    pub const fn second_fraction(self) -> f64 {
        self.fraction
    }

    /// The MJD of this datetime: the date's day, and the time of day rounded once into
    /// the fraction. Errors with [`FitsError::ExternalTimeDataRequired`] for a leap
    /// second, whose instant needs a leap-second table.
    pub fn to_mjd(self) -> Result<Mjd> {
        if self.second >= 60 {
            return Err(FitsError::ExternalTimeDataRequired {
                conversion: TimeConversion::LeapSecond,
            });
        }
        // The JDN counts from noon and the MJD from the midnight 2400000.5 days later.
        let day =
            gregorian_to_jdn(self.year, i64::from(self.month), i64::from(self.day)) - 2_400_001;
        let seconds = self.hour * 3600 + self.minute * 60 + self.second;
        let time = Expansion::two_sum(f64::from(seconds), self.fraction).divided(SECONDS_PER_DAY);
        Ok(Mjd::new(day, 0.0)
            .plus(time)
            .expect("a time of day moves an MJD by less than a day"))
    }
}

/// Parse a FITS datetime: unsigned `YYYY-MM-DD` or signed `±YYYYY-MM-DD`, optionally
/// followed by `Thh:mm:ss[.sss…]` (§9.1.1), with no component defaulting; or the
/// `DD/MM/YY` date of 1900 to 1999 that `DATE` held before 2000 (§4.4.2.1), as cfitsio's
/// `ffs2dt` reads it. A fraction of a second too close to 1 for an `f64` keeps the
/// largest `f64` below 1, so `59.999…` never becomes 60.
impl FromStr for Datetime {
    type Err = FitsError;

    fn from_str(s: &str) -> Result<Self> {
        let invalid = || FitsError::InvalidDatetime { keyword: None };
        let s = s.trim();
        if let [d1, d2, b'/', m1, m2, b'/', y1, y2] = *s.as_bytes() {
            let field = |a: u8, b: u8| -> Option<u32> {
                (a.is_ascii_digit() && b.is_ascii_digit())
                    .then(|| u32::from(a - b'0') * 10 + u32::from(b - b'0'))
            };
            let (day, month, year) = (
                field(d1, d2).ok_or_else(invalid)?,
                field(m1, m2).ok_or_else(invalid)?,
                field(y1, y2).ok_or_else(invalid)?,
            );
            return Self::date(1900 + i64::from(year), month, day);
        }
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
        let date = Self::date(year, month, day)?;
        let Some(time) = time else {
            return Ok(date);
        };
        let mut tp = time.split(':');
        let hour = parse_fixed(tp.next().ok_or_else(invalid)?, 2).ok_or_else(invalid)?;
        let minute = parse_fixed(tp.next().ok_or_else(invalid)?, 2).ok_or_else(invalid)?;
        let seconds = tp.next().ok_or_else(invalid)?;
        if tp.next().is_some() {
            return Err(invalid());
        }
        let (whole, fraction) = match seconds.split_once('.') {
            Some((whole, fraction)) => (whole, Some(fraction)),
            None => (seconds, None),
        };
        let second = parse_fixed(whole, 2).ok_or_else(invalid)?;
        let fraction = match fraction {
            Some(digits) if all_digits(digits) => {
                let value: f64 = format!("0.{digits}").parse().map_err(|_| invalid())?;
                value.min(1.0 - f64::EPSILON / 2.0)
            }
            Some(_) => return Err(invalid()),
            None => 0.0,
        };
        date.at_split(
            hour,
            minute,
            SplitSecond {
                whole: second,
                fraction,
            },
        )
    }
}

/// A second of a time of day: its whole second, 0 to 60, and its fraction, in [0, 1).
#[derive(Debug, Clone, Copy)]
struct SplitSecond {
    whole: u32,
    fraction: f64,
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

#[cfg(test)]
mod tests;
