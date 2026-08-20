//! [`PhysicalOut`]: the float types a scaled read can be produced in.

use crate::data::scaling::Scaling;

/// Output element type of the physical-plane map. Crate-private, hence sealed: the
/// only implementors are `f64` (the canonical plane) and `f32` (the compact plane).
/// The scaling arithmetic always runs in `f64`; `from_f64` is the final narrowing.
///
/// [`scaled`](Self::scaled) and [`scaled_integer`](Self::scaled_integer) pair that
/// narrowing with the [`Scaling`] map, so the decoded-sample and the big-endian
/// physical loops apply one definition of "physical value" rather than each
/// spelling the composition itself.
pub(crate) trait PhysicalOut: Copy {
    fn from_f64(value: f64) -> Self;

    /// The physical value of a real sample, narrowed to the output plane.
    fn scaled(raw: f64, scaling: &Scaling) -> Self {
        Self::from_f64(scaling.scale(raw))
    }

    /// The physical value of an integer sample, mapping the `BLANK` sentinel to `NaN`.
    fn scaled_integer(raw: i64, scaling: &Scaling) -> Self {
        Self::from_f64(scaling.scale_integer(raw))
    }
}

impl PhysicalOut for f64 {
    fn from_f64(value: f64) -> f64 {
        value
    }
}

impl PhysicalOut for f32 {
    fn from_f64(value: f64) -> f32 {
        value as f32
    }
}
