//! The celestial reference frame a WCS declares (`RADESYSa`/`EQUINOXa`, §8.2).

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::wcs::ctype::Ctype;

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
    fn parse(value: &str) -> Result<CelestialReferenceFrame> {
        match value.trim() {
            "ICRS" => Ok(CelestialReferenceFrame::Icrs),
            "FK5" => Ok(CelestialReferenceFrame::Fk5),
            "FK4" => Ok(CelestialReferenceFrame::Fk4),
            "FK4-NO-E" => Ok(CelestialReferenceFrame::Fk4NoE),
            "GAPPT" => Ok(CelestialReferenceFrame::Gappt),
            value => Err(FitsError::InvalidValue {
                card: format!("RADESYS {value:?} is not a standard reference frame"),
            }),
        }
    }
}

impl CelestialFrame {
    /// Resolve `RADESYSa`/`EQUINOXa` (with the legacy unsuffixed `RADECSYS`), or
    /// `None` when neither is declared and no axis needs the frame.
    pub(super) fn from_header(
        header: &Header,
        alt: Option<char>,
        suffix: &str,
        ctype: &[String],
    ) -> Result<Option<CelestialFrame>> {
        let key = key!("RADESYS{suffix}");
        let mut declared = header.get_text(key.as_str())?;
        if declared.is_none() && alt.is_none() {
            declared = header.get_text("RADECSYS")?;
        }
        let equinox = header.get_real(key!("EQUINOX{suffix}").as_str())?;
        if equinox.is_some_and(|value| !value.is_finite() || value < 0.0) {
            return Err(FitsError::InvalidValue {
                card: "EQUINOX must be finite and non-negative".to_string(),
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
        Ok(Some(CelestialFrame {
            reference_frame,
            equinox,
        }))
    }
}
