//! Which celestial coordinate a `CTYPEia` axis carries.

/// Which celestial coordinate an axis carries, as
/// [`Ctype::celestial_axis`](crate::world_coordinates::ctype::Ctype::celestial_axis) classifies it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum CelestialAxis {
    Longitude,
    Latitude,
}
