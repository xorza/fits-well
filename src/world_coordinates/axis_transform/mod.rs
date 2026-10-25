//! [`AxisTransform`]: the non-linear WCS axis algorithms (§8), which for this
//! crate means the spectral family — the `CTYPEi` codes that pair a sampled
//! characteristic with an expressed one, and the conversions between them.

pub(super) mod spectral_algorithm;
pub(crate) mod spectral_kind;
pub(crate) mod spectral_rest;
pub(super) mod spectral_transform;

use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::WcsDefect;
use crate::world_coordinates::axis_transform::spectral_algorithm::SpectralAlgorithm;
use crate::world_coordinates::axis_transform::spectral_kind::{SpectralKind, domain_error, finite};
use crate::world_coordinates::axis_transform::spectral_rest::{
    RestNeed, SpectralParameters, SpectralRest,
};
use crate::world_coordinates::axis_transform::spectral_transform::SpectralTransform;
use crate::world_coordinates::ctype::Ctype;
use crate::world_coordinates::world_unit::WorldUnit;

/// An axis's own step between its intermediate and its world coordinate, with the
/// reference value `CRVALi` it adds, in its world unit.
#[derive(Debug, Clone)]
pub(super) enum AxisTransform {
    Linear {
        reference: f64,
    },
    /// An axis the celestial or the tabular transform evaluates with its partners: the
    /// per-axis step passes its value through to that joint step.
    Joint,
    Logarithmic {
        reference: f64,
    },
    Spectral(SpectralTransform),
    Unsupported,
}

#[derive(Debug)]
pub(super) struct AxisTransformSpec {
    pub(super) transform: AxisTransform,
    /// The unit of the axis's world coordinates, from its declared `CUNITi`.
    pub(super) unit: WorldUnit,
}

impl AxisTransform {
    pub(super) fn parse(
        parsed: Ctype<'_>,
        cunit: &str,
        reference: f64,
        rest: SpectralRest,
        parameters: SpectralParameters,
    ) -> Result<AxisTransformSpec> {
        let Some(code) = parsed.algorithm else {
            return Ok(AxisTransformSpec {
                transform: Self::Linear { reference },
                unit: WorldUnit::DECLARED,
            });
        };
        let kind = SpectralKind::from_code(parsed.head);
        if code == "LOG" {
            if parsed.head.len() != 4 {
                return Ok(unsupported());
            }
            let unit = WorldUnit::of(parsed, cunit)?;
            let reference = reference * unit.scale;
            if !reference.is_finite() || reference <= 0.0 {
                return Err(FitsError::InvalidWcs {
                    defect: WcsDefect::LogarithmicReference {
                        ctype: parsed.value.to_string(),
                    },
                });
            }
            return Ok(AxisTransformSpec {
                transform: Self::Logarithmic { reference },
                unit,
            });
        }
        let Some(kind) = kind else {
            return Ok(unsupported());
        };
        let Some(algorithm) = SpectralAlgorithm::parse(code) else {
            return Ok(unsupported());
        };
        // An `X2P` code must express the coordinate type's own characteristic, and
        // convert between two distinct ones.
        if let SpectralAlgorithm::Pair { sampled, expressed } = algorithm
            && (expressed != kind.characteristic() || sampled == expressed)
        {
            return Err(FitsError::InvalidWcs {
                defect: WcsDefect::InconsistentSpectralAlgorithm {
                    ctype: parsed.value.to_string(),
                },
            });
        }
        let unit = WorldUnit::of(parsed, cunit)?;
        let rest = rest.resolve(RestNeed::of(kind, algorithm.sampled()))?;
        let transform =
            SpectralTransform::new(kind, algorithm, reference * unit.scale, rest, parameters)?;
        Ok(AxisTransformSpec {
            transform: Self::Spectral(transform),
            unit,
        })
    }

    pub(super) fn to_world(&self, intermediate: f64, axis: usize) -> Result<f64> {
        match *self {
            Self::Linear { reference } => Ok(reference + intermediate),
            Self::Joint => Ok(intermediate),
            Self::Logarithmic { reference } => finite(reference * (intermediate / reference).exp())
                .ok_or_else(|| domain_error(axis, "LOG")),
            Self::Spectral(ref transform) => transform.to_world(intermediate, axis),
            Self::Unsupported => {
                panic!("unsupported WCS transform passed completeness check")
            }
        }
    }

    pub(super) fn to_intermediate(&self, world: f64, axis: usize) -> Result<f64> {
        match *self {
            Self::Linear { reference } => Ok(world - reference),
            Self::Joint => Ok(world),
            Self::Logarithmic { reference } if world.is_finite() && world > 0.0 => {
                Ok(reference * (world / reference).ln())
            }
            Self::Logarithmic { .. } => Err(domain_error(axis, "LOG")),
            Self::Spectral(ref transform) => transform.to_intermediate(world, axis),
            Self::Unsupported => {
                panic!("unsupported WCS transform passed completeness check")
            }
        }
    }
}

pub(super) fn is_spectral_type(ctype: Ctype<'_>) -> bool {
    SpectralKind::from_code(ctype.head).is_some()
}

const fn unsupported() -> AxisTransformSpec {
    AxisTransformSpec {
        transform: AxisTransform::Unsupported,
        unit: WorldUnit::DECLARED,
    }
}

#[cfg(test)]
mod tests;
