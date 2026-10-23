//! [`Unit`]: a parsed `TIMEUNIT` or time-axis `CUNITi`.

use crate::error::FitsError;
use crate::error::Result;
use crate::time_coordinates::SEC_PER_DAY;
use crate::time_coordinates::mjd::Mjd;
use crate::time_coordinates::scale::{Scale, ScaleKind};
use crate::unit;
use std::str::FromStr;

/// A time unit (§9.3, Table 31), parsed once from its keyword. Most units have a fixed
/// length; the tropical and Besselian years have a length that depends on the reference
/// epoch and is defined only in a TDB or TT frame, so it is evaluated by
/// [`Unit::seconds`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Unit {
    /// A fixed length, in seconds.
    Seconds(f64),
    /// A multiple of the tropical year (`ta`).
    TropicalYears(f64),
    /// A multiple of the Besselian year (`Ba`).
    BesselianYears(f64),
}

impl Unit {
    /// The unit's length in seconds at the epoch `reference_mjd` in `scale`. Errors with
    /// [`FitsError::ExternalTimeDataRequired`] for a tropical year outside TDB or a
    /// Besselian year outside TT, which need ephemeris data.
    pub fn seconds(self, reference_mjd: Mjd, scale: &Scale) -> Result<f64> {
        let reference_mjd = reference_mjd.to_f64();
        match self {
            Self::Seconds(seconds) => Ok(seconds),
            Self::TropicalYears(factor) => {
                Ok(factor * tropical_year_days(reference_mjd, scale)? * SEC_PER_DAY)
            }
            Self::BesselianYears(factor) => {
                Ok(factor * besselian_year_days(reference_mjd, scale)? * SEC_PER_DAY)
            }
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
            expected: "a time unit",
        };
        if let Some(seconds) = unit::resolve(text, unit::TIME) {
            return Ok(Self::Seconds(seconds));
        }
        let scaled = unit::split_numeric_multiplier(text).ok_or_else(invalid)?;
        match scaled.base {
            "ta" => Ok(Self::TropicalYears(scaled.factor)),
            "Ba" => Ok(Self::BesselianYears(scaled.factor)),
            _ => Err(invalid()),
        }
    }
}

fn tropical_year_days(reference_mjd: f64, scale: &Scale) -> Result<f64> {
    if scale.kind() != Some(ScaleKind::Tdb) {
        return Err(FitsError::ExternalTimeDataRequired {
            operation: "evaluate a tropical year outside the TDB frame",
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
            operation: "evaluate a Besselian year outside the TT/ET frame",
        });
    }
    let centuries = (reference_mjd - 15_019.5) / 36_525.0;
    Ok(365.242_198_781_7 - 0.000_007_854_23 * centuries)
}
