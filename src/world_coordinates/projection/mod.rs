//! Celestial projection classification and closed-set transform kernels.
//!
//! Each family's kernel holds the constants its parameters give, built once in
//! [`Projection::parameters`], and its forward and inverse sit side by side in its
//! file.

use std::f64::consts::PI;
use std::fmt;

use crate::fits_error::{FitsError, Result, WcsDefect};
use crate::world_coordinates::angle;
use crate::world_coordinates::angle::D2R;
use crate::world_coordinates::angle::DEGREE_TOLERANCE;
use crate::world_coordinates::angle::UNIT_TOLERANCE;
use crate::world_coordinates::native_coordinate::NativeCoordinate;
use crate::world_coordinates::projection::conic::{Conic, ConicKind};
use crate::world_coordinates::projection::cube::Cube;
use crate::world_coordinates::projection::equatorial::{Bon, Equatorial};
use crate::world_coordinates::projection::healpix::Healpix;
use crate::world_coordinates::projection::perspective::{Azp, Perspective, Sin, Szp};
use crate::world_coordinates::projection::zenithal::{Air, Zenithal, Zpn, ZpnBranch};

mod conic;
mod cube;
mod equatorial;
mod healpix;
mod perspective;
mod zenithal;

/// A celestial projection algorithm — the 3-letter `CTYPE` code.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Projection {
    /// `TAN` — gnomonic (zenithal).
    Tan,
    /// `SIN` — orthographic/slant (zenithal).
    Sin,
    /// `ARC` — zenithal equidistant.
    Arc,
    /// `STG` — stereographic (zenithal).
    Stg,
    /// `ZEA` — zenithal equal-area.
    Zea,
    /// `CAR` — plate carrée (cylindrical).
    Car,
    /// `CEA` — cylindrical equal-area.
    Cea,
    /// `MER` — Mercator (cylindrical).
    Mer,
    /// `SFL` — Sanson–Flamsteed (pseudo-cylindrical).
    Sfl,
    /// `AIT` — Hammer–Aitoff (all-sky, pseudo-cylindrical).
    Ait,
    /// `MOL` — Mollweide (all-sky, pseudo-cylindrical).
    Mol,
    /// `ZPN` — zenithal polynomial (`PVi_m` coefficients).
    Zpn,
    /// `CYP` — cylindrical perspective (`μ = PVi_1`, `λ = PVi_2`).
    Cyp,
    /// `PAR` — parabolic (pseudo-cylindrical).
    Par,
    /// `COP` — conic perspective (`θ_a = PVi_1`, `η = PVi_2`).
    Cop,
    /// `COE` — conic equal-area.
    Coe,
    /// `COD` — conic equidistant.
    Cod,
    /// `COO` — conic orthomorphic.
    Coo,
    /// `BON` — Bonne's equal-area (pseudo-conic, `θ₁ = PVi_1`).
    Bon,
    /// `AIR` — Airy (zenithal, minimum-error; `θ_b = PVi_1`).
    Air,
    /// `AZP` — zenithal perspective (`μ = PVi_1`, tilt `γ = PVi_2`).
    Azp,
    /// `PCO` — polyconic.
    Pco,
    /// `SZP` — slant zenithal perspective (`μ = PVi_1`, `φc = PVi_2`, `θc = PVi_3`).
    Szp,
    /// `TSC` — tangential spherical cube.
    Tsc,
    /// `CSC` — COBE quadrilateralized spherical cube.
    Csc,
    /// `QSC` — quadrilateralized spherical cube.
    Qsc,
    /// `HPX` — `HEALPix` (`H = PVi_1`, `K = PVi_2`).
    Hpx,
}

/// The projection family, which fixes the fiducial point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Family {
    /// `θ₀ = 90°`: radial about the native pole, the perspectives and `SIN` among them.
    Zenithal,
    /// `θ₀ = θ_a = PVi_1`.
    Conic,
    /// `θ₀ = 0°`: cylindrical, pseudo-cylindrical, polyconic, Bonne, the spherical
    /// cubes and `HEALPix`.
    Equatorial,
}

#[derive(Debug, Clone, Copy)]
struct Entry {
    projection: Projection,
    code: &'static str,
    family: Family,
}

