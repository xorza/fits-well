//! The resolved celestial pair of a WCS: its axes, projection, parameters, and pole.

use crate::alternate::Alternate;
use crate::fits_error::Result;
use crate::header_model::Header;
use crate::keyword::Keyword;
use crate::world_coordinates::angle::DEGREE_TOLERANCE;
use crate::world_coordinates::celestial_coordinate::CelestialCoordinate;
use crate::world_coordinates::celestial_pole::{CelestialPole, Fiducial};
use crate::world_coordinates::distortion::Distortion;
use crate::world_coordinates::distortion::tpv::Tpv;
use crate::world_coordinates::header_read;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::native_rotation::NativeRotation;
use crate::world_coordinates::projected_celestial_axes::ProjectedCelestialAxes;
use crate::world_coordinates::projection::ProjectionParameters;

/// The celestial half of a WCS once the header is resolved: which axes carry the
/// longitude and latitude, the projection and its `PVi_m` parameters, and the
/// native→celestial pole computed from the fiducial point.
#[derive(Debug, Clone)]
pub(super) struct CelestialTransform {
    pub(super) lng: usize,
    pub(super) lat: usize,
    /// The rotation through the celestial pole computed from the fiducial point.
    pub(super) rotation: NativeRotation,
    pub(super) parameters: ProjectionParameters,
    /// The TPV polynomials between the matrix and the projection, for a `-TPV` pair.
    pub(super) tpv: Option<Tpv>,
    /// The projected coordinates of the fiducial point when `PVi_0a` on the longitude
    /// axis is nonzero, which moves the projection's origin there; zero otherwise
    /// (wcslib's `prjoff`).
    offset: [f64; 2],
}

/// The celestial pair a description declares: its axes, the reference point
/// `(α₀, δ₀)` its `CRVALia` give in degrees, and what its `PVi_m` carry.
#[derive(Debug, Clone, Copy)]
pub(super) struct CelestialPair {
    pub(super) axes: ProjectedCelestialAxes,
    pub(super) reference: CelestialCoordinate,
    pub(super) pv_meaning: PvMeaning,
}

impl CelestialTransform {
    /// Resolve the projection parameters of `pair` and its native→celestial pole
    /// (CG 2002 §2.4).
    pub(super) fn from_header(
        header: &Header,
        alt: Alternate,
        pair: CelestialPair,
    ) -> Result<Self> {
        let CelestialPair {
            axes,
            reference,
            pv_meaning,
        } = pair;
        let lng = axes.longitude;
        let lat = axes.latitude;
        let proj = axes.projection;
        let tpv = (axes.distortion == Some(Distortion::Tpv))
            .then(|| Tpv::from_header(header, alt, lng, lat))
            .transpose()?;
        // The celestial `PVi_m` are TPV terms on a `-TPV` pair, and SCAMP's leftovers
        // beside SIP; either way they set no projection parameter, fiducial point or pole.
        let read_pv = |keyword: Option<Keyword>| -> Result<Option<f64>> {
            match pv_meaning {
                PvMeaning::Projection if tpv.is_none() => header_read::real(header, keyword),
                PvMeaning::Projection | PvMeaning::Distortion => Ok(None),
            }
        };
        let mut pv = proj.parameter_defaults();
        for (m, value) in pv.iter_mut().enumerate() {
            if let Some(header_value) =
                read_pv(Keyword::fitting(format_args!("PV{}_{m}{alt}", lat + 1)))?
            {
                *value = header_value;
            }
        }
        let parameters = proj.parameters(pv)?;
        // Fiducial point: projection default, overridable by PVi_1a/PVi_2a on the
        // longitude axis (§8.3).
        let (mut phi0, mut theta0) = (0.0, proj.reference_latitude(&pv));
        if let Some(v) = read_pv(Keyword::fitting(format_args!("PV{}_1{alt}", lng + 1)))? {
            phi0 = v;
        }
        if let Some(v) = read_pv(Keyword::fitting(format_args!("PV{}_2{alt}", lng + 1)))? {
            theta0 = v;
        }
        // LONPOLE (= PVi_3a or LONPOLEa, the `PV` form winning, as wcslib's `wcsset` reads
        // them): default φ0 if δ0 ≥ θ0, else φ0 + 180°.
        let phip = match read_pv(Keyword::fitting(format_args!("PV{}_3{alt}", lng + 1)))? {
            Some(value) => Some(value),
            None => header_read::real(header, Keyword::fitting(format_args!("LONPOLE{alt}")))?,
        }
        .unwrap_or(if reference.dec >= theta0 {
            phi0
        } else {
            phi0 + 180.0
        });
        // LATPOLE (= PVi_4a or LATPOLEa): default 90°.
        let thetap = match read_pv(Keyword::fitting(format_args!("PV{}_4{alt}", lng + 1)))? {
            Some(value) => Some(value),
            None => header_read::real(header, Keyword::fitting(format_args!("LATPOLE{alt}")))?,
        }
        .unwrap_or(90.0);
        let offset = if read_pv(Keyword::fitting(format_args!("PV{}_0{alt}", lng + 1)))?
            .is_some_and(|flag| flag != 0.0)
        {
            let origin = parameters.project(NativeCoordinate::from_degrees(phi0, theta0))?;
            [origin.x, origin.y]
        } else {
            [0.0, 0.0]
        };
        let fiducial = Fiducial {
            phi: phi0,
            theta: theta0,
            celestial: reference,
        };
        Ok(Self {
            lng,
            lat,
            rotation: CelestialPole::from_fiducial(fiducial, phip, thetap)?.into(),
            parameters,
            tpv,
            offset,
        })
    }
}

