//! [`TimeScale`]: a `TIMESYS` / `CTYPEi` time-scale declaration, and the
//! [`TimeScaleKind`] it resolves to.

use std::str::FromStr;

use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;

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
/// A standard scale is normalized to its meaning, keeping the realization suffix
/// when one is given (`TT(TAI)`). Any other nonempty code is kept verbatim as a
/// local scale.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TimeScale {
    Known {
        kind: TimeScaleKind,
        realization: Option<String>,
    },
    Local(String),
}

impl TimeScale {
    /// The standard scale `kind`, with no realization.
    pub const fn known(kind: TimeScaleKind) -> TimeScale {
        TimeScale::Known {
            kind,
            realization: None,
        }
    }

    /// The `TIMESYS` scale `header` declares, `UTC` by default.
    pub(crate) fn declared(header: &Header) -> Result<TimeScale> {
        Ok(match header.get_text("TIMESYS")? {
            Some(value) => value.parse()?,
            None => TimeScale::known(TimeScaleKind::Utc),
        })
    }

    pub(crate) fn kind(&self) -> Option<TimeScaleKind> {
        match self {
            TimeScale::Known { kind, .. } => Some(*kind),
            TimeScale::Local(_) => None,
        }
    }
}

impl FromStr for TimeScale {
    type Err = FitsError;

    fn from_str(s: &str) -> Result<TimeScale> {
        let invalid = || FitsError::InvalidTime {
            detail: format!("time scale '{s}'"),
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
            None if !value.is_empty() && !value.contains(')') => (value, None),
            Some(_) | None => return Err(invalid()),
        };
        let kind = match base.to_ascii_uppercase().as_str() {
            "UTC" | "GMT" => TimeScaleKind::Utc,
            "UT1" | "UT" => TimeScaleKind::Ut1,
            "TAI" | "IAT" => TimeScaleKind::Tai,
            "TT" | "TDT" | "ET" => TimeScaleKind::Tt,
            "TCG" => TimeScaleKind::Tcg,
            "TDB" => TimeScaleKind::Tdb,
            "TCB" => TimeScaleKind::Tcb,
            "GPS" => TimeScaleKind::Gps,
            _ if realization.is_some() => return Ok(TimeScale::Local(value.to_string())),
            _ => return Ok(TimeScale::Local(base.to_string())),
        };
        Ok(TimeScale::Known { kind, realization })
    }
}
