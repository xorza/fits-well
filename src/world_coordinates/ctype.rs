//! The `CTYPEia` grammar (§8.2).

use crate::world_coordinates::celestial_axis::CelestialAxis;
use crate::world_coordinates::distortion::Distortion;
use crate::world_coordinates::projection::Projection;

/// A `CTYPEia` value split into its parts: the coordinate-type name, the algorithm or
/// projection code the `-` padding separates from it (`RA---TAN` → `RA` and `TAN`), and
/// a distortion code after the projection in the 12-character 4-3-3 form
/// (`RA---TAN-SIP`), as wcslib excises one from any projection (`wcs.c`, `wcsset`). A
/// value with no hyphen-delimited suffix is a bare coordinate type.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Ctype<'a> {
    /// The whole value, as the header holds it.
    pub(crate) value: &'a str,
    /// The coordinate-type name, blanks trimmed.
    pub(crate) head: &'a str,
    /// The trailing algorithm code; `None` for a bare head.
    pub(crate) algorithm: Option<&'a str>,
    /// The distortion code of a 12-character value; `None` for another.
    pub(crate) distortion: Option<&'a str>,
}

impl<'a> Ctype<'a> {
    pub(crate) fn parse(value: &'a str) -> Self {
        let trimmed = value.trim_end();
        let (base, distortion) = if trimmed.len() == 12 && trimmed.as_bytes()[8] == b'-' {
            (&trimmed[..8], Some(&trimmed[9..]))
        } else {
            (value, None)
        };
        Ctype {
            value,
            head: base.split('-').next().unwrap_or("").trim(),
            algorithm: base
                .rsplit_once('-')
                .map(|(_, code)| code.trim_end())
                .filter(|code| !code.is_empty()),
            distortion,
        }
    }

    /// The projection a celestial axis's algorithm names, with the distortion its code
    /// or its algorithm names: `TPV` is `TAN` with the TPV polynomial. `None` when either
    /// is one this crate does not evaluate.
    pub(super) fn projection(self) -> Option<(Projection, Option<Distortion>)> {
        let (projection, algorithm_distortion) = match self.algorithm? {
            "TPV" => (Projection::Tan, Some(Distortion::Tpv)),
            code => (Projection::from_code(code)?, None),
        };
        let distortion = match self.distortion {
            None => algorithm_distortion,
            Some(code) if algorithm_distortion.is_none() => Some(Distortion::from_code(code)?),
            Some(_) => return None,
        };
        Some((projection, distortion))
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
