//! [`Mjd`]: a Modified Julian Date as a whole day and a fraction.

/// A Modified Julian Date (`JD − 2400000.5`) as an integer day and a fraction of a day
/// in [0, 1), as `MJDREFI` and `MJDREFF` split it (§9.2.2). The fraction has a spacing
/// of at most 2⁻⁵³ day ≈ 1e-11 s at every date, where one `f64` MJD has 2⁻³⁷ day ≈
/// 0.63 µs at MJD 60000.
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd)]
pub struct Mjd {
    day: i64,
    fraction: f64,
}

/// A value as the unevaluated sum `hi + lo`, which the error-free operations give
/// (Shewchuk 1997, as astropy's `day_frac`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Expansion {
    hi: f64,
    lo: f64,
}

impl Mjd {
    /// The MJD `day + fraction`, the fraction moved into [0, 1): `Mjd::new(5, 1.0)` is
    /// day 6 and fraction 0, and `Mjd::new(5, -0.25)` day 4 and fraction 0.75. Panics
    /// when `fraction` is not finite, or the day leaves the range of `i64`.
    pub fn new(day: i64, fraction: f64) -> Self {
        assert!(fraction.is_finite(), "an MJD fraction is finite");
        Self::normalized(day, Expansion::from(fraction)).expect("an MJD day fits i64")
    }

    /// The whole day.
    pub const fn day(self) -> i64 {
        self.day
    }

    /// The fraction of the day, in [0, 1).
    pub const fn fraction(self) -> f64 {
        self.fraction
    }

    /// The MJD in one `f64`, rounded to its spacing.
    pub fn to_f64(self) -> f64 {
        self.day as f64 + self.fraction
    }

    /// The MJD `whole + fraction` of two header values, `whole` not always integral;
    /// `None` when [`Mjd::plus`] is.
    pub(crate) fn from_days(whole: f64, fraction: f64) -> Option<Self> {
        Self::new(0, 0.0).plus(Expansion::two_sum(whole, fraction))
    }

    /// This MJD `days` later, rounded once into the fraction. `None` when `days` is
    /// not finite or not below 2⁵³ in size, where an `f64` holds no fraction of a day,
    /// or the day leaves the range of `i64`.
    pub(crate) fn plus(self, days: Expansion) -> Option<Self> {
        let whole = days.hi.floor();
        let rest = Expansion::two_sum(days.hi, -whole);
        let sum = Expansion::two_sum(self.fraction, rest.hi);
        Self::normalized(
            self.day.checked_add(whole_days(whole)?)?,
            Expansion {
                hi: sum.hi,
                lo: sum.lo + (rest.lo + days.lo),
            },
        )
    }

    fn normalized(day: i64, fraction: Expansion) -> Option<Self> {
        let whole = fraction.hi.floor();
        let split = Expansion::two_sum(fraction.hi, -whole);
        let rest = split.hi + (split.lo + fraction.lo);
        let carry = rest.floor();
        let mut day = day.checked_add(whole_days(whole + carry)?)?;
        let mut fraction = rest - carry;
        // A rest of −2⁻⁵⁴ less carries 1 − 2⁻⁵⁴, which rounds to 1.
        if fraction == 1.0 {
            day = day.checked_add(1)?;
            fraction = 0.0;
        }
        debug_assert!((0.0..1.0).contains(&fraction));
        Some(Self { day, fraction })
    }
}

/// The integral `whole` as an `i64`, `None` when it is not below 2⁵³ in size or not
/// finite.
fn whole_days(whole: f64) -> Option<i64> {
    (whole.abs() < 2.0_f64.powi(53)).then_some(whole as i64)
}

impl Expansion {
    /// `a + b` without a rounding error (Knuth's two-sum).
    pub(crate) fn two_sum(a: f64, b: f64) -> Self {
        let hi = a + b;
        let b_part = hi - a;
        let lo = (a - (hi - b_part)) + (b - b_part);
        Self { hi, lo }
    }

    /// `a × b` without a rounding error, its error from a fused multiply-add.
    pub(crate) fn two_product(a: f64, b: f64) -> Self {
        let hi = a * b;
        Self {
            hi,
            lo: a.mul_add(b, -hi),
        }
    }

    /// This value times `factor`, rounded once to the expansion's precision.
    pub(crate) fn scaled(self, factor: f64) -> Self {
        let product = Self::two_product(self.hi, factor);
        Self {
            hi: product.hi,
            lo: self.lo.mul_add(factor, product.lo),
        }
    }

    /// This value plus `other`, rounded once to the expansion's precision.
    pub(crate) fn plus(self, other: Self) -> Self {
        let sum = Self::two_sum(self.hi, other.hi);
        Self {
            hi: sum.hi,
            lo: sum.lo + (self.lo + other.lo),
        }
    }

