//! The resolved celestial pair of a WCS: its axes, projection, parameters, and pole.

use crate::error::Result;
use crate::header::Header;
use crate::keyword::AltSuffix;
use crate::keyword::key;
use crate::wcs::celestial_pole::CelestialPole;
use crate::wcs::first_real;
use crate::wcs::projected_celestial_axes::ProjectedCelestialAxes;
use crate::wcs::projection::Projection;

/// The celestial half of a WCS once the header is resolved: which axes carry the
/// longitude and latitude, the projection and its `PVi_m` parameters, and the
/// native→celestial pole computed from the fiducial point.
#[derive(Debug, Clone)]
pub(super) struct CelestialTransform {
    pub(super) lng: usize,
    pub(super) lat: usize,
    pub(super) projection: Projection,
    /// The native→celestial pole, computed from the fiducial point.
    pub(super) pole: CelestialPole,
    pub(super) pv: [f64; 21],
}

impl CelestialTransform {
    /// Resolve the celestial pair's projection parameters and its native→celestial
    /// pole (CG 2002 §2.4), or `None` when the header declares no complete celestial
    /// pair.
    ///
    /// `unsupported_axes` gains the pair when the projection is present but cannot be
    /// evaluated, so a complete transform refuses rather than returning `NaN`.
    pub(super) fn from_header(
        header: &Header,
        a: AltSuffix,
        celestial_axes: Option<ProjectedCelestialAxes>,
        crval: &[f64],
        unsupported_axes: &mut Vec<usize>,
    ) -> Result<Option<CelestialTransform>> {
        let Some(axes) = celestial_axes else {
            return Ok(None);
        };
        let lng = axes.longitude;
        let lat = axes.latitude;
        let proj = axes.projection;
        let mut pv = proj.parameter_defaults();
        for (m, value) in pv.iter_mut().enumerate() {
            if let Some(header_value) = header.get_real(key!("PV{}_{m}{a}", lat + 1).as_str())? {
                *value = header_value;
            }
        }
        proj.validate_parameters(&pv)?;
        // A conic's mid-latitude θ_a = PVi_1 is mandatory and must be non-zero; θ_a = 0
        // (absent, or explicitly 0) is a degenerate cone (`1/tan 0`). Treat it like an
        // unimplemented projection — flag the axes so complete transforms fail rather
        // than returning NaN.
        if proj.is_conic() && pv[1] == 0.0 {
            unsupported_axes.push(lng);
            unsupported_axes.push(lat);
            unsupported_axes.sort_unstable();
            return Ok(None);
        }
        // Fiducial point: projection default, overridable by PVi_1a/PVi_2a on the
        // longitude axis (§8.3).
        let reference = proj.reference_point(&pv);
        let (mut phi0, mut theta0) = (reference.phi, reference.theta);
        if let Some(v) = header.get_real(key!("PV{}_1{a}", lng + 1).as_str())? {
            phi0 = v;
        }
        if let Some(v) = header.get_real(key!("PV{}_2{a}", lng + 1).as_str())? {
            theta0 = v;
        }
        let (alpha0, delta0) = (crval[lng], crval[lat]);
        // LONPOLE (= LONPOLEa or PVi_3a): default φ0 if δ0 ≥ θ0, else φ0 + 180°.
        let phip = first_real(
            header,
            key!("LONPOLE{a}").as_str(),
            key!("PV{}_3{a}", lng + 1).as_str(),
        )?
        .unwrap_or(if delta0 >= theta0 { phi0 } else { phi0 + 180.0 });
        // LATPOLE (= LATPOLEa or PVi_4a): default 90°.
        let thetap = first_real(
            header,
            key!("LATPOLE{a}").as_str(),
            key!("PV{}_4{a}", lng + 1).as_str(),
        )?
        .unwrap_or(90.0);
        Ok(Some(CelestialTransform {
            lng,
            lat,
            projection: proj,
            pole: CelestialPole::from_fiducial(phi0, theta0, alpha0, delta0, phip, thetap),
            pv,
        }))
    }
}
