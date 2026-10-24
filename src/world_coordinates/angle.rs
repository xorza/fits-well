//! Angles in degrees: the conversions, the trigonometry exact at multiples of 90°, the
//! normalizations, and the rounding a bound in degrees allows.

use std::f64::consts::PI;

pub(super) const R2D: f64 = 180.0 / PI;
pub(super) const D2R: f64 = PI / 180.0;
/// How far past a bound a value in degrees may land through rounding and still
/// read as the bound — a latitude past ±90°, a projection-plane coordinate past its
/// edge. 1e-12° is about 70 ulp of 90 in degrees: more than the few
/// chained trigonometric operations behind such a value lose, and within the band
/// of 1e-13° to 1e-11° wcslib's `prjbchk` allows for the same test.
pub(super) const DEGREE_TOLERANCE: f64 = 1e-12;

/// The same allowance for a dimensionless value of order one — a sine or cosine
/// past ±1, a cube-face coordinate past its edge, the perspective σ past [0, 2] —
/// and, scaled by the operands, for a difference that should not be negative. About
/// 4500 ulp of 1, the order of wcslib's 1e-13 `tol` for its zenithal and cube tests.
pub(super) const UNIT_TOLERANCE: f64 = 1e-12;

/// The quarter turn a multiple of 90° lands on (0 to 3), or `None` for any other angle.
#[expect(
    clippy::cast_sign_loss,
    reason = "a multiple of 90° divided by 90 and reduced mod 4 is a whole number in 0..4"
)]
fn quarter_turn(degrees: f64) -> Option<u8> {
    (degrees.rem_euclid(90.0) == 0.0).then(|| (degrees / 90.0).rem_euclid(4.0) as u8)
}

/// `sin` of an angle in degrees, exact at multiples of 90°, where the conversion to
/// radians would leave a residue of order `1e-17` — wcslib's `sind`, which its boundary
/// tests rely on.
pub(super) fn sind(degrees: f64) -> f64 {
    match quarter_turn(degrees) {
        Some(0 | 2) => 0.0,
        Some(1) => 1.0,
        Some(_) => -1.0,
        None => (degrees * D2R).sin(),
    }
}

/// `cos` of an angle in degrees, exact at multiples of 90° — wcslib's `cosd`.
pub(super) fn cosd(degrees: f64) -> f64 {
    match quarter_turn(degrees) {
        Some(0) => 1.0,
        Some(1 | 3) => 0.0,
        Some(_) => -1.0,
        None => (degrees * D2R).cos(),
    }
}

/// Normalize an angle to `[0, 360)` degrees.
pub(super) fn norm360(a: f64) -> f64 {
    // A tiny negative angle's remainder, 360 − |a|, rounds to 360 itself.
    let normalized = a.rem_euclid(360.0);
    if normalized == 360.0 { 0.0 } else { normalized }
}

/// Normalize an angle to `[−180, 180)` degrees, exactly: the truncated remainder is
/// exact, and the one turn added or taken back is exact by Sterbenz's lemma.
pub(super) fn norm180(a: f64) -> f64 {
    let r = a % 360.0;
    if r >= 180.0 {
        r - 360.0
    } else if r < -180.0 {
        r + 360.0
    } else {
        r
    }
}
