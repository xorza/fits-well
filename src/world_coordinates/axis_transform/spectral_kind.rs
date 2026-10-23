//! [`SpectralKind`]: which spectral quantity a `CTYPEi` names, and the
//! conversions between the [`Characteristic`]s one axis can be sampled and
//! expressed in.

use crate::error::FitsError;
use crate::error::Result;
use crate::unit;
use crate::world_coordinates::axis_transform::spectral_rest::ResolvedRest;
use crate::world_coordinates::axis_transform::{PLANCK_CONSTANT, SPEED_OF_LIGHT};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SpectralKind {
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
    pub(super) fn from_code(code: &str) -> Option<Self> {
        match code {
            "FREQ" => Some(Self::Frequency),
            "ENER" => Some(Self::Energy),
            "WAVN" => Some(Self::Wavenumber),
            "VRAD" => Some(Self::RadioVelocity),
            "WAVE" => Some(Self::Wavelength),
            "VOPT" => Some(Self::OpticalVelocity),
            "ZOPT" => Some(Self::Redshift),
            "AWAV" => Some(Self::AirWavelength),
            "VELO" => Some(Self::RelativisticVelocity),
            "BETA" => Some(Self::Beta),
            _ => None,
        }
    }

    pub(super) const fn code(self) -> &'static str {
        match self {
            Self::Frequency => "FREQ",
            Self::Energy => "ENER",
            Self::Wavenumber => "WAVN",
            Self::RadioVelocity => "VRAD",
            Self::Wavelength => "WAVE",
            Self::OpticalVelocity => "VOPT",
            Self::Redshift => "ZOPT",
            Self::AirWavelength => "AWAV",
            Self::RelativisticVelocity => "VELO",
            Self::Beta => "BETA",
        }
    }

    pub(super) const fn characteristic(self) -> Characteristic {
        match self {
            Self::Frequency | Self::Energy | Self::Wavenumber | Self::RadioVelocity => {
                Characteristic::Frequency
            }
            Self::Wavelength | Self::OpticalVelocity | Self::Redshift => Characteristic::Wavelength,
            Self::AirWavelength => Characteristic::AirWavelength,
            Self::RelativisticVelocity | Self::Beta => Characteristic::Velocity,
        }
    }

    pub(super) fn to_characteristic(self, value: f64, rest: ResolvedRest) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        let result = match self {
            Self::Frequency | Self::Wavelength | Self::AirWavelength => nonzero(value)?,
            Self::Energy => nonzero(value)? / PLANCK_CONSTANT,
            Self::Wavenumber => nonzero(value)? * SPEED_OF_LIGHT,
            Self::RadioVelocity => {
                debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
                rest.frequency * (1.0 - value / SPEED_OF_LIGHT)
            }
            Self::OpticalVelocity => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                nonzero(rest.wavelength * (1.0 + value / SPEED_OF_LIGHT))?
            }
            Self::Redshift => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                nonzero(rest.wavelength * (1.0 + value))?
            }
            Self::RelativisticVelocity => subluminal(value)?,
            Self::Beta => subluminal(value * SPEED_OF_LIGHT)?,
        };
        finite(result)
    }

    pub(super) fn world_from_characteristic(self, value: f64, rest: ResolvedRest) -> Option<f64> {
        let value = finite(value)?;
        let result = match self {
            Self::Frequency
            | Self::Wavelength
            | Self::AirWavelength
            | Self::RelativisticVelocity => value,
            Self::Energy => value * PLANCK_CONSTANT,
            Self::Wavenumber | Self::Beta => value / SPEED_OF_LIGHT,
            Self::RadioVelocity => {
                debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
                SPEED_OF_LIGHT * (1.0 - value / rest.frequency)
            }
            Self::OpticalVelocity => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                SPEED_OF_LIGHT * (value / rest.wavelength - 1.0)
            }
            Self::Redshift => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                value / rest.wavelength - 1.0
            }
        };
        finite(result)
    }

    pub(super) fn derivative(self, rest: ResolvedRest) -> f64 {
        match self {
            Self::Frequency
            | Self::Wavelength
            | Self::AirWavelength
            | Self::RelativisticVelocity => 1.0,
            Self::Energy => 1.0 / PLANCK_CONSTANT,
            Self::Wavenumber | Self::Beta => SPEED_OF_LIGHT,
            Self::RadioVelocity => {
                debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
                -rest.frequency / SPEED_OF_LIGHT
            }
            Self::OpticalVelocity => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                rest.wavelength / SPEED_OF_LIGHT
            }
            Self::Redshift => {
                debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
                rest.wavelength
            }
        }
    }

    /// The factor from `unit` to this kind's Table-25 default unit; an absent unit is the
    /// default unit itself.
    pub(super) fn unit_scale(self, unit: &str) -> Result<f64> {
        if unit.trim().is_empty() {
            return Ok(1.0);
        }
        let scale = match self {
            Self::Frequency => unit::resolve(unit, unit::FREQUENCY),
            Self::Energy => unit::resolve(unit, unit::ENERGY),
            Self::Wavenumber => wavenumber_scale(unit),
            Self::RadioVelocity | Self::OpticalVelocity | Self::RelativisticVelocity => {
                velocity_scale(unit)
            }
            Self::Wavelength | Self::AirWavelength => unit::resolve(unit, unit::LENGTH),
            Self::Redshift | Self::Beta => (unit.trim() == "1").then_some(1.0),
        };
        scale.ok_or_else(|| FitsError::InvalidUnit {
            unit: unit.to_string(),
            expected: self.unit_kind(),
        })
    }

    /// The Table-25 default unit the transform evaluates this quantity in.
    pub(super) const fn default_unit(self) -> &'static str {
        match self {
            Self::Frequency => "Hz",
            Self::Energy => "J",
            Self::Wavenumber => "m-1",
            Self::RadioVelocity | Self::OpticalVelocity | Self::RelativisticVelocity => "m/s",
            Self::Wavelength | Self::AirWavelength => "m",
            Self::Redshift | Self::Beta => "",
        }
    }

    /// The kind of unit this quantity is measured in, for error messages.
    const fn unit_kind(self) -> &'static str {
        match self {
            Self::Frequency => "a frequency unit",
            Self::Energy => "an energy unit",
            Self::Wavenumber => "an inverse length unit",
            Self::RadioVelocity | Self::OpticalVelocity | Self::RelativisticVelocity => {
                "a velocity unit"
            }
            Self::Wavelength | Self::AirWavelength => "a length unit",
            Self::Redshift | Self::Beta => "dimensionless",
        }
    }
}

