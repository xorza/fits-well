//! One binary-table `A` field, read in place.

/// One binary-table `A` field, its stored bytes preserved exactly.
///
/// [`CharacterField::members`] stops at the first NUL, while
/// [`CharacterField::bytes`] keeps the terminator, undefined bytes after it, and
/// all trailing spaces. A NUL in the first byte is therefore distinguishable from
/// an empty or all-space field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CharacterField<'a> {
    bytes: &'a [u8],
}

impl<'a> CharacterField<'a> {
    /// The field stored as `bytes` — a fixed column's `repeat` bytes for one row,
    /// or one row of a `P`/`Q` character column.
    pub const fn new(bytes: &'a [u8]) -> Self {
        CharacterField { bytes }
    }

    pub const fn bytes(&self) -> &'a [u8] {
        self.bytes
    }

    /// The defined character members, ending immediately before the first NUL.
    pub fn members(&self) -> &'a [u8] {
        let end = self
            .bytes
            .iter()
            .position(|&byte| byte == 0)
            .unwrap_or(self.bytes.len());
        &self.bytes[..end]
    }

    /// Whether this is the FITS null string, identified by an initial NUL.
    pub fn is_null(&self) -> bool {
        self.bytes.first() == Some(&0)
    }
}

#[cfg(test)]
mod tests;
