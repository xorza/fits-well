//! [`PhysicalSample`]: the float types a physical-plane read writes.

/// The element type of a physical-plane read: `f64`, the canonical plane, or `f32`,
/// the compact one. Sealed: the element type of the caller's buffer picks it, and no
/// other type implements it. The scaling arithmetic always runs in `f64`; `f32` is
/// the final narrowing, so each value is the correctly rounded `f32`.
pub trait PhysicalSample: sealed::Narrow {}

impl PhysicalSample for f64 {}
impl PhysicalSample for f32 {}

pub(crate) mod sealed {
    use crate::image_model::scaling::Scaling;

    /// The narrowing from the `f64` physical value, paired with the [`Scaling`] map so
    /// the decoded-sample and the big-endian loops share one definition of "physical
    /// value".
    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub trait Narrow: Copy {
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

    impl Narrow for f64 {
        fn from_f64(value: f64) -> Self {
            value
        }
    }

    impl Narrow for f32 {
        fn from_f64(value: f64) -> Self {
            value as Self
        }
    }
}
