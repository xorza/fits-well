//! The `CTYPEia` grammar (§8.2).

use crate::wcs::celestial_axis::CelestialAxis;

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
        let head = self.head;
        if head == "RA" || head.ends_with("LON") || (head.len() == 4 && head.ends_with("LN")) {
            Some(CelestialAxis::Longitude)
        } else if head == "DEC"
            || head.ends_with("LAT")
            || (head.len() == 4 && head.ends_with("LT"))
        {
            Some(CelestialAxis::Latitude)
        } else {
            None
        }
    }
}
