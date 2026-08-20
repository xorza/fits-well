//! One binary-table `A` field, stored bytes and all.

/// One binary-table `A` field with its stored bytes preserved exactly.
///
/// [`CharacterField::members`] stops at the first NUL, while [`CharacterField::bytes`]
/// retains the terminator, undefined bytes after it, and all trailing spaces. A NUL
/// in the first byte is therefore distinguishable from an empty or all-space field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CharacterField {
    pub bytes: Vec<u8>,
}

impl CharacterField {
    pub fn new(bytes: impl Into<Vec<u8>>) -> CharacterField {
        CharacterField {
            bytes: bytes.into(),
        }
    }

    /// The defined character members, ending immediately before the first NUL.
    pub fn members(&self) -> &[u8] {
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

    /// Construct the shortest stored representation of a FITS null string.
    pub fn null() -> CharacterField {
        CharacterField { bytes: vec![0] }
    }
}

impl From<&str> for CharacterField {
    fn from(value: &str) -> CharacterField {
        CharacterField::new(value.as_bytes().to_vec())
    }
}

impl From<String> for CharacterField {
    fn from(value: String) -> CharacterField {
        CharacterField::new(value.into_bytes())
    }
}

impl From<Vec<u8>> for CharacterField {
    fn from(value: Vec<u8>) -> CharacterField {
        CharacterField::new(value)
    }
}
