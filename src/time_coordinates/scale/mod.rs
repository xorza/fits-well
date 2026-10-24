//! [`Scale`]: a `TIMESYS` / `CTYPEi` time-scale declaration, and the
//! [`ScaleKind`] it resolves to.

use std::fmt;
use std::str::FromStr;

use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};

/// A recognized FITS time-scale meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScaleKind {
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
pub enum Scale {
    Known {
        kind: ScaleKind,
        realization: Option<String>,
    },
    Local(String),
}

impl Scale {
    /// The standard scale `kind`, with no realization.
    pub const fn known(kind: ScaleKind) -> Self {
        Self::Known {
            kind,
            realization: None,
        }
    }

    /// The `TIMESYS` scale `header` declares, `UTC` by default.
    pub(crate) fn declared(header: &Header) -> Result<Self> {
        Ok(match header.get_text(keywords::TIMESYS)? {
            Some(value) => value.parse().map_err(|_| FitsError::InvalidTimeScale {
                keyword: Some(keywords::TIMESYS),
            })?,
            None => Self::known(ScaleKind::Utc),
        })
    }

    pub(crate) const fn kind(&self) -> Option<ScaleKind> {
        match self {
            Self::Known { kind, .. } => Some(*kind),
            Self::Local(_) => None,
        }
    }
}

impl FromStr for Scale {
    type Err = FitsError;

    fn from_str(s: &str) -> Result<Self> {
        let invalid = || FitsError::InvalidTimeScale { keyword: None };
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
            "UTC" | "GMT" => ScaleKind::Utc,
            "UT1" | "UT" => ScaleKind::Ut1,
            "TAI" | "IAT" => ScaleKind::Tai,
            "TT" | "TDT" | "ET" => ScaleKind::Tt,
            "TCG" => ScaleKind::Tcg,
            "TDB" => ScaleKind::Tdb,
            "TCB" => ScaleKind::Tcb,
            "GPS" => ScaleKind::Gps,
            _ if realization.is_some() => return Ok(Self::Local(value.to_string())),
            _ => return Ok(Self::Local(base.to_string())),
        };
        Ok(Self::Known { kind, realization })
    }
}

/// The standard code of the scale, as `TIMESYS` spells it: `TT` for `TDT` and `ET`
/// alike.
impl fmt::Display for ScaleKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Utc => "UTC",
            Self::Ut1 => "UT1",
            Self::Tai => "TAI",
            Self::Tt => "TT",
            Self::Tcg => "TCG",
            Self::Tdb => "TDB",
            Self::Tcb => "TCB",
            Self::Gps => "GPS",
        })
    }
}

/// A standard scale as its code and its realization, `TT(TAI)`, and a local one as it
/// was read: text that parses back to the same scale.
impl fmt::Display for Scale {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Known {
                kind,
                realization: None,
            } => fmt::Display::fmt(kind, f),
            Self::Known {
                kind,
                realization: Some(realization),
            } => write!(f, "{kind}({realization})"),
            Self::Local(code) => f.write_str(code),
        }
    }
}

#[cfg(test)]
mod tests;
