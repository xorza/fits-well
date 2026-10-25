//! [`PhysicalSample`]: the float types a physical-plane read writes.

/// The element type of a physical-plane read: `f64`, the canonical plane, or `f32`,
/// the compact one. Sealed: the element type of the caller's buffer picks it, and no
/// other type implements it. The scaling arithmetic always runs in `f64`, and `f32`
/// narrows its result: a value is rounded twice, once to `f64` and once to `f32`, so
/// it can differ by one ulp from the `f32` nearest the exact physical value.
pub trait PhysicalSample: sealed::Narrow {}

impl PhysicalSample for f64 {}
impl PhysicalSample for f32 {}

pub(crate) mod sealed {
    /// The narrowing from the `f64` physical value.
    #[expect(
        unnameable_types,
        reason = "sealed: outside code may use it, never name it"
    )]
    pub trait Narrow: Copy + Send + Sync {
        fn from_f64(value: f64) -> Self;
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