/// Each projection's `CTYPE` code and family, in declaration order, so the variant
/// indexes its own entry.
const TABLE: [Entry; 27] = {
    const fn entry(projection: Projection, code: &'static str, family: Family) -> Entry {
        Entry {
            projection,
            code,
            family,
        }
    }
    use Family::{Conic as Cone, Equatorial as Equator, Zenithal as Pole};
    let table = [
        entry(Projection::Tan, "TAN", Pole),
        entry(Projection::Sin, "SIN", Pole),
        entry(Projection::Arc, "ARC", Pole),
        entry(Projection::Stg, "STG", Pole),
        entry(Projection::Zea, "ZEA", Pole),
        entry(Projection::Car, "CAR", Equator),
        entry(Projection::Cea, "CEA", Equator),
        entry(Projection::Mer, "MER", Equator),
        entry(Projection::Sfl, "SFL", Equator),
        entry(Projection::Ait, "AIT", Equator),
        entry(Projection::Mol, "MOL", Equator),
        entry(Projection::Zpn, "ZPN", Pole),
        entry(Projection::Cyp, "CYP", Equator),
        entry(Projection::Par, "PAR", Equator),
        entry(Projection::Cop, "COP", Cone),
        entry(Projection::Coe, "COE", Cone),
        entry(Projection::Cod, "COD", Cone),
        entry(Projection::Coo, "COO", Cone),
        entry(Projection::Bon, "BON", Equator),
        entry(Projection::Air, "AIR", Pole),
        entry(Projection::Azp, "AZP", Pole),
        entry(Projection::Pco, "PCO", Equator),
        entry(Projection::Szp, "SZP", Pole),
        entry(Projection::Tsc, "TSC", Equator),
        entry(Projection::Csc, "CSC", Equator),
        entry(Projection::Qsc, "QSC", Equator),
        entry(Projection::Hpx, "HPX", Equator),
    ];
    let mut index = 0;
    while index < table.len() {
        assert!(
            table[index].projection as usize == index,
            "the table follows the declaration order"
        );
        index += 1;
    }
    table
};

impl fmt::Display for Projection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.code())
    }
}

impl Projection {
    pub(super) fn from_code(code: &str) -> Option<Self> {
        TABLE
            .iter()
            .find(|entry| entry.code == code)
            .map(|entry| entry.projection)
    }

