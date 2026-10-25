//! [`CelestialCoordinate`]: a celestial coordinate pair.

/// A celestial coordinate pair in degrees, as
/// [`NativeRotation::to_celestial`](crate::world_coordinates::native_rotation::NativeRotation::to_celestial)
/// yields.
#[derive(Debug, Clone, Copy)]
pub(super) struct CelestialCoordinate {
    pub(super) ra: f64,
    pub(super) dec: f64,
}
