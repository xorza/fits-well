//! [`SpectralRest`]: the rest frequency or wavelength a spectral axis is
//! measured against, and the [`SpectralParameters`] it is read from.

use crate::error::FitsError;
use crate::error::Result;
use crate::wcs::axis::SPEED_OF_LIGHT;

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpectralParameters {
    pub(super) values: [Option<f64>; 7],
}

impl SpectralParameters {
    pub(crate) fn new(values: [Option<f64>; 7]) -> SpectralParameters {
        SpectralParameters { values }
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpectralRest {
    pub(crate) frequency: Option<f64>,
    pub(crate) wavelength: Option<f64>,
}

impl SpectralRest {
    pub(crate) const NONE: SpectralRest = SpectralRest {
        frequency: None,
        wavelength: None,
    };

    pub(crate) fn new(frequency: Option<f64>, wavelength: Option<f64>) -> Result<SpectralRest> {
        for (name, value) in [("RESTFRQ", frequency), ("RESTWAV", wavelength)] {
            if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
                return Err(FitsError::InvalidValue {
                    card: format!("{name} must be finite and positive"),
                });
            }
        }
        Ok(SpectralRest {
            frequency,
            wavelength,
        })
    }

    pub(super) fn resolve(self, requirement: u8) -> Result<ResolvedRest> {
        let supplied = match (self.frequency, self.wavelength) {
            (Some(frequency), _) => Some(ResolvedRest {
                frequency,
                wavelength: SPEED_OF_LIGHT / frequency,
            }),
            (None, Some(wavelength)) => Some(ResolvedRest {
                frequency: SPEED_OF_LIGHT / wavelength,
                wavelength,
            }),
            (None, None) => None,
        };
        if let Some(rest) = supplied {
            return Ok(rest);
        }
        if requirement == 3 {
            return Ok(ResolvedRest {
                frequency: SPEED_OF_LIGHT,
                wavelength: 1.0,
            });
        }
        if !requirement.is_multiple_of(3) {
            return Err(FitsError::InvalidValue {
                card: "spectral CTYPE requires RESTFRQ or RESTWAV".to_string(),
            });
        }
        Ok(ResolvedRest {
            frequency: 0.0,
            wavelength: 0.0,
        })
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResolvedRest {
    pub(super) frequency: f64,
    pub(super) wavelength: f64,
}
