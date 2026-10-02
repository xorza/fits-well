//! [`AxisTransform`]: the non-linear WCS axis algorithms (§8), which for this
//! crate means the spectral family — the `CTYPEi` codes that pair a sampled
//! characteristic with an expressed one, and the conversions between them.

pub(super) mod spectral_algorithm;
pub(super) mod spectral_kind;
pub(crate) mod spectral_rest;
pub(super) mod spectral_transform;

use crate::error::FitsError;
use crate::error::Result;
use crate::world_coordinates::axis::spectral_algorithm::SpectralAlgorithm;
use crate::world_coordinates::axis::spectral_kind::{SpectralKind, domain_error, finite};
use crate::world_coordinates::axis::spectral_rest::{RestNeed, SpectralParameters, SpectralRest};
use crate::world_coordinates::axis::spectral_transform::SpectralTransform;
use crate::world_coordinates::ctype::Ctype;

pub(super) const SPEED_OF_LIGHT: f64 = 2.997_924_58e8;
/// wcslib's value (CODATA 1986), not the exact SI 6.626 070 15e-34, so energies
/// match wcslib's.
const PLANCK_CONSTANT: f64 = 6.626_075_5e-34;

#[derive(Debug, Clone)]
pub(super) enum AxisTransform {
    Linear,
    Logarithmic,
    Spectral(SpectralTransform),
    Unsupported,
}

#[derive(Debug)]
pub(super) struct AxisTransformSpec {
    pub(super) transform: AxisTransform,
    /// The factor from the declared `CUNITi` to [`Self::world_unit`].
    pub(super) unit_scale: f64,
    /// The unit world coordinates of the axis come out in, when it is not the declared
    /// one: a spectral quantity's Table-25 default unit.
    pub(super) world_unit: Option<&'static str>,
}

impl AxisTransform {
    pub(super) fn parse(
        ctype: &str,
        cunit: &str,
        reference: f64,
        rest: SpectralRest,
        parameters: SpectralParameters,
    ) -> Result<AxisTransformSpec> {
        let parsed = Ctype::parse(ctype);
        let Some(code) = parsed.algorithm else {
            return Ok(AxisTransformSpec {
                transform: AxisTransform::Linear,
                unit_scale: 1.0,
                world_unit: None,
            });
        };
        let kind = SpectralKind::from_code(parsed.head);
        if code == "LOG" {
            if parsed.head.len() != 4 {
                return Ok(unsupported());
            }
            let unit_scale = kind
                .map(|kind| kind.unit_scale(cunit))
                .transpose()?
                .unwrap_or(1.0);
            if !reference.is_finite() || reference * unit_scale <= 0.0 {
                return Err(FitsError::InvalidWcs {
                    detail: format!("{ctype} requires a finite, positive CRVAL"),
                });
            }
            return Ok(AxisTransformSpec {
                transform: AxisTransform::Logarithmic,
                unit_scale,
                world_unit: kind.map(SpectralKind::default_unit),
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
                detail: format!("spectral CTYPE {ctype:?} has inconsistent variables"),
            });
        }
        let unit_scale = kind.unit_scale(cunit)?;
        let rest = rest.resolve(RestNeed::of(kind, algorithm.sampled()))?;
        let transform =
            SpectralTransform::new(kind, algorithm, reference * unit_scale, rest, parameters)?;
        Ok(AxisTransformSpec {
            transform: AxisTransform::Spectral(transform),
            unit_scale,
            world_unit: Some(kind.default_unit()),
        })
    }

    pub(super) fn to_world(&self, intermediate: f64, reference: f64, axis: usize) -> Result<f64> {
        match self {
            AxisTransform::Linear => Ok(reference + intermediate),
            AxisTransform::Logarithmic => finite(reference * (intermediate / reference).exp())
                .map_err(|()| domain_error(axis, "LOG")),
            AxisTransform::Spectral(transform) => transform.to_world(intermediate, axis),
            AxisTransform::Unsupported => {
                panic!("unsupported WCS transform passed completeness check")
            }
        }
    }

    pub(super) fn to_intermediate(&self, world: f64, reference: f64, axis: usize) -> Result<f64> {
        match self {
            AxisTransform::Linear => Ok(world - reference),
            AxisTransform::Logarithmic if world.is_finite() && world > 0.0 => {
                Ok(reference * (world / reference).ln())
            }
            AxisTransform::Logarithmic => Err(domain_error(axis, "LOG")),
            AxisTransform::Spectral(transform) => transform.to_intermediate(world, axis),
            AxisTransform::Unsupported => {
                panic!("unsupported WCS transform passed completeness check")
            }
        }
    }
}

pub(super) fn is_spectral_type(ctype: &str) -> bool {
    SpectralKind::from_code(Ctype::parse(ctype).head).is_some()
}

/// The factor from `cunit` to a spectral axis's Table-25 default unit, or `None`
/// when `head` names no spectral type.
pub(super) fn spectral_unit_scale(head: &str, cunit: &str) -> Result<Option<f64>> {
    SpectralKind::from_code(head)
        .map(|kind| kind.unit_scale(cunit))
        .transpose()
}

fn unsupported() -> AxisTransformSpec {
    AxisTransformSpec {
        transform: AxisTransform::Unsupported,
        unit_scale: 1.0,
        world_unit: None,
    }
}

#[cfg(test)]
mod tests;
