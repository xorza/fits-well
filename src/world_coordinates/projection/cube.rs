//! Spherical cube projections from FITS WCS Paper II, §§5.6–5.8.

use std::f64::consts::FRAC_1_SQRT_2;
use std::f64::consts::PI;

use crate::error::Result;
use crate::world_coordinates::D2R;
use crate::world_coordinates::R2D;
use crate::world_coordinates::UNIT_TOLERANCE;
use crate::world_coordinates::projection::Projection;
use crate::world_coordinates::projection::{NativeCoordinate, ProjectedCoordinate};

const FACE_SCALE: f64 = 45.0;

/// The spherical cube projections.
#[derive(Debug, Clone, Copy)]
pub(super) enum Cube {
    Tsc,
    Csc,
    Qsc,
}

impl Cube {
    const fn projection(self) -> Projection {
        match self {
            Cube::Tsc => Projection::Tsc,
            Cube::Csc => Projection::Csc,
            Cube::Qsc => Projection::Qsc,
        }
    }
}

#[derive(Debug, Clone, Copy)]
struct Direction {
    l: f64,
    m: f64,
    n: f64,
}

#[derive(Debug, Clone, Copy)]
enum CubeFace {
    North,
    Front,
    Right,
    Back,
    Left,
    South,
}

#[derive(Debug, Clone, Copy)]
struct FaceDirection {
    face: CubeFace,
    zeta: f64,
    xi: f64,
    eta: f64,
    x0: f64,
    y0: f64,
    direction: Direction,
}

#[derive(Debug, Clone, Copy)]
struct FaceCoordinate {
    face: CubeFace,
    x: f64,
    y: f64,
}

#[derive(Debug, Clone, Copy)]
struct FaceRatios {
    chi: f32,
    psi: f32,
}

pub(super) fn deproject(cube: Cube, x: f64, y: f64) -> Result<NativeCoordinate> {
    let projection = cube.projection();
    let face = face_coordinate(projection, x, y)?;
    let direction = match cube {
        Cube::Tsc => direction_from_ratios(face.face, face.x, face.y),
        Cube::Csc => {
            let ratios = csc_inverse(face.x, face.y);
            direction_from_csc_ratios(face.face, ratios)
        }
        Cube::Qsc => qsc_inverse(projection, face)?,
    };
    native_coordinate(projection, direction)
}

pub(super) fn project(cube: Cube, phi: f64, theta: f64) -> Result<ProjectedCoordinate> {
    let projection = cube.projection();
    if matches!(cube, Cube::Qsc) && theta.abs() == 90.0 {
        return projection.projected_coordinate(0.0, theta.signum() * 90.0);
    }

    let face = face_direction(phi, theta);
    let coordinate = match cube {
        Cube::Tsc => FaceCoordinate {
            face: face.face,
            x: face.xi / face.zeta,
            y: face.eta / face.zeta,
        },
        Cube::Csc => {
            let chi = (face.xi / face.zeta) as f32;
            let psi = (face.eta / face.zeta) as f32;
            FaceCoordinate {
                face: face.face,
                x: f64::from(csc_forward_axis(chi, psi)),
                y: f64::from(csc_forward_axis(psi, chi)),
            }
        }
        Cube::Qsc => qsc_forward(face, theta),
    };
    // CSC evaluates its polynomial in f32, as wcslib does, so its face coordinates
    // carry f32 rounding: 2⁻²³ ≈ 1.2e-7 of a face.
    let tolerance = if matches!(cube, Cube::Csc) {
        1e-7
    } else {
        UNIT_TOLERANCE
    };
    projected_coordinate(cube, coordinate, face.x0, face.y0, tolerance)
}

fn face_coordinate(projection: Projection, x: f64, y: f64) -> Result<FaceCoordinate> {
    if !x.is_finite() || !y.is_finite() {
        return Err(projection.domain_error());
    }

    let mut xf = x / FACE_SCALE;
    let mut yf = y / FACE_SCALE;
    let in_cross = if xf.abs() <= 1.0 + UNIT_TOLERANCE {
        yf.abs() <= 3.0 + UNIT_TOLERANCE
    } else {
        xf.abs() <= 7.0 + UNIT_TOLERANCE && yf.abs() <= 1.0 + UNIT_TOLERANCE
    };
    if !in_cross {
        return Err(projection.domain_error());
    }

    if xf < -1.0 {
        xf += 8.0;
    }
    let face = if xf > 5.0 {
        xf -= 6.0;
        CubeFace::Left
    } else if xf > 3.0 {
        xf -= 4.0;
        CubeFace::Back
    } else if xf > 1.0 {
        xf -= 2.0;
        CubeFace::Right
    } else if yf > 1.0 {
        yf -= 2.0;
        CubeFace::North
    } else if yf < -1.0 {
        yf += 2.0;
        CubeFace::South
    } else {
        CubeFace::Front
    };
    if xf.abs() > 1.0 + UNIT_TOLERANCE || yf.abs() > 1.0 + UNIT_TOLERANCE {
        return Err(projection.domain_error());
    }
    Ok(FaceCoordinate {
        face,
        x: xf.clamp(-1.0, 1.0),
        y: yf.clamp(-1.0, 1.0),
    })
}

