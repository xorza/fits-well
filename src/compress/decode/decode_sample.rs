//! The stored sample types a decoded tile narrows into.

use crate::compress::decode::wide_plane::WidePlane;
use crate::error::FitsError;
use crate::error::Result;

/// What a decoded value outside the image's `BITPIX` range means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Overshoot {
    /// A lossy codec's reconstruction may overshoot the range its original values lay
    /// in, so the value clamps to the range, as cfitsio clips it.
    Clamp,
    /// A lossless stream an encoder wrote never leaves the range, so the value is
    /// corrupt data (cfitsio `NUM_OVERFLOW`).
    Refuse,
}

/// A stored sample type the decoder scatters into, paired with the plane its tiles
/// decode in. Narrowing happens only as values land in the output plane.
pub(super) trait DecodeSample: Copy + Send + Sync {
    type Wide: WidePlane;

    /// Fit a decoded tile into this type's range, so that [`DecodeSample::narrow`] is
    /// exact, treating a value outside it as `overshoot` says.
    fn fit(values: &mut [Self::Wide], overshoot: Overshoot) -> Result<()>;

    fn narrow(wide: Self::Wide) -> Self;
}

impl DecodeSample for u8 {
    type Wide = i64;
    fn fit(values: &mut [Self::Wide], overshoot: Overshoot) -> Result<()> {
        fit_integers(values, 0, Self::MAX.into(), overshoot)
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide.cast_unsigned() as Self
    }
}

impl DecodeSample for i16 {
    type Wide = i64;
    fn fit(values: &mut [Self::Wide], overshoot: Overshoot) -> Result<()> {
        fit_integers(values, Self::MIN.into(), Self::MAX.into(), overshoot)
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide as Self
    }
}

impl DecodeSample for i32 {
    type Wide = i64;
    fn fit(values: &mut [Self::Wide], overshoot: Overshoot) -> Result<()> {
        fit_integers(values, Self::MIN.into(), Self::MAX.into(), overshoot)
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide as Self
    }
}

impl DecodeSample for i64 {
    type Wide = Self;
    fn fit(_: &mut [Self::Wide], _: Overshoot) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide
    }
}

impl DecodeSample for f32 {
    type Wide = f64;
    /// Narrowing a float rounds rather than overflows.
    fn fit(_: &mut [Self::Wide], _: Overshoot) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide as Self
    }
}

impl DecodeSample for f64 {
    type Wide = Self;
    fn fit(_: &mut [Self::Wide], _: Overshoot) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide
    }
}

fn fit_integers(values: &mut [i64], min: i64, max: i64, overshoot: Overshoot) -> Result<()> {
    match overshoot {
        Overshoot::Clamp => {
            for value in values {
                *value = (*value).clamp(min, max);
            }
        }
        Overshoot::Refuse => {
            if values.iter().any(|value| !(min..=max).contains(value)) {
                return Err(FitsError::CorruptCompressedData {
                    detail: "a decoded tile value is outside the image's BITPIX range".to_string(),
                });
            }
        }
    }
    Ok(())
}
