//! [`AxisTransform`]: the non-linear WCS axis algorithms (§8), which for this
//! crate means the spectral family — the `CTYPEi` codes that pair a sampled
//! characteristic with an expressed one, and the conversions between them.

pub(super) mod spectral_kind;
pub(super) mod spectral_rest;
pub(super) mod spectral_transform;

use crate::error::FitsError;
use crate::error::Result;
use crate::wcs::axis::spectral_kind::{
    Characteristic, SpectralKind, algorithm_name, domain_error, finite, is_spectral_pair_syntax,
    rest_requirement,
};
use crate::wcs::axis::spectral_rest::{SpectralParameters, SpectralRest};
use crate::wcs::axis::spectral_transform::SpectralTransform;
use crate::wcs::ctype::Ctype;

const SPEED_OF_LIGHT: f64 = 2.997_924_58e8;
// wcslib uses the historical WCS-paper value rather than the modern exact SI value.
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
    pub(super) unit_scale: f64,
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
                return Err(FitsError::InvalidValue {
                    card: format!("{ctype} requires a finite, positive CRVAL"),
                });
            }
            return Ok(AxisTransformSpec {
                transform: AxisTransform::Logarithmic,
                unit_scale,
            });
        }
        let Some(kind) = kind else {
            return Ok(unsupported());
        };
        let sampled = match code {
            "GRI" => Some(Characteristic::Wavelength),
            "GRA" => Some(Characteristic::AirWavelength),
            _ => Characteristic::from_algorithm(code, kind.characteristic()),
        };
        let Some(sampled) = sampled else {
            if is_spectral_pair_syntax(code) {
                return Err(FitsError::InvalidValue {
                    card: format!("spectral CTYPE {ctype:?} has inconsistent variables"),
                });
            }
            return Ok(unsupported());
        };
        let unit_scale = kind.unit_scale(cunit)?;
        let requirement = rest_requirement(kind.characteristic(), sampled, kind);
        let rest = rest.resolve(requirement)?;
        let algorithm = match code {
            "GRI" => "GRI",
            "GRA" => "GRA",
            _ => algorithm_name(sampled, kind.characteristic()),
        };
        let transform = SpectralTransform::new(
            kind,
            sampled,
            reference * unit_scale,
            rest,
            algorithm,
            parameters,
        )?;
        Ok(AxisTransformSpec {
            transform: AxisTransform::Spectral(transform),
            unit_scale,
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

pub(super) fn spectral_unit_scale(ctype: &str, cunit: &str) -> Result<Option<f64>> {
    SpectralKind::from_code(Ctype::parse(ctype).head)
        .map(|kind| kind.unit_scale(cunit))
        .transpose()
}

fn unsupported() -> AxisTransformSpec {
    AxisTransformSpec {
        transform: AxisTransform::Unsupported,
        unit_scale: 1.0,
    }
}

#[cfg(test)]
mod tests;
