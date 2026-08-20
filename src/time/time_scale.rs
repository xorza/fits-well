//! [`TimeScale`]: a `TIMESYS` / `CTYPEi` time-scale declaration, and the
//! [`TimeScaleKind`] it resolves to.

use std::str::FromStr;

use crate::error::FitsError;
use crate::error::Result;

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
    pub(super) fn kind(&self) -> Option<TimeScaleKind> {
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

    pub(super) fn is_utc(&self) -> bool {
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