    /// This value over `divisor`: the quotient and the exact remainder `hi − q ×
    /// divisor` over `divisor`.
    pub(crate) fn divided(self, divisor: f64) -> Self {
        let hi = self.hi / divisor;
        let remainder = (-hi).mul_add(divisor, self.hi);
        Self {
            hi,
            lo: (remainder + self.lo) / divisor,
        }
    }
}

impl From<f64> for Expansion {
    fn from(value: f64) -> Self {
        Self { hi: value, lo: 0.0 }
    }
}

#[cfg(test)]
mod tests {
    use crate::internals::rng::SplitMix;
    use crate::time_coordinates::mjd::*;

    /// The day moves by whole days into the fraction's [0, 1).
    #[test]
    fn mjd_normalizes_its_fraction() {
        for (day, fraction, expected_day, expected_fraction) in [
            (5, 1.0, 6, 0.0),
            (5, -0.25, 4, 0.75),
            (5, 2.5, 7, 0.5),
            (-3, 0.0, -3, 0.0),
            (0, -f64::MIN_POSITIVE, 0, 0.0),
        ] {
            let mjd = Mjd::new(day, fraction);
            assert_eq!(
                (mjd.day(), mjd.fraction()),
                (expected_day, expected_fraction),
                "{day} + {fraction}"
            );
        }
        assert!(Mjd::new(4, 0.75) < Mjd::new(5, 0.0));
        assert_eq!(Mjd::new(60000, 0.25).to_f64(), 60000.25);
    }

    /// `value` in units of 2⁻¹⁰⁴: exact for a multiple of 2⁻¹⁰⁴ below 2²³ in size.
    fn fixed(value: f64) -> i128 {
        let scaled = value * 2f64.powi(104);
        debug_assert_eq!(scaled.fract(), 0.0, "{value:e} is not a multiple of 2⁻¹⁰⁴");
        scaled as i128
    }

    /// A draw of either sign whose size has its binary exponent in `exponents`, so it is
    /// a multiple of 2 to the exponent's start less 52.
    fn draw(draws: &mut SplitMix, exponents: std::ops::Range<i32>) -> f64 {
        let span = u64::try_from(exponents.end - exponents.start).unwrap();
        let exponent = exponents.start + i32::try_from(draws.next() % span).unwrap();
        let sign = if draws.next() & 1 == 0 { 1.0 } else { -1.0 };
        sign * (1.0 + draws.unit()) * 2f64.powi(exponent)
    }

    /// Two-sum and two-product are exact, checked in i128 at 2⁻¹⁰⁴: for addends of size
    /// 2⁻⁴⁰ to 2⁴, and for factors of size 1 to 2⁴, every input, `hi` and `lo` is a
    /// multiple of 2⁻¹⁰⁴, and the product of two is below 2⁸.
    #[test]
    fn two_sum_and_two_product_are_exact() {
        let mut draws = SplitMix::new(7);
        for _ in 0..10_000 {
            let [a, b] = [draw(&mut draws, 0..4), draw(&mut draws, -40..4)];
            let sum = Expansion::two_sum(a, b);
            assert_eq!(
                fixed(sum.hi) + fixed(sum.lo),
                fixed(a) + fixed(b),
                "{a:e} + {b:e}"
            );
            let b = draw(&mut draws, 0..4);
            let product = Expansion::two_product(a, b);
            let exact = (fixed(a) >> 52) * (fixed(b) >> 52);
            assert_eq!(
                fixed(product.hi) + fixed(product.lo),
                exact,
                "{a:e} × {b:e}"
            );
        }
    }

    /// `Mjd::plus` rounds the sum once into the fraction, which lies in [0, 1), so it is
    /// within half its ulp, 2⁻⁵⁴ day, of the exact sum in i128 at 2⁻¹⁰⁴ day.
    #[test]
    fn plus_rounds_once() {
        let mut draws = SplitMix::new(11);
        for _ in 0..10_000 {
            let start = Mjd::new(60_000, (draws.next() >> 12) as f64 * 2f64.powi(-52));
            let days = Expansion {
                hi: draw(&mut draws, -40..20),
                lo: draw(&mut draws, -52..-48),
            };
            let sum = start.plus(days).unwrap();
            let exact = (i128::from(start.day()) << 104)
                + fixed(start.fraction())
                + fixed(days.hi)
                + fixed(days.lo);
            let got =
                (i128::from(sum.day()) << 104) + (sum.fraction() * 2f64.powi(104)).round() as i128;
            assert!(
                (got - exact).abs() <= 1 << 50,
                "{start:?} + {days:?} = {sum:?}: off by {} × 2⁻¹⁰⁴",
                got - exact
            );
        }
    }
}
