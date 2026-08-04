use std::fmt;

use crate::error::FitsError;
use crate::error::Result;

/// A signed decimal FITS integer (§4.2.3).
///
/// FITS does not limit integer-keyword range. Values within `i64` use an
/// allocation-free representation; larger parsed values retain their normalized
/// decimal digits exactly.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FitsInteger {
    repr: IntegerRepr,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum IntegerRepr {
    I64(i64),
    Decimal(Box<str>),
}

impl FitsInteger {
    pub(super) fn parse(token: &str) -> Option<FitsInteger> {
        if let Ok(value) = token.parse::<i64>() {
            return Some(FitsInteger::from(value));
        }
        let (negative, digits) = match token.as_bytes().first() {
            Some(b'+') => (false, &token[1..]),
            Some(b'-') => (true, &token[1..]),
            _ => (false, token),
        };
        if digits.is_empty() || !digits.bytes().all(|byte| byte.is_ascii_digit()) {
            return None;
        }
        let significant = digits.trim_start_matches('0');
        if significant.is_empty() {
            return Some(FitsInteger::from(0_i64));
        }
        let decimal = if negative {
            format!("-{significant}")
        } else {
            significant.to_string()
        };
        Some(FitsInteger {
            repr: IntegerRepr::Decimal(decimal.into_boxed_str()),
        })
    }

    fn to_f64(&self) -> Result<f64> {
        let value = match &self.repr {
            IntegerRepr::I64(value) => *value as f64,
            IntegerRepr::Decimal(decimal) => decimal
                .parse::<f64>()
                .ok()
                .filter(|value| value.is_finite())
                .ok_or_else(|| self.out_of_range("f64"))?,
        };
        Ok(value)
    }

    fn out_of_range(&self, target: &'static str) -> FitsError {
        FitsError::IntegerOutOfRange {
            value: self.to_string(),
            target,
        }
    }
}

impl fmt::Display for FitsInteger {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.repr {
            IntegerRepr::I64(value) => value.fmt(f),
            IntegerRepr::Decimal(decimal) => decimal.fmt(f),
        }
    }
}

impl TryFrom<&FitsInteger> for i64 {
    type Error = FitsError;

    fn try_from(value: &FitsInteger) -> Result<i64> {
        match &value.repr {
            IntegerRepr::I64(value) => Ok(*value),
            IntegerRepr::Decimal(decimal) => decimal
                .parse::<i64>()
                .map_err(|_| value.out_of_range("i64")),
        }
    }
}

/// Widths that may not fit `i64` and so fall back to the decimal representation.
/// `i32`/`u32`/`i64` are written out separately below: their `i64::try_from` can
/// never fail, and spelling an infallible conversion fallibly is both misleading and
/// a clippy lint.
macro_rules! impl_integer_from {
    ($($type:ty),+ $(,)?) => {
        $(
            impl From<$type> for FitsInteger {
                fn from(value: $type) -> FitsInteger {
                    if let Ok(value) = i64::try_from(value) {
                        FitsInteger {
                            repr: IntegerRepr::I64(value),
                        }
                    } else {
                        FitsInteger {
                            repr: IntegerRepr::Decimal(value.to_string().into_boxed_str()),
                        }
                    }
                }
            }
        )+
    };
}

impl From<i64> for FitsInteger {
    fn from(value: i64) -> FitsInteger {
        FitsInteger {
            repr: IntegerRepr::I64(value),
        }
    }
}

impl From<i32> for FitsInteger {
    fn from(value: i32) -> FitsInteger {
        FitsInteger::from(i64::from(value))
    }
}

impl From<u32> for FitsInteger {
    fn from(value: u32) -> FitsInteger {
        FitsInteger::from(i64::from(value))
    }
}

impl_integer_from!(i128, u64, u128);

/// Narrow a `usize` count to the `i64` a FITS integer card holds. Counts derived
/// from geometry (row widths, heap lengths, axis lengths) are `usize` internally but
/// must fit a header value; an absurd one becomes [`FitsError::DataUnitOverflow`]
/// rather than wrapping into a plausible-looking card.
pub(crate) fn fits_i64(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}

/// The typed value of a valued keyword record (§4.2).
///
/// The three string-ish "no real value" cases of §4.2.1 are kept distinct: a
/// quoted empty/null string is [`Value::Text`] (possibly empty), whereas a blank
/// value field with no quotes is [`Value::Undefined`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `T` or `F`.
    Logical(bool),
    /// A signed decimal integer with the unbounded range allowed by FITS.
    Integer(FitsInteger),
    Real(f64),
    /// A character string, already unescaped (`''` → `'`) with insignificant
    /// trailing spaces removed.
    Text(String),
    ComplexInteger {
        re: FitsInteger,
        im: FitsInteger,
    },
    ComplexReal {
        re: f64,
        im: f64,
    },
    /// A value indicator was present but the field was blank.
    Undefined,
}

impl Value {
    pub fn as_logical(&self) -> Option<bool> {
        match self {
            Value::Logical(b) => Some(*b),
            _ => None,
        }
    }

