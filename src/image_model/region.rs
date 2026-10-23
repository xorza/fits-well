//! [`Region`]: the part of an image a read covers.

use std::ops::Range;

/// The part of an image a read covers: all of it, or one zero-based, half-open range
/// per axis, fastest axis first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region<'a> {
    All,
    Section(&'a [Range<usize>]),
}

impl<'a> From<&'a [Range<usize>]> for Region<'a> {
    fn from(ranges: &'a [Range<usize>]) -> Self {
        Self::Section(ranges)
    }
}

impl<'a, const N: usize> From<&'a [Range<usize>; N]> for Region<'a> {
    fn from(ranges: &'a [Range<usize>; N]) -> Self {
        Self::Section(ranges)
    }
}
