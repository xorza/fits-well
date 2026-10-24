//! Float comparison for tests: every bound names the reason it holds, and a failure prints it.

#[derive(Debug, Clone, Copy)]
enum Bound {
    Abs(f64),
    Rel(f64),
}

/// How far a value may be from its expectation, and why.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tolerance {
    bound: Bound,
    why: &'static str,
}

impl Tolerance {
    /// The expectation itself.
    pub(crate) const EXACT: Self = Self::abs(0.0, "exact");

    /// At most `count` ulps of 1 × |expectation| from it: `count` × ε relative.
    pub(crate) const fn ulps(count: f64, why: &'static str) -> Self {
        Self::rel(count * f64::EPSILON, why)
    }

    /// At most `bound` from the expectation.
    pub(crate) const fn abs(bound: f64, why: &'static str) -> Self {
        Self {
            bound: Bound::Abs(bound),
            why,
        }
    }

    /// At most `bound` × |expectation| from it.
    pub(crate) const fn rel(bound: f64, why: &'static str) -> Self {
        Self {
            bound: Bound::Rel(bound),
            why,
        }
    }

    fn admits(self, got: f64, want: f64) -> bool {
        let limit = match self.bound {
            Bound::Abs(bound) => bound,
            Bound::Rel(bound) => bound * want.abs(),
        };
        (got - want).abs() <= limit
    }
}

/// The largest difference between `a` and `b`, element by element.
pub(crate) fn max_abs_diff(a: &[f64], b: &[f64]) -> f64 {
    debug_assert_eq!(a.len(), b.len());
    a.iter()
        .zip(b)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f64::max)
}

/// Assert that each of `got` is within `tolerance` of the same element of `want`.
#[track_caller]
pub(crate) fn assert_close(got: &[f64], want: &[f64], tolerance: Tolerance, context: &str) {
    assert_eq!(got.len(), want.len(), "{context}: lengths");
    for (i, (&got, &want)) in got.iter().zip(want).enumerate() {
        assert!(
            tolerance.admits(got, want),
            "{context}[{i}]: {got} is not within {:?} of {want}: {}",
            tolerance.bound,
            tolerance.why
        );
    }
}
