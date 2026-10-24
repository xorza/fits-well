//! [`AxisPlan`]: one axis of a WCS, resolved once from its keywords.

use crate::world_coordinates::axis_transform::AxisTransform;

/// What an axis is, from its `CTYPEi` parsed once.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum AxisKind {
    /// One axis of a celestial pair, which a projection maps.
    Celestial,
    /// A `-TAB` axis, whose coordinates a binary table holds.
    Tabular,
    /// Any other: linear, logarithmic or spectral.
    Other,
}

/// One axis of a WCS: what it is, how its own step evaluates, and the unit its world
/// coordinates come out in.
#[derive(Debug, Clone)]
pub(super) struct AxisPlan {
    pub(super) kind: AxisKind,
    pub(super) transform: AxisTransform,
    /// The unit of the axis's world coordinates where it is not the declared `CUNITi`:
    /// degrees for a celestial axis, the Table-25 default for a spectral one, also when
    /// a table holds its coordinates.
    pub(super) world_unit: Option<&'static str>,
}
