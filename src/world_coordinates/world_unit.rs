//! [`WorldUnit`]: the unit an axis's world coordinates come out in.

use crate::error::{FitsError, Result};
use crate::unit;
use crate::world_coordinates::axis_transform;
use crate::world_coordinates::ctype::Ctype;

/// The unit an axis's world coordinates come out in, and the factor to it from the
/// declared `CUNITi`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct WorldUnit {
    pub(super) scale: f64,
    /// The unit, where it is not the declared one.
    pub(super) unit: Option<&'static str>,
}

impl WorldUnit {
    /// The declared `CUNITi` itself.
    pub(super) const DECLARED: Self = Self {
        scale: 1.0,
        unit: None,
    };

    /// The unit of an axis of `ctype` that an algorithm evaluates: degrees for a
    /// celestial axis (§8.2), the Table-25 default for a spectral one (§8.4), and the
    /// declared unit for any other.
    pub(super) fn of(ctype: Ctype<'_>, cunit: &str) -> Result<Self> {
        if let Some(unit) = axis_transform::spectral_unit(ctype.head, cunit)? {
            return Ok(unit);
        }
        if ctype.celestial_axis().is_some() {
            return Ok(Self {
                scale: angle_scale(cunit)?,
                unit: Some("deg"),
            });
        }
        Ok(Self::DECLARED)
    }
}

fn angle_scale(unit: &str) -> Result<f64> {
    if unit.trim().is_empty() {
        return Ok(1.0);
    }
    unit::resolve(unit, unit::ANGLE).ok_or_else(|| FitsError::InvalidUnit {
        unit: unit.to_string(),
        expected: "an angle unit",
    })
}
