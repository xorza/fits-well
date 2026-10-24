//! The stored sample types a decoded tile narrows into.

use crate::compress::decode::tile_decoder::Overshoot;
use crate::compress::decode::wide_plane::WidePlane;
use crate::error::CompressedDataDefect;
use crate::error::Result;

/// A stored sample type the decoder scatters into, paired with the plane its tiles
/// decode in. Narrowing happens only as values land in the output plane.
pub(super) trait DecodeSample: Copy + Default + Send + Sync {
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
    type Wide = Self;
    fn fit(_: &mut [Self::Wide], _: Overshoot) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: Self::Wide) -> Self {
        wide
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
                return Err(CompressedDataDefect::ValueOutOfRange.into());
            }
        }
    }
    Ok(())
}