impl CelestialTransform {
    /// The celestial coordinate of the pair's intermediate coordinates `(x, y)` (deg):
    /// through TPV on a `-TPV` pair, the projection's inverse and the rotation.
    pub(super) fn to_world(&self, x: f64, y: f64) -> Result<CelestialCoordinate> {
        let [x, y] = self.tpv.as_ref().map_or([x, y], |tpv| tpv.forward(x, y));
        let native = self
            .parameters
            .deproject(x + self.offset[0], y + self.offset[1])?;
        Ok(self.rotation.to_celestial(native))
    }

    /// The intermediate coordinates of the celestial coordinate `(ra, dec)` (deg), the
    /// inverse of [`Self::to_world`]. Errors with
    /// [`FitsError::WcsWorldOutOfDomain`](crate::fits_error::FitsError::WcsWorldOutOfDomain)
    /// for a coordinate that is not finite or whose
    /// latitude lies past ±90°.
    pub(super) fn to_intermediate(&self, ra: f64, dec: f64) -> Result<[f64; 2]> {
        if !ra.is_finite() || !(-90.0 - DEGREE_TOLERANCE..=90.0 + DEGREE_TOLERANCE).contains(&dec) {
            return Err(self.parameters.projection().world_domain_error());
        }
        let native = self.rotation.to_native(ra, dec.clamp(-90.0, 90.0));
        let projected = self.parameters.project(native)?;
        let [x, y] = [projected.x - self.offset[0], projected.y - self.offset[1]];
        self.tpv
            .as_ref()
            .map_or(Ok([x, y]), |tpv| tpv.inverse(x, y))
    }
}

/// What the celestial `PVi_m` of a pair carry besides TPV terms: projection parameters,
/// or a distortion's terms that set nothing (SCAMP's TPV terms beside SIP, which
/// astropy drops: `wcs.py`, `_fix_scamp`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PvMeaning {
    Projection,
    Distortion,
}

#[cfg(test)]
mod tests {
    use crate::fits_error::FitsError;
    use crate::internals::tolerance;
    use crate::internals::tolerance::Tolerance;
    use crate::internals::wcs_header::WcsHeader;
    use crate::world_coordinates::Wcs;
    use crate::world_coordinates::celestial_pole::CelestialPole;
    use crate::world_coordinates::celestial_transform::CelestialTransform;
    use crate::world_coordinates::projection::Projection;
    use std::assert_matches;

    fn transform(projection: &str, keywords: &[(&str, f64)]) -> CelestialTransform {
        let header = WcsHeader::celestial(projection, [1.0, 1.0], [45.0, 30.0], [1e-3, 1e-3])
            .image_header_with(keywords);
        Wcs::from_header(&header, None).unwrap().celestial.unwrap()
    }

