//! [`Axis`]: a WCS time axis in its frame, resolved once.

use crate::error::{FitsError, Result};
use crate::time_coordinates::mjd::{Expansion, Mjd};
use crate::time_coordinates::scale::Scale;
use crate::unit::SECONDS_PER_DAY;

/// A time axis (§9.6) of a header's WCS in its [`Frame`](crate::time::Frame), resolved
/// once from its `CTYPEi` and `CUNITi`, as [`Frame::time_axis`](crate::time::Frame::time_axis)
/// gives it: the MJD and scale of each world value along it, which
/// [`Wcs::axis_world`](crate::wcs::Wcs::axis_world) gives.
#[derive(Debug, Clone, PartialEq)]
pub struct Axis {
    pub(super) scale: Scale,
    pub(super) reference_mjd: Mjd,
    /// The length of the axis unit in seconds.
    pub(super) unit_seconds: f64,
    /// `TIMEOFFS`, in days.
    pub(super) clock_offset: Expansion,
}

impl Axis {
    /// The scale the axis's times are in: the head of its `CTYPEi`, or `TIMESYS` for
    /// `TIME`.
    pub const fn scale(&self) -> &Scale {
        &self.scale
    }

    /// The MJD, in [`Self::scale`], that `value` along the axis names in its unit
    /// relative to `MJDREF`: the reference epoch plus the value and the `TIMEOFFS` clock
    /// correction, each in its own unit. The products and the quotients keep their
    /// rounding errors, which the MJD's fraction takes in one last rounding. Errors with
    /// [`FitsError::TimeOutOfRange`] when the MJD leaves its range.
    pub fn mjd(&self, value: f64) -> Result<Mjd> {
        let days = Expansion::from(value)
            .scaled(self.unit_seconds)
            .divided(SECONDS_PER_DAY)
            .plus(self.clock_offset);
        self.reference_mjd
            .plus(days)
            .ok_or(FitsError::TimeOutOfRange)
    }
}

#[cfg(test)]
mod tests;
