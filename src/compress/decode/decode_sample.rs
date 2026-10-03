//! The stored sample types a decoded tile narrows into.

use crate::compress::decode::wide_plane::WidePlane;
use crate::error::FitsError;
use crate::error::Result;

/// A stored sample type the decoder scatters into, paired with the plane its tiles
/// decode in. Narrowing happens only as values land in the output plane.
pub(super) trait DecodeSample: Copy + Send + Sync {
    type Wide: WidePlane;

    /// Fit a decoded tile into this type's range, so that [`DecodeSample::narrow`] is
    /// exact. A `lossy` codec's reconstruction may overshoot the range its original
    /// values lay in, so its values clamp to the range, as cfitsio clips them. A
    /// lossless stream an encoder wrote never leaves the range, so a value outside it
    /// is corrupt data (cfitsio `NUM_OVERFLOW`).
    fn fit(values: &mut [Self::Wide], lossy: bool) -> Result<()>;

    fn narrow(wide: Self::Wide) -> Self;
}

impl DecodeSample for u8 {
    type Wide = i64;
    fn fit(values: &mut [i64], lossy: bool) -> Result<()> {
        fit_integers(values, 0, u8::MAX.into(), lossy)
    }
    fn narrow(wide: i64) -> u8 {
        wide.cast_unsigned() as u8
    }
}

impl DecodeSample for i16 {
    type Wide = i64;
    fn fit(values: &mut [i64], lossy: bool) -> Result<()> {
        fit_integers(values, i16::MIN.into(), i16::MAX.into(), lossy)
    }
    fn narrow(wide: i64) -> i16 {
        wide as i16
    }
}

impl DecodeSample for i32 {
    type Wide = i64;
    fn fit(values: &mut [i64], lossy: bool) -> Result<()> {
        fit_integers(values, i32::MIN.into(), i32::MAX.into(), lossy)
    }
    fn narrow(wide: i64) -> i32 {
        wide as i32
    }
}

impl DecodeSample for i64 {
    type Wide = i64;
    fn fit(_: &mut [i64], _: bool) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: i64) -> i64 {
        wide
    }
}

impl DecodeSample for f32 {
    type Wide = f64;
    /// Narrowing a float rounds rather than overflows.
    fn fit(_: &mut [f64], _: bool) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: f64) -> f32 {
        wide as f32
    }
}

impl DecodeSample for f64 {
    type Wide = f64;
    fn fit(_: &mut [f64], _: bool) -> Result<()> {
        Ok(())
    }
    fn narrow(wide: f64) -> f64 {
        wide
    }
}

fn fit_integers(values: &mut [i64], min: i64, max: i64, lossy: bool) -> Result<()> {
    if lossy {
        for value in values {
            *value = (*value).clamp(min, max);
        }
        return Ok(());
    }
    if values.iter().any(|value| !(min..=max).contains(value)) {
        return Err(FitsError::CorruptCompressedData {
            detail: "a decoded tile value is outside the image's BITPIX range".to_string(),
        });
    }
    Ok(())
}