    /// The three-letter `CTYPEi` code, `TAN` for `Tan`.
    pub const fn code(self) -> &'static str {
        TABLE[self as usize].code
    }

    const fn family(self) -> Family {
        TABLE[self as usize].family
    }

    pub(super) const fn parameter_defaults(self) -> [f64; 21] {
        let mut pv = [0.0; 21];
        match self {
            Self::Air => pv[1] = 90.0,
            Self::Cyp => {
                pv[1] = 1.0;
                pv[2] = 1.0;
            }
            Self::Cea => pv[1] = 1.0,
            Self::Szp => pv[3] = 90.0,
            Self::Hpx => {
                pv[1] = 4.0;
                pv[2] = 3.0;
            }
            _ => {}
        }
        pv
    }

    /// The native latitude θ₀ of the fiducial point in degrees, whose longitude φ₀ is 0
    /// for every projection: 90 for a zenithal one, `θ_a = PVi_1` for a conic, else 0.
    pub(super) const fn reference_latitude(self, pv: &[f64; 21]) -> f64 {
        match self.family() {
            Family::Zenithal => 90.0,
            Family::Conic => pv[1],
            Family::Equatorial => 0.0,
        }
    }

    /// Validates `pv` for this projection and builds its kernel's constants from it once.
    pub(super) fn parameters(self, pv: [f64; 21]) -> Result<ProjectionParameters> {
        let degenerate = || FitsError::InvalidWcs {
            defect: WcsDefect::DegenerateProjection { projection: self },
        };
        let invalid = match self {
            // wcslib's `ceaset`: 0 < λ ≤ 1.
            Self::Cea => !(pv[1] > 0.0 && pv[1] <= 1.0),
            // A conic's mid-latitude θ_a = `PVi_1` must be nonzero, a cone of `1/tan 0`
            // having no apex; COE's constant (sinθ₁ + sinθ₂)/2 = sinθ_a·cosη is 0 at
            // η = ±90° too (wcslib's `coeset`).
            Self::Cop | Self::Cod | Self::Coo => pv[1] == 0.0,
            Self::Coe => pv[1] == 0.0 || angle::cosd(pv[2]) == 0.0,
            Self::Cyp => pv[2] == 0.0 || pv[1] + pv[2] == 0.0,
            Self::Hpx => !pv[1].is_finite() || pv[1] <= 0.0 || !pv[2].is_finite() || pv[2] <= 0.0,
            // wcslib's `airset`: at θ_b = −90° the constant K is 0 and the radius has no
            // bound to bracket its inverse.
            Self::Air => pv[1] <= -90.0,
            _ => false,
        };
        if invalid {
            return Err(degenerate());
        }
        let zpn = if self == Self::Zpn {
            ZpnBranch::new(&pv).ok_or_else(degenerate)?
        } else {
            ZpnBranch::UNUSED
        };
        Ok(self.kernel(pv, zpn))
    }

    /// The kernel of `pv`, with `zpn` the branch of a ZPN polynomial.
    fn kernel(self, pv: [f64; 21], zpn: ZpnBranch) -> ProjectionParameters {
        let kernel = match self {
            Self::Tan => Kernel::Zenithal(Zenithal::Tan),
            Self::Arc => Kernel::Zenithal(Zenithal::Arc),
            Self::Stg => Kernel::Zenithal(Zenithal::Stg),
            Self::Zea => Kernel::Zenithal(Zenithal::Zea),
            Self::Zpn => Kernel::Zenithal(Zenithal::Zpn(Zpn {
                coefficients: pv,
                branch: zpn,
            })),
            Self::Air => Kernel::Zenithal(Zenithal::Air(Air::new(pv[1]))),
            Self::Sin => Kernel::Perspective(Perspective::Sin(Sin::new(&pv))),
            Self::Azp => Kernel::Perspective(Perspective::Azp(Azp::new(&pv))),
            Self::Szp => Kernel::Perspective(Perspective::Szp(Szp::new(&pv))),
            Self::Cop => Kernel::Conic(Conic::new(ConicKind::Cop, &pv)),
            Self::Coe => Kernel::Conic(Conic::new(ConicKind::Coe, &pv)),
            Self::Cod => Kernel::Conic(Conic::new(ConicKind::Cod, &pv)),
            Self::Coo => Kernel::Conic(Conic::new(ConicKind::Coo, &pv)),
            Self::Car => Kernel::Equatorial(Equatorial::Car),
            Self::Cea => Kernel::Equatorial(Equatorial::Cea { lambda: pv[1] }),
            Self::Mer => Kernel::Equatorial(Equatorial::Mer),
            Self::Sfl => Kernel::Equatorial(Equatorial::Sfl),
            Self::Ait => Kernel::Equatorial(Equatorial::Ait),
            Self::Mol => Kernel::Equatorial(Equatorial::Mol),
            Self::Cyp => Kernel::Equatorial(Equatorial::Cyp {
                mu: pv[1],
                lambda: pv[2],
            }),
            Self::Par => Kernel::Equatorial(Equatorial::Par),
            Self::Bon => Kernel::Equatorial(Equatorial::Bon(Bon::new(pv[1]))),
            Self::Pco => Kernel::Equatorial(Equatorial::Pco),
            Self::Tsc => Kernel::Cube(Cube::Tsc),
            Self::Csc => Kernel::Cube(Cube::Csc),
            Self::Qsc => Kernel::Cube(Cube::Qsc),
            Self::Hpx => Kernel::Healpix(Healpix::new(&pv)),
        };
        ProjectionParameters {
            projection: self,
            kernel,
        }
    }

    pub(super) const fn domain_error(self) -> FitsError {
        FitsError::WcsPixelOutOfDomain { projection: self }
    }

    pub(super) const fn world_domain_error(self) -> FitsError {
        FitsError::WcsWorldOutOfDomain { projection: self }
    }

    fn checked_asin(self, value: f64) -> Result<f64> {
        if !value.is_finite() || !(-1.0 - UNIT_TOLERANCE..=1.0 + UNIT_TOLERANCE).contains(&value) {
            return Err(self.domain_error());
        }
        Ok(value.clamp(-1.0, 1.0).asin())
    }

    /// The root of `value`, a difference of terms of size `scale`: a value that
    /// rounding alone pushed below zero reads as zero.
    fn checked_sqrt(self, value: f64, scale: f64) -> Result<f64> {
        if !value.is_finite() || value < -UNIT_TOLERANCE * scale.abs().max(1.0) {
            return Err(self.domain_error());
        }
        Ok(value.max(0.0).sqrt())
    }

    /// A deprojected point, refused unless `φ` is within `[−180°, 180°]` and `θ` within
    /// `[−90°, 90°]` — wcslib's `prjbchk`, on by default (`bounds & 4`). A projection
    /// images each native longitude once, so a point whose inverse lands past ±180° lies
    /// off the map: beyond `CAR`'s edge, outside the `AIT` and `MOL` ellipses, outside a
    /// conic's wedge.
    fn native_coordinate(self, phi: f64, theta: f64) -> Result<NativeCoordinate> {
        if !theta.is_finite()
            || !(-90.0 - DEGREE_TOLERANCE..=90.0 + DEGREE_TOLERANCE).contains(&theta)
        {
            return Err(self.domain_error());
        }
        Ok(NativeCoordinate::from_degrees(
            self.native_longitude(phi)?,
            theta.clamp(-90.0, 90.0),
        ))
    }

    /// The deprojected point at colatitude `zeta` (radians), refused unless it lies in
    /// `[0, π]`, as [`Self::native_coordinate`] bounds θ.
    fn native_colatitude(self, phi: f64, zeta: f64) -> Result<NativeCoordinate> {
        const TOLERANCE: f64 = DEGREE_TOLERANCE * D2R;
        if !zeta.is_finite() || !(-TOLERANCE..=PI + TOLERANCE).contains(&zeta) {
            return Err(self.domain_error());
        }
        Ok(NativeCoordinate::from_colatitude(
            self.native_longitude(phi)?,
            zeta.clamp(0.0, PI),
        ))
    }

    /// The deprojected point at depth `sigma` = 1 − sin θ in `[0, 2]`, as SIN and SZP
    /// solve for it: cos θ = √(σ(2 − σ)) has no cancellation next to the pole.
    fn native_depth(self, phi: f64, sigma: f64) -> Result<NativeCoordinate> {
        debug_assert!((0.0..=2.0).contains(&sigma), "a depth: {sigma}");
        Ok(NativeCoordinate::from_sin_cos(
            self.native_longitude(phi)?,
            1.0 - sigma,
            (sigma * (2.0 - sigma)).sqrt(),
        ))
    }

    fn native_longitude(self, phi: f64) -> Result<f64> {
        if !phi.is_finite()
            || !(-180.0 - DEGREE_TOLERANCE..=180.0 + DEGREE_TOLERANCE).contains(&phi)
        {
            return Err(self.domain_error());
        }
        Ok(phi.clamp(-180.0, 180.0))
    }

    const fn projected_coordinate(self, x: f64, y: f64) -> Result<ProjectedCoordinate> {
        if !x.is_finite() || !y.is_finite() {
            return Err(self.world_domain_error());
        }
        Ok(ProjectedCoordinate { x, y })
    }
}

