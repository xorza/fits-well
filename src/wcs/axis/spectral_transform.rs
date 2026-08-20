//! [`SpectralTransform`]: how a spectral axis maps between its sampled and
//! expressed characteristics.

use crate::error::Result;
use crate::wcs::axis::spectral_kind::{
    DomainResult, conversion_derivative, convert, domain_error, finite, invalid_grism,
    invalid_reference, required_grism_parameter,
};

use crate::wcs::axis::spectral_kind::{Characteristic, SpectralKind};
use crate::wcs::axis::spectral_rest::ResolvedRest;
use crate::wcs::axis::spectral_rest::SpectralParameters;

#[derive(Debug, Clone)]
pub(crate) struct SpectralTransform {
    kind: SpectralKind,
    sampled: Characteristic,
    rest: ResolvedRest,
    sampling: SpectralSampling,
    algorithm: &'static str,
}

#[derive(Debug, Clone)]
enum SpectralSampling {
    Linear { reference: f64, derivative: f64 },
    Grism(Grism),
}

#[derive(Debug, Clone, Copy)]
struct Grism {
    offset: f64,
    scale: f64,
    beta_reference: f64,
    refractive_term: f64,
    wavelength_scale: f64,
}

impl SpectralTransform {
    pub(super) fn new(
        kind: SpectralKind,
        sampled: Characteristic,
        reference_world: f64,
        rest: ResolvedRest,
        algorithm: &'static str,
        parameters: SpectralParameters,
    ) -> Result<SpectralTransform> {
        let physical = kind
            .to_characteristic(reference_world, rest)
            .map_err(|()| invalid_reference(kind))?;
        let reference = convert(kind.characteristic(), sampled, physical, rest)
            .map_err(|()| invalid_reference(kind))?;
        let derivative = kind.derivative(rest)
            * conversion_derivative(kind.characteristic(), sampled, physical, reference, rest)
                .map_err(|()| invalid_reference(kind))?;
        if !reference.is_finite() || !derivative.is_finite() || derivative == 0.0 {
            return Err(invalid_reference(kind));
        }
        let sampling = if matches!(algorithm, "GRI" | "GRA") {
            SpectralSampling::Grism(Grism::new(reference, derivative, parameters, algorithm)?)
        } else {
            SpectralSampling::Linear {
                reference,
                derivative,
            }
        };
        Ok(SpectralTransform {
            kind,
            sampled,
            rest,
            sampling,
            algorithm,
        })
    }

    pub(super) fn to_world(&self, intermediate: f64, axis: usize) -> Result<f64> {
        let sampled = self
            .sampling
            .to_sampled(intermediate)
            .map_err(|()| domain_error(axis, self.algorithm))?;
        let physical = convert(self.sampled, self.kind.characteristic(), sampled, self.rest)
            .map_err(|()| domain_error(axis, self.algorithm))?;
        self.kind
            .world_from_characteristic(physical, self.rest)
            .and_then(finite)
            .map_err(|()| domain_error(axis, self.algorithm))
    }

    pub(super) fn to_intermediate(&self, world: f64, axis: usize) -> Result<f64> {
        let physical = self
            .kind
            .to_characteristic(world, self.rest)
            .map_err(|()| domain_error(axis, self.algorithm))?;
        let sampled = convert(
            self.kind.characteristic(),
            self.sampled,
            physical,
            self.rest,
        )
        .map_err(|()| domain_error(axis, self.algorithm))?;
        self.sampling
            .to_intermediate(sampled)
            .map_err(|()| domain_error(axis, self.algorithm))
    }
}

impl SpectralSampling {
    fn to_sampled(&self, intermediate: f64) -> DomainResult {
        match self {
            SpectralSampling::Linear {
                reference,
                derivative,
            } => finite(reference + intermediate * derivative),
            SpectralSampling::Grism(grism) => grism.to_sampled(intermediate),
        }
    }

    pub(super) fn to_intermediate(&self, sampled: f64) -> DomainResult {
        match self {
            SpectralSampling::Linear {
                reference,
                derivative,
            } => finite((sampled - reference) / derivative),
            SpectralSampling::Grism(grism) => grism.to_intermediate(sampled),
        }
    }
}

impl Grism {
    pub(super) fn new(
        reference: f64,
        derivative: f64,
        parameters: SpectralParameters,
        algorithm: &'static str,
    ) -> Result<Grism> {
        let [
            density,
            order,
            incidence,
            refractive_index,
            refractive_derivative,
            grating_tilt,
            detector_tilt,
        ] = parameters.values;
        let density = required_grism_parameter(density, algorithm, 0)?;
        let order = required_grism_parameter(order, algorithm, 1)?;
        let incidence = incidence.unwrap_or(0.0);
        let refractive_index = refractive_index.unwrap_or(1.0);
        let refractive_derivative = refractive_derivative.unwrap_or(0.0);
        let grating_tilt = grating_tilt.unwrap_or(0.0);
        let detector_tilt = detector_tilt.unwrap_or(0.0);
        let values = [
            incidence,
            refractive_index,
            refractive_derivative,
            grating_tilt,
            detector_tilt,
        ];
        if values.iter().any(|value| !value.is_finite()) {
            return Err(invalid_grism(algorithm, "parameters must be finite"));
        }

        let incidence_sine = incidence.to_radians().sin();
        let grating_cosine = grating_tilt.to_radians().cos();
        let detector_cosine = detector_tilt.to_radians().cos();
        if grating_cosine == 0.0 || detector_cosine == 0.0 {
            return Err(invalid_grism(
                algorithm,
                "grating and detector tilts must have non-zero cosine",
            ));
        }
        let ruling = density * order / grating_cosine;
        let beta_sine = ruling * reference - refractive_index * incidence_sine;
        if !(-1.0..=1.0).contains(&beta_sine) {
            return Err(invalid_grism(
                algorithm,
                "reference wavelength is outside the grism domain",
            ));
        }
        let beta = beta_sine.asin();
        let dispersion = ruling - refractive_derivative * incidence_sine;
        let scale = derivative * dispersion / (beta.cos() * detector_cosine * detector_cosine);
        if !dispersion.is_finite() || dispersion == 0.0 || !scale.is_finite() || scale == 0.0 {
            return Err(invalid_grism(
                algorithm,
                "detector parameters produce zero dispersion",
            ));
        }
        Ok(Grism {
            offset: -detector_tilt.to_radians().tan(),
            scale,
            beta_reference: beta + detector_tilt.to_radians(),
            refractive_term: (refractive_index - refractive_derivative * reference)
                * incidence_sine,
            wavelength_scale: 1.0 / dispersion,
        })
    }

    fn to_sampled(self, intermediate: f64) -> DomainResult {
        let grism_parameter = finite(self.offset + intermediate * self.scale)?;
        let beta = grism_parameter.atan() + self.beta_reference;
        finite((beta.sin() + self.refractive_term) * self.wavelength_scale)
    }

    pub(super) fn to_intermediate(self, sampled: f64) -> DomainResult {
        let sine = sampled / self.wavelength_scale - self.refractive_term;
        if !sine.is_finite() || !(-1.0..=1.0).contains(&sine) {
            return Err(());
        }
        let grism_parameter = (sine.asin() - self.beta_reference).tan();
        finite((grism_parameter - self.offset) / self.scale)
    }
}
