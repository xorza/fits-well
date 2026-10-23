//! [`Defect`]: why a header record is kept as malformed.

/// Why a header record did not parse. The record keeps its 80 bytes; a lookup of its
/// keyword and a write of its header fail with `FitsError::InvalidValue`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Defect {
    /// The name is no keyword: a non-ASCII byte, or a character a keyword does not take.
    Name,
    /// The value does not parse, or holds a non-ASCII byte.
    Value,
}
