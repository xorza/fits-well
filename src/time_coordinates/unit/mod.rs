//! [`Unit`]: a parsed `TIMEUNIT` or time-axis `CUNITi`.

use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::TimeConversion;
use crate::fits_error::UnitKind;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use crate::unit;
use crate::unit::SECONDS_PER_DAY;
use crate::unit::{PowerOfTen, UnitParts};
use std::fmt;
use std::str::FromStr;

/// A time unit (§9.3, Table 31), parsed once from its keyword, as it is spelled: a
/// power-of-ten multiplier, and a base unit with its SI prefix or a year of epoch-dependent
/// length. Most units have a fixed length; the tropical and Besselian years have a length
/// that depends on the reference epoch and is defined only in a TDB or TT frame, so
/// [`Unit::seconds`] evaluates it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Unit {
    spelling: Spelling,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Spelling {
    /// A unit of fixed length from the time table.
    Fixed(UnitParts),
    /// A multiple of the tropical year (`ta`).
    TropicalYears(PowerOfTen),
    /// A multiple of the Besselian year (`Ba`).
    BesselianYears(PowerOfTen),
}

impl Unit {
    /// The unit's length in seconds at the epoch `reference_mjd` in `scale`. Errors with
    /// [`FitsError::ExternalTimeDataRequired`] for a tropical year outside TDB or a
    /// Besselian year outside TT, which need ephemeris data.
    pub fn seconds(self, reference_mjd: Mjd, scale: &Scale) -> Result<f64> {
        let reference_mjd = reference_mjd.to_f64();
        match self.spelling {
            Spelling::Fixed(parts) => Ok(parts.scale()),
            Spelling::TropicalYears(multiplier) => Ok(multiplier.factor()
                * tropical_year_days(reference_mjd, scale)?
                * SECONDS_PER_DAY),
            Spelling::BesselianYears(multiplier) => Ok(multiplier.factor()
                * besselian_year_days(reference_mjd, scale)?
                * SECONDS_PER_DAY),
        }
    }
}

/// The unit `text` spells, with SI prefixes and a numeric multiplier. Errors with
/// [`FitsError::InvalidUnit`] for any other text.
impl FromStr for Unit {
    type Err = FitsError;

    fn from_str(text: &str) -> Result<Self> {
        let invalid = || FitsError::InvalidUnit {
            unit: text.to_string(),
            expected: UnitKind::Time,
        };
        if let Some(parts) = unit::split(text, unit::TIME) {
            return Ok(Self {
                spelling: Spelling::Fixed(parts),
            });
        }
        let scaled = unit::split_numeric_multiplier(text).ok_or_else(invalid)?;
        let spelling = match scaled.base {
            "ta" => Spelling::TropicalYears(scaled.multiplier),
            "Ba" => Spelling::BesselianYears(scaled.multiplier),
            _ => return Err(invalid()),
        };
        Ok(Self { spelling })
    }
}

/// The multiplier as `10**k` when it is not 1, then the prefix and the base: `ms`,
/// `10**3 d`, `ta`. A spelling of the multiplier other than `10**k` prints as `10**k`.
impl fmt::Display for Unit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let multiplier = match self.spelling {
            Spelling::Fixed(parts) => parts.multiplier,
            Spelling::TropicalYears(multiplier) | Spelling::BesselianYears(multiplier) => {
                multiplier
            }
        };
        if multiplier.exponent() != 0 {
            write!(f, "10**{} ", multiplier.exponent())?;
        }
        match self.spelling {
            Spelling::Fixed(parts) => {
                if let Some(prefix) = parts.prefix {
                    f.write_str(prefix.symbol)?;
                }
                f.write_str(parts.base.symbol)
            }
            Spelling::TropicalYears(_) => f.write_str("ta"),
            Spelling::BesselianYears(_) => f.write_str("Ba"),
        }
    }
}

fn tropical_year_days(reference_mjd: f64, scale: &Scale) -> Result<f64> {
    if scale.kind() != Some(ScaleKind::Tdb) {
        return Err(FitsError::ExternalTimeDataRequired {
            conversion: TimeConversion::TropicalYear,
        });
    }
    let centuries = (reference_mjd - 51_544.5) / 36_525.0;
    Ok(
        365.242_190_402_112_4 - 0.000_006_152_513_49 * centuries - 6.0921e-10 * centuries.powi(2)
            + 2.6525e-10 * centuries.powi(3),
    )
}

fn besselian_year_days(reference_mjd: f64, scale: &Scale) -> Result<f64> {
    if scale.kind() != Some(ScaleKind::Tt) {
        return Err(FitsError::ExternalTimeDataRequired {
            conversion: TimeConversion::BesselianYear,
        });
    }
    let centuries = (reference_mjd - 15_019.5) / 36_525.0;
    Ok(365.242_198_781_7 - 0.000_007_854_23 * centuries)
}

#[cfg(test)]
mod tests;