/// The four basic spectral characteristics (WCS Paper III, Table 2), in the order
/// of [`Characteristic::ALL`], which their discriminants index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Characteristic {
    Frequency,
    Wavelength,
    AirWavelength,
    Velocity,
}

impl Characteristic {
    pub(super) const ALL: [Self; 4] = [
        Self::Frequency,
        Self::Wavelength,
        Self::AirWavelength,
        Self::Velocity,
    ];

    /// The letter an algorithm code names this characteristic by.
    pub(super) const fn letter(self) -> u8 {
        match self {
            Self::Frequency => b'F',
            Self::Wavelength => b'W',
            Self::AirWavelength => b'A',
            Self::Velocity => b'V',
        }
    }

    pub(super) fn from_letter(letter: u8) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|characteristic| characteristic.letter() == letter)
    }
}

pub(super) fn convert(
    from: Characteristic,
    to: Characteristic,
    value: f64,
    rest: ResolvedRest,
) -> Option<f64> {
    let value = finite(value)?;
    if from == to {
        return Some(value);
    }
    match (from, to) {
        (Characteristic::Frequency, Characteristic::Wavelength)
        | (Characteristic::Wavelength, Characteristic::Frequency) => {
            Some(SPEED_OF_LIGHT / nonzero(value)?)
        }
        (Characteristic::Wavelength, Characteristic::AirWavelength) => wave_to_air(value),
        (Characteristic::AirWavelength, Characteristic::Wavelength) => air_to_wave(value),
        (Characteristic::Frequency, Characteristic::AirWavelength) => {
            wave_to_air(SPEED_OF_LIGHT / nonzero(value)?)
        }
        (Characteristic::AirWavelength, Characteristic::Frequency) => {
            Some(SPEED_OF_LIGHT / nonzero(air_to_wave(value)?)?)
        }
        (Characteristic::Frequency, Characteristic::Velocity) => {
            debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
            let squared_rest = rest.frequency.powi(2);
            let squared_value = value.powi(2);
            finite(SPEED_OF_LIGHT * (squared_rest - squared_value) / (squared_rest + squared_value))
        }
        (Characteristic::Velocity, Characteristic::Frequency) => {
            let value = subluminal(value)?;
            debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
            finite(rest.frequency * ((SPEED_OF_LIGHT - value) / (SPEED_OF_LIGHT + value)).sqrt())
        }
        (Characteristic::Wavelength, Characteristic::Velocity) => {
            debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
            let squared_rest = rest.wavelength.powi(2);
            let squared_value = value.powi(2);
            finite(SPEED_OF_LIGHT * (squared_value - squared_rest) / (squared_value + squared_rest))
        }
        (Characteristic::Velocity, Characteristic::Wavelength) => {
            let value = subluminal(value)?;
            debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
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

pub(super) fn conversion_derivative(
    from: Characteristic,
    to: Characteristic,
    from_value: f64,
    to_value: f64,
    rest: ResolvedRest,
) -> Option<f64> {
    if from == to {
        return Some(1.0);
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

fn frequency_derivative_from_velocity(value: f64, rest: ResolvedRest) -> Option<f64> {
    let gamma = lorentz_factor(value)?;
    debug_assert!(rest.frequency != 0.0, "resolved rest frequency");
    finite(-gamma * rest.frequency / (SPEED_OF_LIGHT + value))
}

fn wavelength_derivative_from_velocity(value: f64, rest: ResolvedRest) -> Option<f64> {
    let gamma = lorentz_factor(value)?;
    debug_assert!(rest.wavelength != 0.0, "resolved rest wavelength");
    finite(gamma * rest.wavelength / (SPEED_OF_LIGHT - value))
}

fn lorentz_factor(value: f64) -> Option<f64> {
    let value = subluminal(value)?;
    finite(1.0 / (1.0 - (value / SPEED_OF_LIGHT).powi(2)).sqrt())
}

fn wave_to_air(wave: f64) -> Option<f64> {
    let wave = nonzero(wave)?;
    let mut index = 1.0;
    for _ in 0..4 {
        let inverse_square = (index / wave).powi(2);
        index = refractive_index(inverse_square)?;
    }
    finite(wave / index)
}

fn air_to_wave(air: f64) -> Option<f64> {
    let air = nonzero(air)?;
    finite(air * refractive_index((1.0 / air).powi(2))?)
}

fn wave_derivative_from_air(air: f64) -> Option<f64> {
    let air = nonzero(air)?;
    let inverse_square = (1.0 / air).powi(2);
    let first = 0.41e14 - inverse_square;
    let second = 1.46e14 - inverse_square;
    if first == 0.0 || second == 0.0 {
        return None;
    }
    let index = refractive_index(inverse_square)?;
    finite(index - 2.0 * inverse_square * (2.554e8 / first.powi(2) + 294.981e8 / second.powi(2)))
}

pub(super) fn refractive_index(inverse_square: f64) -> Option<f64> {
    let first = 0.41e14 - inverse_square;
    let second = 1.46e14 - inverse_square;
    if first == 0.0 || second == 0.0 {
        return None;
    }
    finite(1.000_064_328 + 2.554e8 / first + 294.981e8 / second)
}

/// `[multiplier] length⁻¹` in any of FITS's spellings, per metre.
fn wavenumber_scale(unit: &str) -> Option<f64> {
    let scaled = unit::split_numeric_multiplier(unit)?;
    let compact = scaled.base.replace(' ', "");
    let length = ["**-1", "^-1", "-1"]
        .into_iter()
        .find_map(|suffix| compact.strip_suffix(suffix))
        .or_else(|| compact.strip_prefix("1/"))
        .or_else(|| compact.strip_prefix('/'))?;
    unit::resolve(length, unit::LENGTH).map(|scale| scaled.factor / scale)
}

/// `[multiplier] length / time` in any of FITS's spellings — `km/s`, `m s-1`, `m.s**-1`,
/// `m*s^-1` — per metre per second.
fn velocity_scale(unit: &str) -> Option<f64> {
    let separator = |c: char| c.is_whitespace() || c == '.' || c == '*';
    let scaled = unit::split_numeric_multiplier(unit)?;
    let text = scaled.base.trim();
    let (length, time) = if let Some((length, time)) = text.split_once('/') {
        (length.trim(), time.trim())
    } else {
        let per_time = ["**-1", "^-1", "-1"]
            .into_iter()
            .find_map(|suffix| text.strip_suffix(suffix))?;
        let (length, time) = per_time.rsplit_once(separator)?;
        (length.trim_end_matches(separator), time)
    };
    let length = unit::resolve(length, unit::LENGTH)?;
    let time = unit::resolve(time, unit::TIME)?;
    Some(scaled.factor * length / time)
}

fn nonzero(value: f64) -> Option<f64> {
    (value.is_finite() && value != 0.0).then_some(value)
}

fn subluminal(value: f64) -> Option<f64> {
    (value.is_finite() && value.abs() < SPEED_OF_LIGHT).then_some(value)
}

pub(super) const fn finite(value: f64) -> Option<f64> {
    if value.is_finite() { Some(value) } else { None }
}

pub(super) fn invalid_reference(kind: SpectralKind) -> FitsError {
    FitsError::InvalidWcs {
        detail: format!("{} has an invalid spectral reference value", kind.code()),
    }
}

pub(super) fn required_grism_parameter(
    value: Option<f64>,
    algorithm: &'static str,
    parameter: usize,
) -> Result<f64> {
    match value {
        Some(value) if value.is_finite() && value != 0.0 => Ok(value),
        _ => Err(invalid_grism(
            algorithm,
            &format!("PV_i_{parameter} must be specified, finite, and non-zero"),
        )),
    }
}

pub(super) fn invalid_grism(algorithm: &'static str, detail: &str) -> FitsError {
    FitsError::InvalidWcs {
        detail: format!("{algorithm} {detail}"),
    }
}

pub(super) const fn domain_error(axis: usize, algorithm: &'static str) -> FitsError {
    FitsError::WcsCoordinateDomain { axis, algorithm }
}
