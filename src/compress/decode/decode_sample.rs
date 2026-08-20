//! The stored sample types a decoded tile narrows into.

use crate::compress::decode::wide_plane::WidePlane;

/// A stored sample type the decoder scatters into, paired with the plane its tiles
/// decode in. Narrowing happens only as values land in the output plane.
pub(super) trait DecodeSample: Copy + Send + Sync {
    type Wide: WidePlane;
    fn narrow(wide: Self::Wide) -> Self;
}

impl DecodeSample for u8 {
    type Wide = i64;
    fn narrow(wide: i64) -> u8 {
        wide as u8
    }
}

impl DecodeSample for i16 {
    type Wide = i64;
    fn narrow(wide: i64) -> i16 {
        wide as i16
    }
}

impl DecodeSample for i32 {
    type Wide = i64;
    fn narrow(wide: i64) -> i32 {
        wide as i32
    }
}

impl DecodeSample for i64 {
    type Wide = i64;
    fn narrow(wide: i64) -> i64 {
        wide
    }
}

impl DecodeSample for f32 {
    type Wide = f64;
    fn narrow(wide: f64) -> f32 {
        wide as f32
    }
}

impl DecodeSample for f64 {
    type Wide = f64;
    fn narrow(wide: f64) -> f64 {
        wide
    }
}
