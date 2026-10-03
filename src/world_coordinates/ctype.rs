//! The `CTYPEia` grammar (§8.2).

use crate::world_coordinates::celestial_axis::CelestialAxis;

/// A `CTYPEia` value split into its two parts: the coordinate-type name, and the
/// algorithm or projection code the `-` padding separates from it (`RA---TAN` → `RA`
/// and `TAN`). A value with no hyphen-delimited suffix is a bare coordinate type.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctype<'a> {
    /// The coordinate-type name, blanks trimmed.
    pub(crate) head: &'a str,
    /// The trailing algorithm code; `None` for a bare head.
    pub(crate) algorithm: Option<&'a str>,
}

impl<'a> Ctype<'a> {
    pub(crate) fn parse(value: &'a str) -> Ctype<'a> {
        Ctype {
            head: value.split('-').next().unwrap_or("").trim(),
            algorithm: value
                .rsplit_once('-')
                .map(|(_, code)| code.trim_end())
                .filter(|code| !code.is_empty()),
        }
    }

    /// The celestial coordinate this axis carries (§8.2): `RA` and the `xLON`/`yzLN`
    /// forms are longitudes; `DEC` and `xLAT`/`yzLT` are latitudes; `None` for any
    /// non-celestial axis.
    pub(super) fn celestial_axis(self) -> Option<CelestialAxis> {
        self.celestial_system().map(|system| system.axis)
    }

    /// The celestial coordinate this axis carries and the system it belongs to: the head
    /// without its `LON`/`LAT`/`LN`/`LT` suffix, so `GLON` and `GLAT` share `G` and `HPLN`
    /// and `HPLT` share `HP`; `RA` and `DEC` share the equatorial system.
    pub(super) fn celestial_system(self) -> Option<CelestialSystem<'a>> {
        let head = self.head;
        let (axis, system) = if head == "RA" {
            (CelestialAxis::Longitude, "RA/DEC")
        } else if head == "DEC" {
            (CelestialAxis::Latitude, "RA/DEC")
        } else if let Some(system) = head.strip_suffix("LON") {
            (CelestialAxis::Longitude, system)
        } else if let Some(system) = head.strip_suffix("LAT") {
            (CelestialAxis::Latitude, system)
        } else if head.len() == 4 && head.ends_with("LN") {
            (CelestialAxis::Longitude, &head[..2])
        } else if head.len() == 4 && head.ends_with("LT") {
            (CelestialAxis::Latitude, &head[..2])
        } else {
            return None;
        };
        Some(CelestialSystem { axis, system })
    }
}

/// One axis of a celestial coordinate system, as [`Ctype::celestial_system`] reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct CelestialSystem<'a> {
    pub(super) axis: CelestialAxis,
    /// What the longitude and latitude of one system share in their names.
    pub(super) system: &'a str,
}
