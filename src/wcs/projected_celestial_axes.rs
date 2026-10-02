//! The celestial longitude/latitude axis pair and the projection they share.

use crate::error::FitsError;
use crate::error::Result;
use crate::wcs::celestial_axis::CelestialAxis;
use crate::wcs::ctype::Ctype;
use crate::wcs::projection::Projection;

/// A celestial longitude/latitude axis pair, before their projection is resolved.
#[derive(Debug, Clone, Copy)]
pub(super) struct CelestialAxisPair {
    pub(super) longitude: usize,
    pub(super) latitude: usize,
}

/// A celestial axis pair whose shared `CTYPE` algorithm is a projection this crate
/// evaluates — the form the transform pipeline needs.
#[derive(Debug, Clone, Copy)]
pub(super) struct ProjectedCelestialAxes {
    pub(super) longitude: usize,
    pub(super) latitude: usize,
    pub(super) projection: Projection,
}

impl CelestialAxisPair {
    /// The first longitude axis of `ctype` that has a latitude axis of its own system,
    /// paired with the first such latitude axis — or `None`. A celestial axis left over
    /// is a coordinate the WCS cannot evaluate.
    pub(super) fn find(ctype: &[String]) -> Option<CelestialAxisPair> {
        let systems: Vec<_> = ctype
            .iter()
            .map(|ctype| Ctype::parse(ctype).celestial_system())
            .collect();
        systems.iter().enumerate().find_map(|(longitude, system)| {
            let system = (*system)?;
            if system.axis != CelestialAxis::Longitude {
                return None;
            }
            let latitude = systems.iter().position(|other| {
                other.is_some_and(|other| {
                    other.axis == CelestialAxis::Latitude && other.system == system.system
                })
            })?;
            Some(CelestialAxisPair {
                longitude,
                latitude,
            })
        })
    }

    pub(super) fn contains(self, axis: usize) -> bool {
        axis == self.longitude || axis == self.latitude
    }
}

impl ProjectedCelestialAxes {
    /// Locate the celestial longitude/latitude axis pair and their shared projection,
    /// or `None` if the header has no complete supported pair. Errors if the two axes
    /// declare different projection codes.
    pub(super) fn find(ctype: &[String]) -> Result<Option<ProjectedCelestialAxes>> {
        let Some(pair) = CelestialAxisPair::find(ctype) else {
            return Ok(None);
        };
        let longitude = Ctype::parse(&ctype[pair.longitude]).algorithm;
        if longitude != Ctype::parse(&ctype[pair.latitude]).algorithm {
            return Err(FitsError::ConflictingWcsKeywords {
                detail: "celestial longitude and latitude axes declare different projections",
            });
        }
        Ok(longitude
            .and_then(Projection::from_code)
            .map(|projection| ProjectedCelestialAxes {
                longitude: pair.longitude,
                latitude: pair.latitude,
                projection,
            }))
    }
}
