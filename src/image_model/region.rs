//! [`Region`]: the part of an image a read covers.

use std::ops::{Range, RangeFull};

use crate::fits_error::{FitsError, Result};

/// The part of an image a read covers: all of it, or one zero-based, half-open range
/// per axis, fastest axis first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Region<'a> {
    All,
    Section(&'a [Range<usize>]),
}

/// `..`, the whole image, as a range of every element is in std.
impl From<RangeFull> for Region<'_> {
    fn from(_: RangeFull) -> Self {
        Self::All
    }
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

/// One run of a section's stored bytes: contiguous in the image and in the section.
#[derive(Debug, Clone, Copy)]
pub(crate) struct ByteRun {
    pub(crate) offset: usize,
    pub(crate) len: usize,
}

/// The section `ranges` selects of an image of `shape`, of `selected_shape`, stored
/// `element_size` bytes a sample.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SectionRuns<'a> {
    pub(crate) shape: &'a [usize],
    pub(crate) ranges: &'a [Range<usize>],
    pub(crate) selected_shape: &'a [usize],
    pub(crate) element_size: usize,
}

impl SectionRuns<'_> {
    /// Give `visit` each run of the section's stored bytes, in order, with the runs that
    /// follow one another joined.
    pub(crate) fn visit(self, mut visit: impl FnMut(ByteRun) -> Result<()>) -> Result<()> {
        let Self {
            shape,
            ranges,
            selected_shape,
            element_size,
        } = self;
        debug_assert_eq!(shape.len(), ranges.len());
        debug_assert_eq!(shape.len(), selected_shape.len());
        if selected_shape.is_empty() || selected_shape.contains(&0) {
            return Ok(());
        }
        let row_count = selected_shape[1..]
            .iter()
            .try_fold(1usize, |count, &len| count.checked_mul(len))
            .ok_or(FitsError::DataUnitOverflow)?;
        let row_bytes = selected_shape[0]
            .checked_mul(element_size)
            .ok_or(FitsError::DataUnitOverflow)?;
        let mut pending: Option<ByteRun> = None;
        for row_index in 0..row_count {
            let mut remainder = row_index;
            let mut element_offset = ranges[0].start;
            let mut stride = shape[0];
            for axis in 1..shape.len() {
                let coordinate = ranges[axis].start + remainder % selected_shape[axis];
                remainder /= selected_shape[axis];
                element_offset = coordinate
                    .checked_mul(stride)
                    .and_then(|value| element_offset.checked_add(value))
                    .ok_or(FitsError::DataUnitOverflow)?;
                if axis + 1 < shape.len() {
                    stride = stride
                        .checked_mul(shape[axis])
                        .ok_or(FitsError::DataUnitOverflow)?;
                }
            }
            let next = ByteRun {
                offset: element_offset
                    .checked_mul(element_size)
                    .ok_or(FitsError::DataUnitOverflow)?,
                len: row_bytes,
            };
            if let Some(previous) = pending.as_mut() {
                let end = previous
                    .offset
                    .checked_add(previous.len)
                    .ok_or(FitsError::DataUnitOverflow)?;
                if end == next.offset {
                    previous.len = previous
                        .len
                        .checked_add(next.len)
                        .ok_or(FitsError::DataUnitOverflow)?;
                    continue;
                }
            }
            if let Some(previous) = pending.replace(next) {
                visit(previous)?;
            }
        }
        if let Some(run) = pending {
            visit(run)?;
        }
        Ok(())
    }
}