/// A projection's kernel, the constants its `PVi_m` parameters give, validated and
/// built once rather than for each point.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ProjectionParameters {
    projection: Projection,
    kernel: Kernel,
}

#[derive(Debug, Clone, Copy, PartialEq)]
enum Kernel {
    Zenithal(Zenithal),
    Perspective(Perspective),
    Conic(Conic),
    Equatorial(Equatorial),
    Cube(Cube),
    Healpix(Healpix),
}

#[derive(Debug, Clone, Copy)]
pub(super) struct ProjectedCoordinate {
    pub(super) x: f64,
    pub(super) y: f64,
}

impl ProjectionParameters {
    pub(super) const fn projection(&self) -> Projection {
        self.projection
    }

    /// Project a native point to intermediate world `(x, y)` (deg).
    ///
    /// A point the projection has no image for is refused: behind a zenithal
    /// projection's horizon, past a perspective projection's limb or beyond its point of
    /// divergence, at a pole a projection sends to infinity, or past ZPN's inflection.
    /// The bounds are wcslib's (`*s2x` with `bounds & 1`), SZP's limb excepted.
    pub(super) fn project(&self, native: NativeCoordinate) -> Result<ProjectedCoordinate> {
        let projection = self.projection;
        if !native.is_finite() {
            return Err(projection.world_domain_error());
        }
        match self.kernel {
            Kernel::Zenithal(kernel) => kernel.project(projection, native),
            Kernel::Perspective(kernel) => kernel.project(projection, native),
            Kernel::Conic(kernel) => kernel.project(projection, native),
            Kernel::Equatorial(kernel) => kernel.project(projection, native),
            Kernel::Cube(kernel) => kernel.project(native.phi, native.theta()),
            Kernel::Healpix(kernel) => kernel.project(projection, native),
        }
    }