fn face_direction(phi: f64, theta: f64) -> FaceDirection {
    let longitude = phi * D2R;
    let latitude = theta * D2R;
    let direction = Direction {
        l: latitude.cos() * longitude.cos(),
        m: latitude.cos() * longitude.sin(),
        n: latitude.sin(),
    };

    let mut face = CubeFace::North;
    let mut zeta = direction.n;
    if direction.l > zeta {
        face = CubeFace::Front;
        zeta = direction.l;
    }
    if direction.m > zeta {
        face = CubeFace::Right;
        zeta = direction.m;
    }
    if -direction.l > zeta {
        face = CubeFace::Back;
        zeta = -direction.l;
    }
    if -direction.m > zeta {
        face = CubeFace::Left;
        zeta = -direction.m;
    }
    if -direction.n > zeta {
        face = CubeFace::South;
        zeta = -direction.n;
    }

    let (xi, eta, x0, y0) = match face {
        CubeFace::Front => (direction.m, direction.n, 0.0, 0.0),
        CubeFace::Right => (-direction.l, direction.n, 2.0, 0.0),
        CubeFace::Back => (-direction.m, direction.n, 4.0, 0.0),
        CubeFace::Left => (direction.l, direction.n, 6.0, 0.0),
        CubeFace::South => (direction.m, direction.l, 0.0, -2.0),
        CubeFace::North => (direction.m, -direction.l, 0.0, 2.0),
    };
    FaceDirection {
        face,
        zeta,
        xi,
        eta,
        x0,
        y0,
        direction,
    }
}

fn direction_from_ratios(face: CubeFace, chi: f64, psi: f64) -> Direction {
    let t = 1.0 / (1.0 + chi * chi + psi * psi).sqrt();
    direction_from_normalized_ratios(face, chi, psi, t)
}

fn direction_from_csc_ratios(face: CubeFace, ratios: FaceRatios) -> Direction {
    let t = 1.0 / (1.0 + f64::from(ratios.chi * ratios.chi + ratios.psi * ratios.psi)).sqrt();
    direction_from_normalized_ratios(face, f64::from(ratios.chi), f64::from(ratios.psi), t)
}

fn direction_from_normalized_ratios(face: CubeFace, chi: f64, psi: f64, t: f64) -> Direction {
    match face {
        CubeFace::Front => Direction {
            l: t,
            m: chi * t,
            n: psi * t,
        },
        CubeFace::Right => Direction {
            l: -chi * t,
            m: t,
            n: psi * t,
        },
        CubeFace::Back => Direction {
            l: -t,
            m: -chi * t,
            n: psi * t,
        },
        CubeFace::Left => Direction {
            l: chi * t,
            m: -t,
            n: psi * t,
        },
        CubeFace::South => Direction {
            l: psi * t,
            m: chi * t,
            n: -t,
        },
        CubeFace::North => Direction {
            l: -psi * t,
            m: chi * t,
            n: t,
        },
    }
}

fn native_coordinate(projection: Projection, direction: Direction) -> Result<NativeCoordinate> {
    let phi = if direction.l == 0.0 && direction.m == 0.0 {
        0.0
    } else {
        direction.m.atan2(direction.l) * R2D
    };
    let theta = projection.checked_asin(direction.n)? * R2D;
    projection.native_coordinate(phi, theta)
}

fn projected_coordinate(
    cube: Cube,
    coordinate: FaceCoordinate,
    x0: f64,
    y0: f64,
    tolerance: f64,
) -> Result<ProjectedCoordinate> {
    let projection = cube.projection();
    if coordinate.x.abs() > 1.0 + tolerance || coordinate.y.abs() > 1.0 + tolerance {
        return Err(projection.world_domain_error());
    }
    if matches!(cube, Cube::Csc) {
        let x = coordinate.x.clamp(-1.0, 1.0) as f32 + x0 as f32;
        let y = coordinate.y.clamp(-1.0, 1.0) as f32 + y0 as f32;
        return projection
            .projected_coordinate(FACE_SCALE * f64::from(x), FACE_SCALE * f64::from(y));
    }
    projection.projected_coordinate(
        FACE_SCALE * (coordinate.x.clamp(-1.0, 1.0) + x0),
        FACE_SCALE * (coordinate.y.clamp(-1.0, 1.0) + y0),
    )
}

