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
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CelestialReferenceFrame {
    Icrs,
    Fk5,
    Fk4,
    Fk4NoE,
    Gappt,
    /// A frame this crate does not model, as the header names it, blanks trimmed: the
    /// celestial axes read as they would in a frame it does.
    Other(String),
}

/// Resolved celestial-frame metadata for an equatorial/ecliptic WCS.
#[derive(Debug, Clone, PartialEq)]
pub struct CelestialFrame {
    /// `RADESYSa`, including the standard default selected from `EQUINOXa`.
    pub reference_frame: CelestialReferenceFrame,
    /// The declared non-negative `EQUINOXa`, or `None` when omitted.
    pub equinox: Option<f64>,
}

impl CelestialReferenceFrame {
    fn parse(value: &str) -> Self {
        match value.trim() {
            "ICRS" => Self::Icrs,
            "FK5" => Self::Fk5,
            "FK4" => Self::Fk4,
            "FK4-NO-E" => Self::Fk4NoE,
            "GAPPT" => Self::Gappt,
            value => Self::Other(value.to_string()),
        }
    }
}

impl CelestialFrame {
    /// Resolve `RADESYSa`/`EQUINOXa` (with the legacy unsuffixed `RADECSYS`), or
    /// `None` when neither is declared and no axis needs the frame.
    pub(super) fn from_header(
        header: &Header,
        alt: Alternate,
        ctype: &[Ctype<'_>],
    ) -> Result<Option<Self>> {
        let mut declared =
            header_read::text(header, Keyword::fitting(format_args!("RADESYS{alt}")))?;
        if declared.is_none() && !alt.is_alternate() {
            declared = header.get_text(keywords::RADECSYS)?;
        }
        let mut equinox =
            header_read::real(header, Keyword::fitting(format_args!("EQUINOX{alt}")))?;
        // §8.3: `EPOCH`, primary only, is the deprecated name of `EQUINOX`, read in its
        // absence, as wcslib's `wcspih` reads it.
        if equinox.is_none() && !alt.is_alternate() {
            equinox = header_read::real(header, keywords::EPOCH)?;
        }
        if equinox.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(FitsError::InvalidWcs {
                defect: WcsDefect::InvalidEquinox,
            });
        }
        let applies = ctype
            .iter()
            .any(|ctype| matches!(ctype.head, "RA" | "DEC" | "ELON" | "ELAT"));
        if !applies && declared.is_none() && equinox.is_none() {
            return Ok(None);
        }
        let reference_frame = match declared {
            Some(value) => CelestialReferenceFrame::parse(value),
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
