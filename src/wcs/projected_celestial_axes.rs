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
    /// The first longitude axis and the first latitude axis of `ctype`, or `None`
    /// unless both are present.
    pub(super) fn find(ctype: &[String]) -> Option<CelestialAxisPair> {
        let mut lng = None;
        let mut lat = None;
        for (i, t) in ctype.iter().enumerate() {
            match Ctype::parse(t).celestial_axis() {
                Some(CelestialAxis::Longitude) => lng = lng.or(Some(i)),
                Some(CelestialAxis::Latitude) => lat = lat.or(Some(i)),
                None => {}
            }
        }
        let (Some(lng), Some(lat)) = (lng, lat) else {
            return None;
        };
        Some(CelestialAxisPair {
            longitude: lng,
            latitude: lat,
        })
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
