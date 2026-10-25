//! [`Quantization`]: how a float image's tiles become integers before a codec (§10.2).

use crate::fits_error::CompressionParameter;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// A float image's quantization for [`crate::FitsWriter::write_compressed_image`].
///
/// [`Quantization::NONE`] stores the floats losslessly (`ZQUANTIZ = 'NONE'`, `GZIP_1`
/// or `GZIP_2` only). [`Quantization::relative`] spaces each tile's levels at its
/// background noise divided by `level`, as fpack's `-q level`;
/// [`Quantization::absolute`] spaces every tile's levels at one `step`, as fpack's
/// `-q -step`. The default is `relative(4.0)` with `SUBTRACTIVE_DITHER_1`, cfitsio's.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Quantization(Spacing);

/// Whether a float image is stored losslessly or quantized.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum Spacing {
    None,
    Quantized(Levels),
}

/// The levels a quantized image's tiles round to, and the dither before rounding.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Levels {
    pub(super) step: QuantizationStep,
    pub(super) dither: Dither,
}

/// The spacing of a tile's quantized levels (cfitsio's `qlevel`, whose sign selects
/// the form).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) enum QuantizationStep {
    /// The tile's noise estimate divided by this level.
    Relative(f64),
    /// This step, whatever the tile's noise.
    Absolute(f64),
}

impl Quantization {
    /// Lossless: a float image's samples themselves, gzip'd. An integer image and a table
    /// take it too, as they keep their values.
    pub const NONE: Self = Self(Spacing::None);

    /// Levels spaced at each tile's noise estimate divided by `level`, a finite
    /// positive number.
    pub fn relative(level: f64, dither: Dither) -> Result<Self> {
        Ok(Self(Spacing::Quantized(Levels {
            step: QuantizationStep::Relative(positive(level, |level| {
                CompressionParameter::QuantizationLevel { level }
            })?),
            dither,
        })))
    }

    /// Levels spaced at `step` for every tile, a finite positive number.
    pub fn absolute(step: f64, dither: Dither) -> Result<Self> {
        Ok(Self(Spacing::Quantized(Levels {
            step: QuantizationStep::Absolute(positive(step, |step| {
                CompressionParameter::QuantizationStep { step }
            })?),
            dither,
        })))
    }

    pub(super) const fn spacing(self) -> Spacing {
        self.0
    }
}

impl Default for Quantization {
    fn default() -> Self {
        Self(Spacing::Quantized(Levels {
            step: QuantizationStep::Relative(4.0),
            dither: Dither::default(),
        }))
    }
}

/// The dither a quantized float image applies before rounding (`ZQUANTIZ`), with its
/// seed (`ZDITHER0`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dither {
    /// `NO_DITHER`: plain rounding.
    None,
    /// `SUBTRACTIVE_DITHER_1`: a per-pixel offset from the shared random sequence.
    Subtractive1(DitherSeed),
    /// `SUBTRACTIVE_DITHER_2`: as `Subtractive1`, but an exact zero stays zero.
    Subtractive2(DitherSeed),
}

impl Default for Dither {
    fn default() -> Self {
        Self::Subtractive1(DitherSeed::FROM_SAMPLES)
    }
}

impl Dither {
    pub(super) const fn method(self) -> DitherMethod {
        match self {
            Self::None => DitherMethod::None,
            Self::Subtractive1(_) => DitherMethod::Subtractive1,
            Self::Subtractive2(_) => DitherMethod::Subtractive2,
        }
    }

    pub(super) const fn seed(self) -> Option<DitherSeed> {
        match self {
            Self::None => None,
            Self::Subtractive1(seed) | Self::Subtractive2(seed) => Some(seed),
        }
    }
}

/// Where a dither's start in the random sequence (`ZDITHER0`, 1 to 10000) comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DitherSeed(Seed);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Seed {
    FromSamples,
    Fixed(u16),
}

impl DitherSeed {
    /// `(Σ bytes of every sample mod 10000) + 1`: the same image gives the same file,
    /// and images that differ get different seeds.
    pub const FROM_SAMPLES: Self = Self(Seed::FromSamples);

    /// The seed `seed`, from 1 to 10000.
    pub fn fixed(seed: u16) -> Result<Self> {
        if !(1..=10_000).contains(&seed) {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::DitherSeed { seed },
            });
        }
        Ok(Self(Seed::Fixed(seed)))
    }

    pub(super) const fn source(self) -> Seed {
        self.0
    }
}

/// The dither method a `ZQUANTIZ` value names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DitherMethod {
    None,
    Subtractive1,
    Subtractive2,
}

impl DitherMethod {
    /// Whether this method applies a per-pixel dither (everything but `None`).
    pub(super) const fn dithered(self) -> bool {
        !matches!(self, Self::None)
    }

    /// The method a `ZQUANTIZ` value names; `None` for a value this crate does not
    /// model. A float image must reproduce its dither exactly, so the decoder refuses
    /// an unrecognized value there; an integer image ignores `ZQUANTIZ`.
    pub(super) fn parse(s: &str) -> Option<Self> {
        match s {
            "NO_DITHER" => Some(Self::None),
            "SUBTRACTIVE_DITHER_1" => Some(Self::Subtractive1),
            "SUBTRACTIVE_DITHER_2" => Some(Self::Subtractive2),
            _ => None,
        }
    }

    pub(super) const fn name(self) -> &'static str {
        match self {
            Self::None => "NO_DITHER",
            Self::Subtractive1 => "SUBTRACTIVE_DITHER_1",
            Self::Subtractive2 => "SUBTRACTIVE_DITHER_2",
        }
    }
}

fn positive(value: f64, parameter: impl FnOnce(f64) -> CompressionParameter) -> Result<f64> {
    if !value.is_finite() || value <= 0.0 {
        return Err(FitsError::InvalidCompressionParameter {
            parameter: parameter(value),
        });
    }
    Ok(value)
}