    /// Convert an integer-valued keyword to `i64`.
    ///
    /// Returns `Ok(None)` for a different value type or a non-integral real.
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::IntegerOutOfRange`] when the exact integer lies outside
    /// `i64`, including an integral [`Value::Real`] outside that range.
    pub fn as_integer(&self) -> Result<Option<i64>> {
        match self {
            Value::Integer(i) => Ok(Some(i64::try_from(i)?)),
            // A mandatory-integer keyword written in real form (e.g. `NAXIS = 2.0`):
            // accept an integral value rather than reporting it absent, which would
            // silently mis-size the data unit. Reject integral reals outside i64
            // instead of accepting Rust's saturating float-to-integer cast.
            Value::Real(r) if r.fract() == 0.0 => {
                const I64_MIN_F64: f64 = -9_223_372_036_854_775_808.0;
                const I64_MAX_EXCLUSIVE_F64: f64 = 9_223_372_036_854_775_808.0;
                if *r >= I64_MIN_F64 && *r < I64_MAX_EXCLUSIVE_F64 {
                    Ok(Some(*r as i64))
                } else {
                    Err(FitsError::IntegerOutOfRange {
                        value: r.to_string(),
                        target: "i64",
                    })
                }
            }
            _ => Ok(None),
        }
    }

    /// The value as `f64`, converting an exact [`Value::Integer`] to a real.
    ///
    /// # Errors
    ///
    /// Returns [`FitsError::IntegerOutOfRange`] if the exact integer is too large
    /// for a finite `f64`.
    pub fn as_real(&self) -> Result<Option<f64>> {
        match self {
            Value::Real(r) => Ok(Some(*r)),
            Value::Integer(i) => Ok(Some(i.to_f64()?)),
            _ => Ok(None),
        }
    }

    pub fn as_text(&self) -> Option<&str> {
        match self {
            Value::Text(s) => Some(s),
            _ => None,
        }
    }
}

impl From<bool> for Value {
    fn from(b: bool) -> Self {
        Value::Logical(b)
    }
}

impl From<FitsInteger> for Value {
    fn from(integer: FitsInteger) -> Self {
        Value::Integer(integer)
    }
}

macro_rules! impl_value_from_integer {
    ($($type:ty),+ $(,)?) => {
        $(
            impl From<$type> for Value {
                fn from(value: $type) -> Value {
                    Value::Integer(value.into())
                }
            }
        )+
    };
}

// Every integer width `FitsInteger` accepts, wrapped identically — unlike the
// `FitsInteger` conversions above, none of these needs a fallible narrowing.
impl_value_from_integer!(i32, i64, i128, u32, u64, u128);

impl From<f64> for Value {
    fn from(r: f64) -> Self {
        Value::Real(r)
    }
}

impl From<&str> for Value {
    fn from(s: &str) -> Self {
        Value::Text(s.to_string())
    }
}

impl From<String> for Value {
    fn from(s: String) -> Self {
        Value::Text(s)
    }
}

#[cfg(test)]
mod tests {
    use crate::header::value::*;

    #[test]
    fn accessors_only_match_their_own_variant() {
        assert_eq!(Value::Logical(true).as_logical(), Some(true));
        assert_eq!(Value::Logical(true).as_integer().unwrap(), None);

        assert_eq!(Value::from(42_i64).as_integer().unwrap(), Some(42));
        assert_eq!(Value::from(42_i64).as_logical(), None);

        assert_eq!(Value::Text("x".into()).as_text(), Some("x"));
        assert_eq!(Value::Text("x".into()).as_integer().unwrap(), None);

        assert_eq!(Value::Undefined.as_text(), None);
        assert_eq!(Value::Undefined.as_real().unwrap(), None);
    }

    #[test]
    fn as_real_widens_integers_but_not_other_types() {
        assert_eq!(Value::Real(1.5).as_real().unwrap(), Some(1.5));
        assert_eq!(Value::from(3_i64).as_real().unwrap(), Some(3.0));
        assert_eq!(Value::Logical(true).as_real().unwrap(), None);
    }

    #[test]
    fn from_conversions_pick_the_right_variant() {
        assert_eq!(Value::from(true), Value::Logical(true));
        assert_eq!(Value::from(16_i32), Value::Integer(16_i64.into()));
        assert_eq!(Value::from(16_i64), Value::Integer(16_i64.into()));
        assert_eq!(
            Value::from(9_223_372_036_854_775_808_u64),
            Value::Integer(9_223_372_036_854_775_808_u64.into())
        );
        assert_eq!(Value::from(1.5_f64), Value::Real(1.5));
        assert_eq!(Value::from("hi"), Value::Text("hi".into()));
        assert_eq!(Value::from(String::from("hi")), Value::Text("hi".into()));
    }

    #[test]
    fn exact_integer_conversion_reports_i64_boundaries() {
        for (decimal, expected) in [
            ("-9223372036854775808", Some(i64::MIN)),
            ("9223372036854775807", Some(i64::MAX)),
            ("-9223372036854775809", None),
            ("9223372036854775808", None),
        ] {
            let value = FitsInteger::parse(decimal).unwrap();
            assert_eq!(value.to_string(), decimal);
            match expected {
                Some(expected) => assert_eq!(i64::try_from(&value).unwrap(), expected),
                None => assert!(matches!(
                    i64::try_from(&value),
                    Err(FitsError::IntegerOutOfRange { value, target })
                        if value == decimal && target == "i64"
                )),
            }
        }

        assert_eq!(
            Value::Real(-9_223_372_036_854_775_808.0)
                .as_integer()
                .unwrap(),
            Some(i64::MIN)
        );
        assert!(matches!(
            Value::Real(9_223_372_036_854_775_808.0).as_integer(),
            Err(FitsError::IntegerOutOfRange { target: "i64", .. })
        ));
    }
}