    /// Deproject intermediate world `(x, y)` (deg) to a native point.
    pub(super) fn deproject(&self, x: f64, y: f64) -> Result<NativeCoordinate> {
        let projection = self.projection;
        match self.kernel {
            Kernel::Zenithal(kernel) => kernel.deproject(projection, x, y),
            Kernel::Perspective(kernel) => kernel.deproject(projection, x, y),
            Kernel::Conic(kernel) => kernel.deproject(projection, x, y),
            Kernel::Equatorial(kernel) => kernel.deproject(projection, x, y),
            Kernel::Cube(kernel) => kernel.deproject(x, y),
            Kernel::Healpix(kernel) => kernel.deproject(projection, x, y),
        }
    }
}

const fn no_convergence(projection: Projection) -> FitsError {
    FitsError::WcsNoConvergence {
        algorithm: projection.code(),
    }
}

#[derive(Debug, Clone, Copy)]
struct NewtonEvaluation {
    residual: f64,
    derivative: f64,
}

/// The interval that holds a root and where its search starts: the residual is negative
/// toward `negative` and positive toward `positive`, whichever end is the lower, and the
/// search starts at `start`, clamped into the interval.
#[derive(Debug, Clone, Copy)]
struct Bracket {
    negative: f64,
    positive: f64,
    start: f64,
}

/// The root of `evaluate` in `bracket`, from its start: Newton's method on the exact
/// derivative, kept in the bracket that each residual narrows. A step that leaves the
/// bracket, or is not at most half the step before it, bisects the bracket instead, as
/// Boost's `newton_raphson_iterate` does. The iteration stops when the root moves by at
/// most four ulps or the bracket is that narrow, and errors with
/// [`FitsError::WcsNoConvergence`] when it does not stop in 100 steps, which bisection
/// alone would need only past 1000.
fn solve_newton(
    projection: Projection,
    bracket: Bracket,
    evaluate: impl Fn(f64) -> NewtonEvaluation,
) -> Result<f64> {
    let Bracket {
        mut negative,
        mut positive,
        start,
    } = bracket;
    let close = |a: f64, b: f64| (a - b).abs() <= 4.0 * f64::EPSILON * a.abs().max(b.abs());
    let mut value = start.clamp(negative.min(positive), negative.max(positive));
    let mut previous_step = f64::INFINITY;
    for _ in 0..100 {
        let evaluation = evaluate(value);
        if evaluation.residual == 0.0 {
            return Ok(value);
        }
        if evaluation.residual.is_nan() {
            return Err(no_convergence(projection));
        }
        if evaluation.residual < 0.0 {
            negative = value;
        } else {
            positive = value;
        }
        let newton = value - evaluation.residual / evaluation.derivative;
        let step = (newton - value).abs();
        let inside = newton > negative.min(positive) && newton < negative.max(positive);
        let next = if inside && step <= previous_step / 2.0 {
            previous_step = step;
            newton
        } else {
            let midpoint = f64::midpoint(negative, positive);
            previous_step = (midpoint - value).abs();
            midpoint
        };
        if close(next, value) || close(negative, positive) {
            return Ok(next);
        }
        value = next;
    }
    Err(no_convergence(projection))
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::world_coordinates::projection::zenithal::ZpnBranch;
    use crate::world_coordinates::projection::{Projection, ProjectionParameters, TABLE};

    impl Projection {
        /// Every projection.
        pub(crate) fn all() -> impl Iterator<Item = Self> {
            TABLE.iter().map(|entry| entry.projection)
        }

        /// The `PVi_0`, `PVi_1`, … the tests give the projection: each parameter it takes
        /// set off its default, where it has one.
        pub(crate) const fn test_parameters(self) -> &'static [f64] {
            match self {
                Self::Cea => &[0.0, 0.5],
                Self::Zpn => &[0.0, 1.0, 0.0, 0.1],
                Self::Cyp => &[0.0, 1.0, 0.5],
                Self::Cop | Self::Coe | Self::Cod | Self::Coo => &[0.0, 45.0, 15.0],
                Self::Bon | Self::Air => &[0.0, 45.0],
                Self::Azp => &[0.0, 2.0, 30.0],
                Self::Szp => &[0.0, 2.0, 180.0, 60.0],
                Self::Hpx => &[0.0, 3.0, 4.0],
                _ => &[],
            }
        }

        /// The kernel of `pv` taken as given, for kernel tests that bypass validation.
        pub(super) fn raw_parameters(self, pv: [f64; 21]) -> ProjectionParameters {
            self.kernel(pv, ZpnBranch::new(&pv).unwrap_or(ZpnBranch::UNUSED))
        }
    }
}

#[cfg(test)]
mod tests;
