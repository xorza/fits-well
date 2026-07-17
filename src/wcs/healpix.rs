//! HEALPix projection from Calabretta & Roukema (2007).

use crate::error::Result;
use crate::wcs::D2R;
use crate::wcs::DOMAIN_TOLERANCE;
use crate::wcs::NativeCoordinate;
use crate::wcs::ProjectedCoordinate;
use crate::wcs::Projection;
use crate::wcs::R2D;

#[derive(Debug, Clone, Copy)]
struct Parameters {
    k: f64,
    h_odd: i64,
    k_odd: bool,
    equatorial_sine: f64,
    y_per_sine: f64,
    polar_y0: f64,
    transition_y: f64,
    facet_half_width: f64,
    facet_index_scale: f64,
}

impl Parameters {
    fn new(pv: &[f64; 21]) -> Parameters {
        let h = pv[1];
        let k = pv[2];
        debug_assert!(h.is_finite() && h > 0.0 && k.is_finite() && k > 0.0);
        Parameters {
            k,
            h_odd: (h.round() as i64).rem_euclid(2),
            k_odd: (k.round() as i64).rem_euclid(2) != 0,
            equatorial_sine: (k - 1.0) / k,
            y_per_sine: 90.0 * k / h,
            polar_y0: (k + 1.0) / 2.0,
            transition_y: 90.0 * (k - 1.0) / h,
            facet_half_width: 180.0 / h,
            facet_index_scale: h / 360.0,
        }
    }

    fn facet_center(self, x: f64) -> f64 {
        -180.0
            + (2.0 * ((x + 180.0) * self.facet_index_scale).floor() + 1.0) * self.facet_half_width
    }

    fn offset_southern_facet(self, longitude: f64, delta: f64) -> f64 {
        let facet = (longitude / self.facet_half_width).floor() as i64 + self.h_odd;
        if facet.rem_euclid(2) != 0 {
            delta - self.facet_half_width
        } else {
            delta + self.facet_half_width
        }
    }
}

pub(crate) fn deproject(
    projection: Projection,
    x: f64,
    y: f64,
    pv: &[f64; 21],
) -> Result<NativeCoordinate> {
    if !x.is_finite() || !y.is_finite() {
        return Err(projection.domain_error());
    }
    let parameters = Parameters::new(pv);
    let abs_y = y.abs();
    if abs_y <= parameters.transition_y + DOMAIN_TOLERANCE {
        let theta = projection.checked_asin(y / parameters.y_per_sine)? * R2D;
        return projection.native_coordinate(x, theta);
    }

    let maximum_y = parameters.facet_half_width * parameters.polar_y0;
    if abs_y > maximum_y + DOMAIN_TOLERANCE {
        return Err(projection.domain_error());
    }

    let sigma = (parameters.polar_y0 - abs_y / parameters.facet_half_width).max(0.0);
    let mut delta = x - parameters.facet_center(x);
    if !parameters.k_odd && y < 0.0 {
        delta = parameters.offset_southern_facet(x, delta);
    }

    let phi = if sigma == 0.0 {
        if delta.abs() > DOMAIN_TOLERANCE {
            return Err(projection.domain_error());
        }
        x
    } else {
        let facet_position = delta / sigma;
        if facet_position.abs() > parameters.facet_half_width + DOMAIN_TOLERANCE {
            return Err(projection.domain_error());
        }
        x + facet_position - delta
    };
    let sin_theta = 1.0 - sigma * sigma / parameters.k;
    let theta = projection.checked_asin(sin_theta)? * R2D * y.signum();
    projection.native_coordinate(phi, theta)
}

pub(crate) fn project(
    projection: Projection,
    phi: f64,
    theta: f64,
    pv: &[f64; 21],
) -> Result<ProjectedCoordinate> {
    let parameters = Parameters::new(pv);
    let sin_theta = (theta * D2R).sin();
    if sin_theta.abs() <= parameters.equatorial_sine {
        return projection.projected_coordinate(phi, parameters.y_per_sine * sin_theta);
    }

    let sigma = (parameters.k * (1.0 - sin_theta.abs())).sqrt();
    let mut delta = phi - parameters.facet_center(phi);
    if !parameters.k_odd && theta < 0.0 {
        delta = parameters.offset_southern_facet(phi, delta);
    }
    let mut x = phi + delta * (sigma - 1.0);
    let y = parameters.facet_half_width * (parameters.polar_y0 - sigma) * theta.signum();
    if x > 180.0 {
        x = 360.0 - x;
    }
    projection.projected_coordinate(x, y)
}
