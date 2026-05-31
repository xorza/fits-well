/// The typed value of a valued keyword record (§4.2).
///
/// The three string-ish "no real value" cases of §4.2.1 are kept distinct: a
/// quoted empty/null string is [`Value::Text`] (possibly empty), whereas a blank
/// value field with no quotes is [`Value::Undefined`].
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    /// `T` or `F`.
    Logical(bool),
    Integer(i64),
    Real(f64),
    /// A character string, already unescaped (`''` → `'`) with insignificant
    /// trailing spaces removed.
    Text(String),
    ComplexInteger {
        re: i64,
        im: i64,
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

    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Value::Integer(i) => Some(*i),
            _ => None,
        }
    }

    /// The value as `f64`, widening an [`Value::Integer`] to a real.
    pub fn as_real(&self) -> Option<f64> {
        match self {
            Value::Real(r) => Some(*r),
            Value::Integer(i) => Some(*i as f64),
            _ => None,
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

impl From<i64> for Value {
    fn from(i: i64) -> Self {
        Value::Integer(i)
    }
}

impl From<i32> for Value {
    fn from(i: i32) -> Self {
        Value::Integer(i as i64)
    }
}

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
    use super::*;

    #[test]
    fn accessors_only_match_their_own_variant() {
        assert_eq!(Value::Logical(true).as_logical(), Some(true));
        assert_eq!(Value::Logical(true).as_integer(), None);

        assert_eq!(Value::Integer(42).as_integer(), Some(42));
        assert_eq!(Value::Integer(42).as_logical(), None);

        assert_eq!(Value::Text("x".into()).as_text(), Some("x"));
        assert_eq!(Value::Text("x".into()).as_integer(), None);

        assert_eq!(Value::Undefined.as_text(), None);
        assert_eq!(Value::Undefined.as_real(), None);
    }

    #[test]
    fn as_real_widens_integers_but_not_other_types() {
        assert_eq!(Value::Real(1.5).as_real(), Some(1.5));
        assert_eq!(Value::Integer(3).as_real(), Some(3.0));
        assert_eq!(Value::Logical(true).as_real(), None);
    }

    #[test]
    fn from_conversions_pick_the_right_variant() {
        assert_eq!(Value::from(true), Value::Logical(true));
        assert_eq!(Value::from(16_i32), Value::Integer(16));
        assert_eq!(Value::from(16_i64), Value::Integer(16));
        assert_eq!(Value::from(1.5_f64), Value::Real(1.5));
        assert_eq!(Value::from("hi"), Value::Text("hi".into()));
        assert_eq!(Value::from(String::from("hi")), Value::Text("hi".into()));
    }
}
