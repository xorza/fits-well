use crate::error::FitsError;
use crate::error::Result;
use crate::unit;

const SPEED_OF_LIGHT: f64 = 2.997_924_58e8;
// wcslib uses the historical WCS-paper value rather than the modern exact SI value.
const PLANCK_CONSTANT: f64 = 6.626_075_5e-34;

#[derive(Debug, Clone)]
pub(crate) enum AxisTransform {
    Linear,
    Logarithmic,
    Spectral(SpectralTransform),
    Unsupported,
}

#[derive(Debug)]
pub(crate) struct AxisTransformSpec {
    pub(crate) transform: AxisTransform,
    pub(crate) unit_scale: f64,
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct SpectralRest {
    frequency: Option<f64>,
    wavelength: Option<f64>,
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

    fn resolve(self, requirement: u8) -> Result<ResolvedRest> {
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

#[derive(Debug, Clone)]
pub(crate) struct SpectralTransform {
    kind: SpectralKind,
    sampled: Characteristic,
    rest: ResolvedRest,
    reference: f64,
    derivative: f64,
    algorithm: &'static str,
}

impl SpectralTransform {
    fn new(
        kind: SpectralKind,
        sampled: Characteristic,
        reference_world: f64,
        rest: ResolvedRest,
        algorithm: &'static str,
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
        Ok(SpectralTransform {
            kind,
            sampled,
            rest,
            reference,
            derivative,
            algorithm,
        })
    }

    fn to_world(&self, intermediate: f64, axis: usize) -> Result<f64> {
        let sampled = self.reference + intermediate * self.derivative;
        let physical = convert(self.sampled, self.kind.characteristic(), sampled, self.rest)
            .map_err(|()| domain_error(axis, self.algorithm))?;
        self.kind
            .world_from_characteristic(physical, self.rest)
            .and_then(finite)
            .map_err(|()| domain_error(axis, self.algorithm))
    }

    fn to_intermediate(&self, world: f64, axis: usize) -> Result<f64> {
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
        finite((sampled - self.reference) / self.derivative)
            .map_err(|()| domain_error(axis, self.algorithm))
    }
}

impl AxisTransform {
    pub(crate) fn parse(
        ctype: &str,
        cunit: &str,
        reference: f64,
        rest: SpectralRest,
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
        let Some(sampled) = Characteristic::from_algorithm(code, kind.characteristic()) else {
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
        let algorithm = algorithm_name(sampled, kind.characteristic());
        let transform =
            SpectralTransform::new(kind, sampled, reference * unit_scale, rest, algorithm)?;
        Ok(AxisTransformSpec {
            transform: AxisTransform::Spectral(transform),
            unit_scale,
        })
    }

    pub(crate) fn to_world(&self, intermediate: f64, reference: f64, axis: usize) -> Result<f64> {
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

    pub(crate) fn to_intermediate(&self, world: f64, reference: f64, axis: usize) -> Result<f64> {
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

pub(crate) fn has_analytic_spectral_algorithm(ctype: &str) -> bool {
    let parts = CTypeParts::parse(ctype);
    let Some(kind) = SpectralKind::from_code(parts.head) else {
        return false;
    };
    parts
        .code
        .is_some_and(|code| Characteristic::from_algorithm(code, kind.characteristic()).is_some())
}

pub(crate) fn is_spectral_type(ctype: &str) -> bool {
    SpectralKind::from_code(CTypeParts::parse(ctype).head).is_some()
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpectralKind {
    Frequency,
    Energy,
    Wavenumber,
    RadioVelocity,
    Wavelength,
    OpticalVelocity,
    Redshift,
    AirWavelength,
    RelativisticVelocity,
    Beta,
}

impl SpectralKind {
    fn from_code(code: &str) -> Option<SpectralKind> {
        match code {
            "FREQ" => Some(SpectralKind::Frequency),
            "ENER" => Some(SpectralKind::Energy),
            "WAVN" => Some(SpectralKind::Wavenumber),
            "VRAD" => Some(SpectralKind::RadioVelocity),
            "WAVE" => Some(SpectralKind::Wavelength),
            "VOPT" => Some(SpectralKind::OpticalVelocity),
            "ZOPT" => Some(SpectralKind::Redshift),
            "AWAV" => Some(SpectralKind::AirWavelength),
            "VELO" => Some(SpectralKind::RelativisticVelocity),
            "BETA" => Some(SpectralKind::Beta),
            _ => None,
        }
    }

    fn code(self) -> &'static str {
        match self {
            SpectralKind::Frequency => "FREQ",
            SpectralKind::Energy => "ENER",
            SpectralKind::Wavenumber => "WAVN",
            SpectralKind::RadioVelocity => "VRAD",
            SpectralKind::Wavelength => "WAVE",
            SpectralKind::OpticalVelocity => "VOPT",
            SpectralKind::Redshift => "ZOPT",
            SpectralKind::AirWavelength => "AWAV",
            SpectralKind::RelativisticVelocity => "VELO",
            SpectralKind::Beta => "BETA",
        }
    }

    fn characteristic(self) -> Characteristic {
        match self {
            SpectralKind::Frequency
            | SpectralKind::Energy
            | SpectralKind::Wavenumber
            | SpectralKind::RadioVelocity => Characteristic::Frequency,
            SpectralKind::Wavelength | SpectralKind::OpticalVelocity | SpectralKind::Redshift => {
                Characteristic::Wavelength
            }
            SpectralKind::AirWavelength => Characteristic::AirWavelength,
            SpectralKind::RelativisticVelocity | SpectralKind::Beta => Characteristic::Velocity,
        }
    }

    fn to_characteristic(self, value: f64, rest: ResolvedRest) -> DomainResult {
        if !value.is_finite() {
            return Err(());
        }
        let result = match self {
            SpectralKind::Frequency => nonzero(value)?,
            SpectralKind::Energy => nonzero(value)? / PLANCK_CONSTANT,
            SpectralKind::Wavenumber => nonzero(value)? * SPEED_OF_LIGHT,
            SpectralKind::RadioVelocity => {
                assert!(rest.frequency != 0.0, "resolved rest frequency");
                rest.frequency * (1.0 - value / SPEED_OF_LIGHT)
            }
            SpectralKind::Wavelength => nonzero(value)?,
            SpectralKind::OpticalVelocity => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                nonzero(rest.wavelength * (1.0 + value / SPEED_OF_LIGHT))?
            }
            SpectralKind::Redshift => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                nonzero(rest.wavelength * (1.0 + value))?
            }
            SpectralKind::AirWavelength => nonzero(value)?,
            SpectralKind::RelativisticVelocity => subluminal(value)?,
            SpectralKind::Beta => subluminal(value * SPEED_OF_LIGHT)?,
        };
        finite(result)
    }

    fn world_from_characteristic(self, value: f64, rest: ResolvedRest) -> DomainResult {
        let value = finite(value)?;
        let result = match self {
            SpectralKind::Frequency => value,
            SpectralKind::Energy => value * PLANCK_CONSTANT,
            SpectralKind::Wavenumber => value / SPEED_OF_LIGHT,
            SpectralKind::RadioVelocity => {
                assert!(rest.frequency != 0.0, "resolved rest frequency");
                SPEED_OF_LIGHT * (1.0 - value / rest.frequency)
            }
            SpectralKind::Wavelength => value,
            SpectralKind::OpticalVelocity => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                SPEED_OF_LIGHT * (value / rest.wavelength - 1.0)
            }
            SpectralKind::Redshift => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                value / rest.wavelength - 1.0
            }
            SpectralKind::AirWavelength => value,
            SpectralKind::RelativisticVelocity => value,
            SpectralKind::Beta => value / SPEED_OF_LIGHT,
        };
        finite(result)
    }

    fn derivative(self, rest: ResolvedRest) -> f64 {
        match self {
            SpectralKind::Frequency
            | SpectralKind::Wavelength
            | SpectralKind::AirWavelength
            | SpectralKind::RelativisticVelocity => 1.0,
            SpectralKind::Energy => 1.0 / PLANCK_CONSTANT,
            SpectralKind::Wavenumber => SPEED_OF_LIGHT,
            SpectralKind::RadioVelocity => {
                assert!(rest.frequency != 0.0, "resolved rest frequency");
                -rest.frequency / SPEED_OF_LIGHT
            }
            SpectralKind::OpticalVelocity => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                rest.wavelength / SPEED_OF_LIGHT
            }
            SpectralKind::Redshift => {
                assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                rest.wavelength
            }
            SpectralKind::Beta => SPEED_OF_LIGHT,
        }
    }

    fn unit_scale(self, unit: &str) -> Result<f64> {
        let scaled =
            unit::split_numeric_multiplier(unit).ok_or_else(|| FitsError::InvalidValue {
                card: format!("invalid CUNIT {unit:?} for {}", self.code()),
            })?;
        let unit = scaled.base;
        let scale = match self {
            SpectralKind::Frequency => prefixed_unit(unit, "Hz", 1.0),
            SpectralKind::Energy => energy_scale(unit),
            SpectralKind::Wavenumber => wavenumber_scale(unit),
            SpectralKind::RadioVelocity
            | SpectralKind::OpticalVelocity
            | SpectralKind::RelativisticVelocity => velocity_scale(unit),
            SpectralKind::Wavelength | SpectralKind::AirWavelength => length_scale(unit),
            SpectralKind::Redshift | SpectralKind::Beta => {
                if unit.is_empty() || unit == "1" {
                    Some(1.0)
                } else {
                    None
                }
            }
        };
        scale
            .or_else(|| unit.is_empty().then_some(1.0))
            .map(|scale| scaled.factor * scale)
            .ok_or_else(|| FitsError::InvalidValue {
                card: format!(
                    "CUNIT {unit:?} is not convertible to the default unit for {}",
                    self.code()
                ),
            })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Characteristic {
    Frequency,
    Wavelength,
    AirWavelength,
    Velocity,
}

impl Characteristic {
    fn from_algorithm(code: &str, expressed: Characteristic) -> Option<Characteristic> {
        let sampled = match code {
            "F2W" | "F2V" | "F2A" => Characteristic::Frequency,
            "W2F" | "W2V" | "W2A" => Characteristic::Wavelength,
            "V2F" | "V2W" | "V2A" => Characteristic::Velocity,
            "A2F" | "A2W" | "A2V" => Characteristic::AirWavelength,
            _ => return None,
        };
        let target = match code.as_bytes()[2] {
            b'F' => Characteristic::Frequency,
            b'W' => Characteristic::Wavelength,
            b'A' => Characteristic::AirWavelength,
            b'V' => Characteristic::Velocity,
            _ => return None,
        };
        (target == expressed).then_some(sampled)
    }
}

#[derive(Debug, Clone, Copy)]
struct ResolvedRest {
    frequency: f64,
    wavelength: f64,
}

type DomainResult = std::result::Result<f64, ()>;

fn rest_requirement(expressed: Characteristic, sampled: Characteristic, kind: SpectralKind) -> u8 {
    let mut requirement = u8::from(matches!(
        kind,
        SpectralKind::RadioVelocity | SpectralKind::OpticalVelocity | SpectralKind::Redshift
    ));
    if (expressed == Characteristic::Velocity) != (sampled == Characteristic::Velocity) {
        requirement += 2;
    }
    requirement
}

fn convert(
    from: Characteristic,
    to: Characteristic,
    value: f64,
    rest: ResolvedRest,
) -> DomainResult {
    let value = finite(value)?;
    if from == to {
        return Ok(value);
    }
    match (from, to) {
        (Characteristic::Frequency, Characteristic::Wavelength)
        | (Characteristic::Wavelength, Characteristic::Frequency) => {
            Ok(SPEED_OF_LIGHT / nonzero(value)?)
        }
        (Characteristic::Wavelength, Characteristic::AirWavelength) => wave_to_air(value),
        (Characteristic::AirWavelength, Characteristic::Wavelength) => air_to_wave(value),
        (Characteristic::Frequency, Characteristic::AirWavelength) => {
            wave_to_air(SPEED_OF_LIGHT / nonzero(value)?)
        }
        (Characteristic::AirWavelength, Characteristic::Frequency) => {
            Ok(SPEED_OF_LIGHT / nonzero(air_to_wave(value)?)?)
        }
        (Characteristic::Frequency, Characteristic::Velocity) => {
            assert!(rest.frequency != 0.0, "resolved rest frequency");
            let squared_rest = rest.frequency.powi(2);
            let squared_value = value.powi(2);
            finite(SPEED_OF_LIGHT * (squared_rest - squared_value) / (squared_rest + squared_value))
        }
        (Characteristic::Velocity, Characteristic::Frequency) => {
            let value = subluminal(value)?;
            assert!(rest.frequency != 0.0, "resolved rest frequency");
            finite(rest.frequency * ((SPEED_OF_LIGHT - value) / (SPEED_OF_LIGHT + value)).sqrt())
        }
        (Characteristic::Wavelength, Characteristic::Velocity) => {
            assert!(rest.wavelength != 0.0, "resolved rest wavelength");
            let squared_rest = rest.wavelength.powi(2);
            let squared_value = value.powi(2);
            finite(SPEED_OF_LIGHT * (squared_value - squared_rest) / (squared_value + squared_rest))
        }
        (Characteristic::Velocity, Characteristic::Wavelength) => {
            let value = subluminal(value)?;
            assert!(rest.wavelength != 0.0, "resolved rest wavelength");
            finite(rest.wavelength * ((SPEED_OF_LIGHT + value) / (SPEED_OF_LIGHT - value)).sqrt())
        }
        (Characteristic::AirWavelength, Characteristic::Velocity) => convert(
            Characteristic::Wavelength,
            Characteristic::Velocity,
            air_to_wave(value)?,
            rest,
        ),
        (Characteristic::Velocity, Characteristic::AirWavelength) => wave_to_air(convert(
            Characteristic::Velocity,
            Characteristic::Wavelength,
            value,
            rest,
        )?),
        _ => unreachable!("all characteristic conversions are covered"),
    }
}

fn conversion_derivative(
    from: Characteristic,
    to: Characteristic,
    from_value: f64,
    to_value: f64,
    rest: ResolvedRest,
) -> DomainResult {
    if from == to {
        return Ok(1.0);
    }
    let derivative = match (from, to) {
        (Characteristic::Frequency, Characteristic::Wavelength)
        | (Characteristic::Wavelength, Characteristic::Frequency) => -to_value / from_value,
        (Characteristic::Wavelength, Characteristic::AirWavelength) => {
            1.0 / wave_derivative_from_air(to_value)?
        }
        (Characteristic::AirWavelength, Characteristic::Wavelength) => {
            wave_derivative_from_air(from_value)?
        }
        (Characteristic::Frequency, Characteristic::AirWavelength) => {
            let wave = SPEED_OF_LIGHT / from_value;
            1.0 / ((-from_value / wave) * wave_derivative_from_air(to_value)?)
        }
        (Characteristic::AirWavelength, Characteristic::Frequency) => {
            let wave = air_to_wave(from_value)?;
            (-to_value / wave) * wave_derivative_from_air(from_value)?
        }
        (Characteristic::Frequency, Characteristic::Velocity) => {
            1.0 / frequency_derivative_from_velocity(to_value, rest)?
        }
        (Characteristic::Velocity, Characteristic::Frequency) => {
            frequency_derivative_from_velocity(from_value, rest)?
        }
        (Characteristic::Wavelength, Characteristic::Velocity) => {
            1.0 / wavelength_derivative_from_velocity(to_value, rest)?
        }
        (Characteristic::Velocity, Characteristic::Wavelength) => {
            wavelength_derivative_from_velocity(from_value, rest)?
        }
        (Characteristic::AirWavelength, Characteristic::Velocity) => {
            wave_derivative_from_air(from_value)?
                / wavelength_derivative_from_velocity(to_value, rest)?
        }
        (Characteristic::Velocity, Characteristic::AirWavelength) => {
            wavelength_derivative_from_velocity(from_value, rest)?
                / wave_derivative_from_air(to_value)?
        }
        _ => unreachable!("all characteristic derivatives are covered"),
    };
    finite(derivative)
}

fn frequency_derivative_from_velocity(value: f64, rest: ResolvedRest) -> DomainResult {
    let gamma = lorentz_factor(value)?;
    assert!(rest.frequency != 0.0, "resolved rest frequency");
    finite(-gamma * rest.frequency / (SPEED_OF_LIGHT + value))
}

fn wavelength_derivative_from_velocity(value: f64, rest: ResolvedRest) -> DomainResult {
    let gamma = lorentz_factor(value)?;
    assert!(rest.wavelength != 0.0, "resolved rest wavelength");
    finite(gamma * rest.wavelength / (SPEED_OF_LIGHT - value))
}

fn lorentz_factor(value: f64) -> DomainResult {
    let value = subluminal(value)?;
    finite(1.0 / (1.0 - (value / SPEED_OF_LIGHT).powi(2)).sqrt())
}

fn wave_to_air(wave: f64) -> DomainResult {
    let wave = nonzero(wave)?;
    let mut index = 1.0;
    for _ in 0..4 {
        let inverse_square = (index / wave).powi(2);
        index = refractive_index(inverse_square)?;
    }
    finite(wave / index)
}

fn air_to_wave(air: f64) -> DomainResult {
    let air = nonzero(air)?;
    finite(air * refractive_index((1.0 / air).powi(2))?)
}

fn wave_derivative_from_air(air: f64) -> DomainResult {
    let air = nonzero(air)?;
    let inverse_square = (1.0 / air).powi(2);
    let first = 0.41e14 - inverse_square;
    let second = 1.46e14 - inverse_square;
    if first == 0.0 || second == 0.0 {
        return Err(());
    }
    let index = refractive_index(inverse_square)?;
    finite(index - 2.0 * inverse_square * (2.554e8 / first.powi(2) + 294.981e8 / second.powi(2)))
}

fn refractive_index(inverse_square: f64) -> DomainResult {
    let first = 0.41e14 - inverse_square;
    let second = 1.46e14 - inverse_square;
    if first == 0.0 || second == 0.0 {
        return Err(());
    }
    finite(1.000_064_328 + 2.554e8 / first + 294.981e8 / second)
}

fn is_spectral_pair_syntax(code: &str) -> bool {
    let bytes = code.as_bytes();
    bytes.len() == 3
        && matches!(bytes[0], b'F' | b'W' | b'A' | b'V')
        && bytes[1] == b'2'
        && matches!(bytes[2], b'F' | b'W' | b'A' | b'V')
}

fn algorithm_name(sampled: Characteristic, expressed: Characteristic) -> &'static str {
    match (sampled, expressed) {
        (Characteristic::Frequency, Characteristic::Wavelength) => "F2W",
        (Characteristic::Frequency, Characteristic::Velocity) => "F2V",
        (Characteristic::Frequency, Characteristic::AirWavelength) => "F2A",
        (Characteristic::Wavelength, Characteristic::Frequency) => "W2F",
        (Characteristic::Wavelength, Characteristic::Velocity) => "W2V",
        (Characteristic::Wavelength, Characteristic::AirWavelength) => "W2A",
        (Characteristic::Velocity, Characteristic::Frequency) => "V2F",
        (Characteristic::Velocity, Characteristic::Wavelength) => "V2W",
        (Characteristic::Velocity, Characteristic::AirWavelength) => "V2A",
        (Characteristic::AirWavelength, Characteristic::Frequency) => "A2F",
        (Characteristic::AirWavelength, Characteristic::Wavelength) => "A2W",
        (Characteristic::AirWavelength, Characteristic::Velocity) => "A2V",
        _ => panic!("spectral algorithm must convert distinct characteristics"),
    }
}

fn prefixed_unit(unit: &str, base: &str, base_scale: f64) -> Option<f64> {
    if unit == base {
        return Some(base_scale);
    }
    let prefix = unit.strip_suffix(base)?;
    unit::si_prefix(prefix).map(|scale| scale * base_scale)
}

fn length_scale(unit: &str) -> Option<f64> {
    match unit {
        "m" => Some(1.0),
        "Angstrom" | "angstrom" => Some(1e-10),
        _ => prefixed_unit(unit, "m", 1.0),
    }
}

fn energy_scale(unit: &str) -> Option<f64> {
    match unit {
        "J" => Some(1.0),
        "erg" => Some(1e-7),
        "eV" => Some(1.602_176_634e-19),
        _ => prefixed_unit(unit, "J", 1.0).or_else(|| prefixed_unit(unit, "eV", 1.602_176_634e-19)),
    }
}

fn wavenumber_scale(unit: &str) -> Option<f64> {
    let compact = unit.replace(' ', "");
    for suffix in ["**-1", "^-1", "-1"] {
        if let Some(length) = compact.strip_suffix(suffix) {
            return length_scale(length).map(|scale| 1.0 / scale);
        }
    }
    compact
        .strip_prefix("1/")
        .or_else(|| compact.strip_prefix('/'))
        .and_then(length_scale)
        .map(|scale| 1.0 / scale)
}

fn velocity_scale(unit: &str) -> Option<f64> {
    let compact = unit.replace([' ', '.'], "");
    if let Some(length) = compact.strip_suffix("/s") {
        return length_scale(length);
    }
    for suffix in ["s**-1", "s^-1", "s-1"] {
        if let Some(length) = compact.strip_suffix(suffix) {
            return length_scale(length.trim_end_matches('*'));
        }
    }
    None
}

fn nonzero(value: f64) -> DomainResult {
    if value.is_finite() && value != 0.0 {
        Ok(value)
    } else {
        Err(())
    }
}

fn subluminal(value: f64) -> DomainResult {
    if value.is_finite() && value.abs() < SPEED_OF_LIGHT {
        Ok(value)
    } else {
        Err(())
    }
}

fn finite(value: f64) -> DomainResult {
    if value.is_finite() {
        Ok(value)
    } else {
        Err(())
    }
}

fn invalid_reference(kind: SpectralKind) -> FitsError {
    FitsError::InvalidValue {
        card: format!("{} has an invalid spectral reference value", kind.code()),
    }
}

fn domain_error(axis: usize, algorithm: &'static str) -> FitsError {
    FitsError::WcsCoordinateDomain { axis, algorithm }
}