fn csc_inverse(x: f64, y: f64) -> FaceRatios {
    let xf = x as f32;
    let yf = y as f32;
    FaceRatios {
        chi: xf + xf * (1.0 - xf * xf) * csc_inverse_polynomial(xf * xf, yf * yf),
        psi: yf + yf * (1.0 - yf * yf) * csc_inverse_polynomial(yf * yf, xf * xf),
    }
}

fn csc_inverse_polynomial(primary: f32, secondary: f32) -> f32 {
    const P00: f32 = -0.272_926_96;
    const P10: f32 = -0.076_299_69;
    const P20: f32 = -0.227_970_56;
    const P30: f32 = 0.548_523_84;
    const P40: f32 = -0.629_300_65;
    const P50: f32 = 0.257_957_94;
    const P60: f32 = 0.025_843_75;
    const P01: f32 = -0.028_194_52;
    const P11: f32 = -0.014_715_65;
    const P21: f32 = 0.480_515_1;
    const P31: f32 = -1.741_144_5;
    const P41: f32 = 1.715_475_1;
    const P51: f32 = -0.530_223_37;
    const P02: f32 = 0.270_581_6;
    const P12: f32 = -0.568_009_4;
    const P22: f32 = 0.308_033_17;
    const P32: f32 = 0.989_381;
    const P42: f32 = -0.831_804_7;
    const P03: f32 = -0.604_415_6;
    const P13: f32 = 1.508_800_9;
    const P23: f32 = -0.936_785_76;
    const P33: f32 = 0.086_938_41;
    const P04: f32 = 0.934_120_8;
    const P14: f32 = -1.416_019_2;
    const P24: f32 = 0.338_874_46;
    const P05: f32 = -0.639_153_06;
    const P15: f32 = 0.520_322_4;
    const P06: f32 = 0.143_815_85;

    let z0 = P00
        + primary
            * (P10
                + primary
                    * (P20 + primary * (P30 + primary * (P40 + primary * (P50 + primary * P60)))));
    let z1 =
        P01 + primary * (P11 + primary * (P21 + primary * (P31 + primary * (P41 + primary * P51))));
    let z2 = P02 + primary * (P12 + primary * (P22 + primary * (P32 + primary * P42)));
    let z3 = P03 + primary * (P13 + primary * (P23 + primary * P33));
    let z4 = P04 + primary * (P14 + primary * P24);
    let z5 = P05 + primary * P15;
    z0 + secondary
        * (z1
            + secondary
                * (z2 + secondary * (z3 + secondary * (z4 + secondary * (z5 + secondary * P06)))))
}

fn csc_forward_axis(primary: f32, secondary: f32) -> f32 {
    const GSTAR: f32 = 1.374_848_5;
    const M: f32 = 0.004_869_492;
    const GAMMA: f32 = -0.131_616_71;
    const OMEGA1: f32 = -0.159_596_23;
    const D0: f32 = 0.075_919_62;
    const D1: f32 = -0.021_776_25;
    const C00: f32 = 0.141_189_63;
    const C10: f32 = 0.080_970_13;
    const C01: f32 = -0.281_528_53;
    const C11: f32 = 0.153_841_12;
    const C20: f32 = -0.178_251_2;
    const C02: f32 = 0.106_959_47;

    let p2 = primary * primary;
    let s2 = secondary * secondary;
    let pco = 1.0 - p2;
    let sco = 1.0 - s2;
    let product = (primary * secondary).abs();
    let p4 = if p2 > 1e-16 { p2 * p2 } else { 0.0 };
    let s4 = if s2 > 1e-16 { s2 * s2 } else { 0.0 };
    let p2s2 = if product > 1e-16 { p2 * s2 } else { 0.0 };
    primary
        * (p2
            + pco
                * (GSTAR
                    + s2 * (GAMMA * pco
                        + M * p2
                        + sco * (C00 + C10 * p2 + C01 * s2 + C11 * p2s2 + C20 * p4 + C02 * s4))
                    + p2 * (OMEGA1 - pco * (D0 + D1 * p2))))
}

