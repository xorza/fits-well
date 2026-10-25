//! [`AsciiTform`]: the `TFORMn` of an ASCII-table column (§7.2.5).

use std::fmt::{self, Write as _};
use std::str::FromStr;

use crate::fits_error::{FitsError, Result};

/// The `TFORMn` of an ASCII-table column (§7.2.5): its type and field width, and for a
/// float column the form and the digits after the decimal point. The reader holds it
/// for each column and the writer writes it, so the `F`, `E` or `D` letter a table was
/// written with reads back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiTform {
    /// `Aw` — character string.
    Text { width: usize },
    /// `Iw` — decimal integer.
    Integer { width: usize },
    /// `Fw.d` / `Ew.d` / `Dw.d` — floating point.
    Float(AsciiFloatFormat),
}

impl AsciiTform {
    /// The field width in characters.
    pub const fn width(self) -> usize {
        match self {
            Self::Text { width } | Self::Integer { width } => width,
            Self::Float(format) => format.width(),
        }
    }
}

impl FromStr for AsciiTform {
    type Err = FitsError;

    /// The form `Aw`, `Iw`, `Fw.d`, `Ew.d` or `Dw.d` spells, blanks around it trimmed; a
    /// float form's `.d` is 0 when absent, and a string or integer form's is not read.
    /// Errors with [`FitsError::InvalidTform`] for any other text.
    fn from_str(value: &str) -> Result<Self> {
        let s = value.trim();
        let invalid = || FitsError::InvalidTform {
            tform: value.to_string(),
        };
        let letter = s.bytes().next().ok_or_else(invalid)?;
        let rest = &s[1..];
        let (width, decimals) = match rest.split_once('.') {
            Some((w, d)) => (
                w.trim().parse().map_err(|_| invalid())?,
                d.trim().parse().map_err(|_| invalid())?,
            ),
            None => (rest.trim().parse().map_err(|_| invalid())?, 0),
        };
        Ok(match letter {
            b'A' => Self::Text { width },
            b'I' => Self::Integer { width },
            b'F' => Self::Float(AsciiFloatFormat::F { width, decimals }),
            b'E' => Self::Float(AsciiFloatFormat::E { width, decimals }),
            b'D' => Self::Float(AsciiFloatFormat::D { width, decimals }),
            _ => return Err(invalid()),
        })
    }
}

impl fmt::Display for AsciiTform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Text { width } => write!(f, "A{width}"),
            Self::Integer { width } => write!(f, "I{width}"),
            Self::Float(format) => {
                write!(
                    f,
                    "{}{}.{}",
                    format.letter(),
                    format.width(),
                    format.decimals()
                )
            }
        }
    }
}

/// The `TFORMn` of an ASCII float column (§7.2.5): the field width and the digits after
/// the decimal point, which every cell holds. A value is rounded to `decimals` digits,
/// and one too wide for `width` is refused. The cells are written as cfitsio writes
/// them through C's `%w.df` and `%w.dE`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AsciiFloatFormat {
    /// `Fw.d`: fixed point, as `-12.50`.
    F { width: usize, decimals: usize },
    /// `Ew.d`: one digit before the point and a signed exponent of two digits or more,
    /// as `-1.25E+01`.
    E { width: usize, decimals: usize },
    /// `Dw.d`: as `Ew.d`, with the exponent letter `D`.
    D { width: usize, decimals: usize },
}

impl AsciiFloatFormat {
    pub const fn width(self) -> usize {
        match self {
            Self::F { width, .. } | Self::E { width, .. } | Self::D { width, .. } => width,
        }
    }

    pub const fn decimals(self) -> usize {
        match self {
            Self::F { decimals, .. } | Self::E { decimals, .. } | Self::D { decimals, .. } => {
                decimals
            }
        }
    }

    pub(crate) const fn letter(self) -> char {
        match self {
            Self::F { .. } => 'F',
            Self::E { .. } => 'E',
            Self::D { .. } => 'D',
        }
    }

    /// The width of the narrowest positive cell: `0.` and the decimals, and for an
    /// exponent form `E+00` too; `None` past `usize::MAX`.
    pub(crate) const fn minimum_width(self) -> Option<usize> {
        let point = match self {
            Self::F { .. } => 2,
            Self::E { .. } | Self::D { .. } => 6,
        };
        self.decimals().checked_add(point)
    }

    /// Append `value`, which is finite, to `out`.
    pub(crate) fn write(self, value: f64, out: &mut String) {
        debug_assert!(value.is_finite(), "a float cell is finite");
        let start = out.len();
        let decimals = self.decimals();
        match self {
            Self::F { .. } => {
                write!(out, "{value:.decimals$}").expect("a `String` takes every write");
                if decimals == 0 {
                    out.push('.');
                }
            }
            Self::E { .. } | Self::D { .. } => {
                write!(out, "{value:.decimals$E}").expect("a `String` takes every write");
                let at = start
                    + out[start..]
                        .find('E')
                        .expect("the exponent form writes an `E`");
                let exponent: i32 = out[at + 1..]
                    .parse()
                    .expect("the exponent form writes an integer exponent");
                out.truncate(at);
                if decimals == 0 {
                    out.push('.');
                }
                let sign = if exponent < 0 { '-' } else { '+' };
                write!(out, "{}{sign}{:02}", self.letter(), exponent.unsigned_abs())
                    .expect("a `String` takes every write");
            }
        }
    }
}
