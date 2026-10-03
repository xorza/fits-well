//! [`SpectralRest`]: the rest frequency or wavelength a spectral axis is
//! measured against, and the [`SpectralParameters`] it is read from.

use crate::error::FitsError;
use crate::error::Result;
use crate::world_coordinates::axis::SPEED_OF_LIGHT;
use crate::world_coordinates::axis::spectral_kind::{Characteristic, SpectralKind};

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpectralParameters {
    pub(super) values: [Option<f64>; 7],
}

impl SpectralParameters {
    pub(crate) const fn new(values: [Option<f64>; 7]) -> SpectralParameters {
        SpectralParameters { values }
    }
}

/// The rest frequency and wavelength a spectral axis declares (`RESTFRQa`,
/// `RESTWAVa`), each finite and positive when given.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SpectralRest {
    /// `RESTFRQa`, in Hz.
    pub frequency: Option<f64>,
    /// `RESTWAVa`, in metres.
    pub wavelength: Option<f64>,
}

impl SpectralRest {
    pub(crate) const NONE: SpectralRest = SpectralRest {
        frequency: None,
        wavelength: None,
    };

    pub(crate) fn new(frequency: Option<f64>, wavelength: Option<f64>) -> Result<SpectralRest> {
        for (name, value) in [("RESTFRQ", frequency), ("RESTWAV", wavelength)] {
            if value.is_some_and(|value| !value.is_finite() || value <= 0.0) {
                return Err(FitsError::InvalidWcs {
                    detail: format!("{name} must be finite and positive"),
                });
            }
        }
        Ok(SpectralRest {
            frequency,
            wavelength,
        })
    }

    pub(super) fn resolve(self, need: RestNeed) -> Result<ResolvedRest> {
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
        match need {
            // Unused: any value would do, and zero keeps an accidental use visible.
            RestNeed::Unused => Ok(ResolvedRest {
                frequency: 0.0,
                wavelength: 0.0,
            }),
            RestNeed::Required => Err(FitsError::InvalidWcs {
                detail: "spectral CTYPE requires RESTFRQ or RESTWAV".to_string(),
            }),
            RestNeed::Cancels => Ok(ResolvedRest {
                frequency: SPEED_OF_LIGHT,
                wavelength: 1.0,
            }),
        }
    }
}

/// Whether a spectral axis needs a rest value — wcslib's `restreq`, whose two bits
/// say that the coordinate type is a velocity or redshift measured against a rest
/// value, and that the algorithm converts between velocity and another
/// characteristic.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RestNeed {
    /// Neither needs one.
    Unused,
    /// Exactly one needs it, so the header must give `RESTFRQ` or `RESTWAV`.
    Required,
    /// Both need it, and the two uses cancel: any rest value gives the same
    /// coordinates, so a missing one is not an error (wcslib `restreq == 3`).
    Cancels,
}

impl RestNeed {
    pub(super) fn of(kind: SpectralKind, sampled: Characteristic) -> RestNeed {
        let by_kind = matches!(
            kind,
            SpectralKind::RadioVelocity | SpectralKind::OpticalVelocity | SpectralKind::Redshift
        );
        let expressed = kind.characteristic();
        let by_algorithm =
            (expressed == Characteristic::Velocity) != (sampled == Characteristic::Velocity);
        match (by_kind, by_algorithm) {
            (false, false) => RestNeed::Unused,
            (true, true) => RestNeed::Cancels,
            _ => RestNeed::Required,
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ResolvedRest {
    pub(super) frequency: f64,
    pub(super) wavelength: f64,
}
