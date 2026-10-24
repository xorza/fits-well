//! The celestial reference frame a WCS declares (`RADESYSa`/`EQUINOXa`, §8.2).

use crate::alternate::Alternate;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::WcsDefect;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::header_read;

/// The reference frame named by `RADESYSa`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CelestialReferenceFrame {
    Icrs,
    Fk5,
    Fk4,
    Fk4NoE,
    Gappt,
}

/// Resolved celestial-frame metadata for an equatorial/ecliptic WCS.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CelestialFrame {
    /// `RADESYSa`, including the standard default selected from `EQUINOXa`.
    pub reference_frame: CelestialReferenceFrame,
    /// The declared non-negative `EQUINOXa`, or `None` when omitted.
    pub equinox: Option<f64>,
}

impl CelestialReferenceFrame {
    fn parse(value: &str) -> Result<Self> {
        match value.trim() {
            "ICRS" => Ok(Self::Icrs),
            "FK5" => Ok(Self::Fk5),
            "FK4" => Ok(Self::Fk4),
            "FK4-NO-E" => Ok(Self::Fk4NoE),
            "GAPPT" => Ok(Self::Gappt),
            value => Err(FitsError::InvalidWcs {
                defect: WcsDefect::UnknownCelestialFrame {
                    value: value.to_string(),
                },
            }),
        }
    }
}

impl CelestialFrame {
    /// Resolve `RADESYSa`/`EQUINOXa` (with the legacy unsuffixed `RADECSYS`), or
    /// `None` when neither is declared and no axis needs the frame.
    pub(super) fn from_header(
        header: &Header,
        alt: Alternate,
        ctype: &[String],
    ) -> Result<Option<Self>> {
        let mut declared =
            header_read::text(header, Keyword::fitting(format_args!("RADESYS{alt}")))?;
        if declared.is_none() && !alt.is_alternate() {
            declared = header.get_text(keywords::RADECSYS)?;
        }
        let equinox = header_read::real(header, Keyword::fitting(format_args!("EQUINOX{alt}")))?;
        if equinox.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(FitsError::InvalidWcs {
                defect: WcsDefect::InvalidEquinox,
            });
        }
        let applies = ctype
            .iter()
            .any(|ctype| matches!(Ctype::parse(ctype).head, "RA" | "DEC" | "ELON" | "ELAT"));
        if !applies && declared.is_none() && equinox.is_none() {
            return Ok(None);
        }
        let reference_frame = match declared {
            Some(value) => CelestialReferenceFrame::parse(value)?,
            None => match equinox {
                Some(value) if value < 1984.0 => CelestialReferenceFrame::Fk4,
                Some(_) => CelestialReferenceFrame::Fk5,
                None => CelestialReferenceFrame::Icrs,
            },
        };
        Ok(Some(Self {
            reference_frame,
            equinox,
        }))
    }
}

#[cfg(test)]
mod tests;
