//! `HEALPix` projection from Calabretta & Roukema (2007).

use crate::fits_error::Result;
use crate::world_coordinates::angle::DEGREE_TOLERANCE;
use crate::world_coordinates::angle::R2D;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::ProjectedCoordinate;
use crate::world_coordinates::projection::Projection;

/// HEALPix's kernel: the constants of `H = PVi_1` and `K = PVi_2`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct Healpix {
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

impl Healpix {
    pub(super) fn new(pv: &[f64; 21]) -> Self {
        let h = pv[1];
        let k = pv[2];
        debug_assert!(h.is_finite() && h > 0.0 && k.is_finite() && k > 0.0);
        Self {
            k,
            h_odd: (h.round() as i64).rem_euclid(2),
            k_odd: (k.round() as i64).rem_euclid(2) != 0,
            equatorial_sine: (k - 1.0) / k,
            y_per_sine: 90.0 * k / h,
            polar_y0: f64::midpoint(k, 1.0),
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

    pub(super) fn deproject(
        self,
        projection: Projection,
        x: f64,
        y: f64,
    ) -> Result<NativeCoordinate> {
        if !x.is_finite() || !y.is_finite() {
            return Err(projection.domain_error());
        }
        let parameters = self;
        let abs_y = y.abs();
        if abs_y <= parameters.transition_y + DEGREE_TOLERANCE {
            let theta = projection.checked_asin(y / parameters.y_per_sine)? * R2D;
            return projection.native_coordinate(x, theta);
        }

        let maximum_y = parameters.facet_half_width * parameters.polar_y0;
        if abs_y > maximum_y + DEGREE_TOLERANCE {
            return Err(projection.domain_error());
        }

        let sigma = (parameters.polar_y0 - abs_y / parameters.facet_half_width).max(0.0);
        let mut delta = x - parameters.facet_center(x);
        if !parameters.k_odd && y < 0.0 {
            delta = parameters.offset_southern_facet(x, delta);
        }

        let phi = if sigma == 0.0 {
            if delta.abs() > DEGREE_TOLERANCE {
                return Err(projection.domain_error());
            }
            x
        } else {
            let facet_position = delta / sigma;
            if facet_position.abs() > parameters.facet_half_width + DEGREE_TOLERANCE {
                return Err(projection.domain_error());
            }
            x + facet_position - delta
        };
        // 1 − sin θ = σ²/K, so cos θ = (σ/√K)·√(2 − σ²/K): no cancellation next to the
        // pole, where sin θ = 1 − σ²/K rounds.
        let depth = sigma * sigma / parameters.k;
        debug_assert!(depth <= 1.0, "the polar caps lie where σ ≤ 1: {depth}");
        let cos_theta = sigma / parameters.k.sqrt() * (2.0 - depth).sqrt();
        let phi = projection.native_longitude(phi)?;
        Ok(NativeCoordinate::from_sin_cos(
            phi,
            (1.0 - depth).copysign(y),
            cos_theta,
        ))
    }

    pub(super) fn project(
        self,
        projection: Projection,
        native: NativeCoordinate,
    ) -> Result<ProjectedCoordinate> {
        let parameters = self;
        let (phi, sin_theta, cos_theta) = (native.phi, native.sin_theta(), native.cos_theta());
        if sin_theta.abs() <= parameters.equatorial_sine {
            return projection.projected_coordinate(phi, parameters.y_per_sine * sin_theta);
        }

        // σ = √(K(1 − |sin θ|)) with 1 − |sin θ| = cos²θ / (1 + |sin θ|).
        let sigma = cos_theta * (parameters.k / (1.0 + sin_theta.abs())).sqrt();
        let mut delta = phi - parameters.facet_center(phi);
        if !parameters.k_odd && sin_theta < 0.0 {
            delta = parameters.offset_southern_facet(phi, delta);
        }
        let mut x = phi + delta * (sigma - 1.0);
        let y = parameters.facet_half_width * (parameters.polar_y0 - sigma) * sin_theta.signum();
        if x > 180.0 {
            x = 360.0 - x;
        }
        projection.projected_coordinate(x, y)
    }
}