fn qsc_inverse(projection: Projection, face: FaceCoordinate) -> Result<Direction> {
    let direct = face.x.abs() > face.y.abs();
    let primary = if direct { face.x } else { face.y };
    let secondary = if direct { face.y } else { face.x };
    let (omega, tau, zeco) = if primary == 0.0 {
        (0.0, 1.0, 0.0)
    } else {
        let angle = 15.0 * secondary / primary * D2R;
        let omega = angle.sin() / (angle.cos() - FRAC_1_SQRT_2);
        let tau = 1.0 + omega * omega;
        let zeco = primary * primary * (1.0 - 1.0 / (1.0 + tau).sqrt());
        (omega, tau, zeco)
    };
    let mut zeta = 1.0 - zeco;
    let w = if zeta < -1.0 {
        if zeta < -1.0 - UNIT_TOLERANCE {
            return Err(projection.domain_error());
        }
        zeta = -1.0;
        0.0
    } else {
        projection.checked_sqrt(zeco * (2.0 - zeco) / tau, 1.0)?
    };

    let signed_x = w.copysign(face.x);
    let signed_y = w.copysign(face.y);
    let direction = match (face.face, direct) {
        (CubeFace::Front, true) => Direction {
            l: zeta,
            m: signed_x,
            n: signed_x * omega,
        },
        (CubeFace::Front, false) => Direction {
            l: zeta,
            m: signed_y * omega,
            n: signed_y,
        },
        (CubeFace::Right, true) => Direction {
            l: -signed_x,
            m: zeta,
            n: signed_x * omega,
        },
        (CubeFace::Right, false) => Direction {
            l: -signed_y * omega,
            m: zeta,
            n: signed_y,
        },
        (CubeFace::Back, true) => Direction {
            l: -zeta,
            m: -signed_x,
            n: signed_x * omega,
        },
        (CubeFace::Back, false) => Direction {
            l: -zeta,
            m: -signed_y * omega,
            n: signed_y,
        },
        (CubeFace::Left, true) => Direction {
            l: signed_x,
            m: -zeta,
            n: signed_x * omega,
        },
        (CubeFace::Left, false) => Direction {
            l: signed_y * omega,
            m: -zeta,
            n: signed_y,
        },
        (CubeFace::South, true) => Direction {
            l: signed_x * omega,
            m: signed_x,
            n: -zeta,
        },
        (CubeFace::South, false) => Direction {
            l: signed_y,
            m: signed_y * omega,
            n: -zeta,
        },
        (CubeFace::North, true) => Direction {
            l: -signed_x * omega,
            m: signed_x,
            n: zeta,
        },
        (CubeFace::North, false) => Direction {
            l: -signed_y,
            m: signed_y * omega,
            n: zeta,
        },
    };
    Ok(direction)
}

fn qsc_forward(face: FaceDirection, theta: f64) -> FaceCoordinate {
    let mut zeco = 1.0 - face.zeta;
    // wcslib's `qscs2x` cutoff: below it 1 − ζ has lost most of its digits, so the
    // small-angle form takes over.
    if zeco < 1e-8 {
        zeco = qsc_small_angle_zeco(face, theta);
    }

    let mut x = 0.0;
    let mut y = 0.0;
    if face.xi != 0.0 || face.eta != 0.0 {
        if -face.xi > face.eta.abs() {
            x = -qsc_primary(face.eta / face.xi, zeco);
            y = qsc_secondary(x, face.eta / face.xi);
        } else if face.xi > face.eta.abs() {
            x = qsc_primary(face.eta / face.xi, zeco);
            y = qsc_secondary(x, face.eta / face.xi);
        } else if -face.eta >= face.xi.abs() {
            y = -qsc_primary(face.xi / face.eta, zeco);
            x = qsc_secondary(y, face.xi / face.eta);
        } else {
            y = qsc_primary(face.xi / face.eta, zeco);
            x = qsc_secondary(y, face.xi / face.eta);
        }
    }
    FaceCoordinate {
        face: face.face,
        x,
        y,
    }
}

fn qsc_small_angle_zeco(face: FaceDirection, theta: f64) -> f64 {
    let longitude = face.direction.m.atan2(face.direction.l);
    match face.face {
        CubeFace::Front => f64::midpoint(longitude * longitude, (theta * D2R).powi(2)),
        CubeFace::Right => f64::midpoint((longitude - PI / 2.0).powi(2), (theta * D2R).powi(2)),
        CubeFace::Back => {
            let offset = longitude - PI.copysign(longitude);
            f64::midpoint(offset * offset, (theta * D2R).powi(2))
        }
        CubeFace::Left => f64::midpoint((longitude + PI / 2.0).powi(2), (theta * D2R).powi(2)),
        CubeFace::South => ((theta + 90.0) * D2R).powi(2) / 2.0,
        CubeFace::North => ((90.0 - theta) * D2R).powi(2) / 2.0,
    }
}

fn qsc_primary(omega: f64, zeco: f64) -> f64 {
    let tau = 1.0 + omega * omega;
    (zeco / (1.0 - 1.0 / (1.0 + tau).sqrt())).sqrt()
}

fn qsc_secondary(primary: f64, omega: f64) -> f64 {
    let tau = 1.0 + omega * omega;
    primary / 15.0 * (omega.atan() * R2D - (omega / (2.0 * tau).sqrt()).asin() * R2D)
}