    /// LONPOLE defaults to φ₀ when δ₀ ≥ θ₀ and to φ₀ + 180° otherwise: a zenithal
    /// fiducial point (θ₀ = 90° > δ₀ = 30°) takes 180°, a cylindrical one (θ₀ = 0) 0°.
    /// `PV1_3` wins over `LONPOLE`, as wcslib's `wcsset` reads them; `PV1_4` stands in for
    /// `LATPOLE` and picks the southern pole of CAR, (45°, −60°).
    #[test]
    fn pole_keywords_and_their_pv_forms_set_the_rotation() {
        let pole = |projection, keywords| transform(projection, keywords).rotation.pole;
        assert_eq!(pole("TAN", &[]).lonpole, 180.0);
        assert_eq!(pole("CAR", &[]).lonpole, 0.0);
        assert_eq!(pole("TAN", &[("PV1_3", 170.0)]).lonpole, 170.0);
        assert_eq!(
            pole("TAN", &[("PV1_3", 170.0), ("LONPOLE", 160.0)]).lonpole,
            170.0
        );
        assert_eq!(
            pole("CAR", &[("PV1_4", -90.0)]),
            CelestialPole {
                ra: 45.0,
                dec: -60.0,
                lonpole: 0.0
            }
        );
        // `PV1_1` and `PV1_2` move the fiducial point: TAN's at θ₀ = 0 is the CAR case.
        assert_eq!(
            pole("TAN", &[("PV1_2", 0.0), ("LONPOLE", 0.0)]),
            pole("CAR", &[])
        );
    }

    /// A nonzero `PV1_0` moves the projection's origin to the fiducial point (wcslib's
    /// `prjoff`): CAR's fiducial moved to (φ₀, θ₀) = (10°, 20°) then maps the reference
    /// point to the intermediate origin, where without the flag the origin is native
    /// (0°, 0°) and the reference point projects to CAR's (x, y) = (φ₀, θ₀) = (10°, 20°).
    #[test]
    fn nonzero_pv_0_moves_the_projection_origin_to_the_fiducial_point() {
        let fiducial = [("PV1_1", 10.0), ("PV1_2", 20.0)];
        let moved = transform("CAR", &[fiducial[0], fiducial[1], ("PV1_0", 1.0)]);
        let world = moved.to_world(0.0, 0.0).unwrap();
        tolerance::assert_close(
            &[world.ra, world.dec],
            &[45.0, 30.0],
            Tolerance::abs(
                4.0 * 32.0 * f64::EPSILON,
                "a few ulps of 45°, whose ulp is 32 ε",
            ),
            "fiducial at the origin",
        );
        tolerance::assert_close(
            &moved.to_intermediate(45.0, 30.0).unwrap(),
            &[0.0, 0.0],
            Tolerance::abs(
                4.0 * 16.0 * f64::EPSILON,
                "a few ulps of the origin (10°, 20°) it is taken from",
            ),
            "reference point at the origin",
        );
        let unmoved = transform("CAR", &fiducial);
        tolerance::assert_close(
            &unmoved.to_intermediate(45.0, 30.0).unwrap(),
            &[10.0, 20.0],
            Tolerance::abs(
                4.0 * 16.0 * f64::EPSILON,
                "a few ulps of 20°, whose ulp is 16 ε",
            ),
            "fiducial off the origin",
        );
    }

    /// A latitude past ±90° by more than the degree tolerance, or a coordinate that is not
    /// finite, has no intermediate point; one within the tolerance is the pole.
    #[test]
    fn intermediate_coordinates_take_latitudes_to_the_poles() {
        let tan = transform("TAN", &[]);
        let pole = tan.to_intermediate(45.0, 90.0).unwrap();
        assert_eq!(tan.to_intermediate(45.0, 90.0 + 1e-13).unwrap(), pole);
        for [ra, dec] in [
            [45.0, 90.0 + 1e-11],
            [45.0, -90.0 - 1e-11],
            [f64::NAN, 30.0],
            [f64::INFINITY, 30.0],
        ] {
            assert_matches!(
                tan.to_intermediate(ra, dec),
                Err(FitsError::WcsWorldOutOfDomain {
                    projection: Projection::Tan
                }),
                "({ra}, {dec})"
            );
        }
        // Each world coordinate is below 64°, whose ulp is 32 ε.
        let world = tan.to_world(0.25, -0.5).unwrap();
        tolerance::assert_close(
            &tan.to_intermediate(world.ra, world.dec).unwrap(),
            &[0.25, -0.5],
            Tolerance::abs(
                2.0 * 32.0 * f64::EPSILON,
                "two ulps of the world coordinate",
            ),
            "round trip",
        );
    }
}
