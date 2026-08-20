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
        let parts = CTypeParts::parse(ctype);
        let Some(code) = parts.code else {
            return Ok(AxisTransformSpec {
                transform: AxisTransform::Linear,
                unit_scale: 1.0,
            });
        };
        let kind = SpectralKind::from_code(parts.head);
        if code == "LOG" {
            if parts.head.len() != 4 {
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
    SpectralKind::from_code(CTypeParts::parse(ctype).head).is_some()
}

pub(super) fn spectral_unit_scale(ctype: &str, cunit: &str) -> Result<Option<f64>> {
    SpectralKind::from_code(CTypeParts::parse(ctype).head)
        .map(|kind| kind.unit_scale(cunit))
        .transpose()
}

fn unsupported() -> AxisTransformSpec {
    AxisTransformSpec {
        transform: AxisTransform::Unsupported,
        unit_scale: 1.0,
    }
}

#[derive(Debug, Clone, Copy)]
struct CTypeParts<'a> {
    head: &'a str,
    code: Option<&'a str>,
}

impl<'a> CTypeParts<'a> {
    fn parse(ctype: &'a str) -> CTypeParts<'a> {
        let head = ctype.split('-').next().unwrap_or("").trim_end();
        let code = ctype
            .rsplit_once('-')
            .map(|parts| parts.1.trim_end())
            .filter(|code| !code.is_empty());
        CTypeParts { head, code }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::wcs::axis::spectral_kind::{conversion_derivative, convert};
    use crate::wcs::axis::spectral_rest::ResolvedRest;

    /// An in-domain value for each characteristic, all describing roughly the same
    /// 1 µm photon, so every conversion below stays well inside its domain.
    fn sample(characteristic: Characteristic) -> f64 {
        match characteristic {
            Characteristic::Frequency => 3.0e14,
            Characteristic::Wavelength | Characteristic::AirWavelength => 1.0e-6,
            Characteristic::Velocity => 1.0e7,
        }
    }

    /// [`convert`] and [`conversion_derivative`] are two parallel matches over the
    /// same twelve characteristic pairs. Nothing makes the compiler check that an
    /// arm of one agrees with its partner in the other, so a formula edited in one
    /// place and not the other is a silent numerical error — a spectral axis that
    /// still transforms, just wrongly, in one direction.
    ///
    /// Differentiate `convert` numerically and hold `conversion_derivative` to it.
    /// A central difference is second-order accurate, so with a relative step of
    /// 1e-6 the two should agree far inside this tolerance; anything that disagrees
    /// at 1e-5 is a wrong formula, not roundoff.
    #[test]
    fn conversion_derivatives_match_the_conversions_they_describe() {
        let rest = ResolvedRest {
            frequency: 3.0e14,
            wavelength: SPEED_OF_LIGHT / 3.0e14,
        };
        let all = [
            Characteristic::Frequency,
            Characteristic::Wavelength,
            Characteristic::AirWavelength,
            Characteristic::Velocity,
        ];
        let mut checked = 0;
        for from in all {
            for to in all {
                if from == to {
                    continue;
                }
                let x = sample(from);
                let step = x.abs() * 1e-6;
                let ahead = convert(from, to, x + step, rest).unwrap();
                let behind = convert(from, to, x - step, rest).unwrap();
                let numeric = (ahead - behind) / (2.0 * step);
                let at = convert(from, to, x, rest).unwrap();
                let analytic = conversion_derivative(from, to, x, at, rest).unwrap();
                let relative = (analytic - numeric).abs() / numeric.abs();
                assert!(
                    relative < 1e-5,
                    "d({to:?})/d({from:?}) at {x:e}: analytic {analytic:e}, \
                     numeric {numeric:e} (relative error {relative:e})"
                );
                checked += 1;
            }
        }
        // Every ordered pair of distinct characteristics is a real algorithm in
        // Table 26; none may be silently absent from the sweep.
        assert_eq!(checked, 12);
    }
}
