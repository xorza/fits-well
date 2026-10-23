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
