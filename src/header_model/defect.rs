//! [`Defect`]: why a header record is kept as malformed.

use std::fmt;

/// Why a header record did not parse. The record keeps its 80 bytes; a lookup of its
/// keyword and a write of its header fail with `FitsError::InvalidValue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// The name is no keyword: a non-ASCII byte, or a character a keyword does not take.
    Name,
    /// The value does not parse, or holds a non-ASCII byte.
    Value,
}

impl fmt::Display for Defect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Name => "its name is no keyword",
            Self::Value => "its value does not parse",
        })
    }
}
