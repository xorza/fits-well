//! [`CommentaryName`]: the name of a header record with no value indicator.

use std::fmt;
use std::str;
use std::str::FromStr;

use crate::error::{FitsError, Result};

/// The name of a record with no value indicator: bytes 1–8 of the record with their
/// trailing blanks dropped. The standard lets such a record start with any 8 bytes of
/// printable ASCII (§4.1.2.1), which no [`Keyword`](crate::header::Keyword) holds, so
/// the name is its own type; the blank name is `""`. It is held inline, so it is `Copy`
/// and makes no allocation.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct CommentaryName {
    bytes: [u8; 8],
    len: u8,
}

impl CommentaryName {
    pub const COMMENT: Self = Self::new("COMMENT");
    pub const HISTORY: Self = Self::new("HISTORY");
    pub const BLANK: Self = Self::new("");
    pub const CONTINUE: Self = Self::new("CONTINUE");

    /// The name `name`.
    ///
    /// # Panics
    /// When `name` is no commentary name; in a `const`, the build fails instead.
    const fn new(name: &str) -> Self {
        match Self::checked(name) {
            Some(name) => name,
            None => panic!("a commentary name is up to 8 bytes of printable ASCII"),
        }
    }

    /// `name` when it is at most 8 bytes of printable ASCII with no trailing blank.
    const fn checked(name: &str) -> Option<Self> {
        let name = name.as_bytes();
        if name.len() > 8 || (!name.is_empty() && name[name.len() - 1] == b' ') {
            return None;
        }
        let mut bytes = [0; 8];
        let mut i = 0;
        while i < name.len() {
            if !matches!(name[i], 0x20..=0x7e) {
                return None;
            }
            bytes[i] = name[i];
            i += 1;
        }
        Some(Self {
            bytes,
            len: name.len() as u8,
        })
    }

    pub const fn as_str(&self) -> &str {
        match str::from_utf8(self.bytes.split_at(self.len as usize).0) {
            Ok(text) => text,
            Err(_) => panic!("a commentary name holds only ASCII"),
        }
    }
}

impl TryFrom<&str> for CommentaryName {
    type Error = FitsError;

    fn try_from(name: &str) -> Result<Self> {
        Self::checked(name).ok_or_else(|| FitsError::InvalidKeyword {
            name: name.to_string(),
        })
    }
}

impl TryFrom<&String> for CommentaryName {
    type Error = FitsError;

    fn try_from(name: &String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl TryFrom<String> for CommentaryName {
    type Error = FitsError;

    fn try_from(name: String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl FromStr for CommentaryName {
    type Err = FitsError;

    fn from_str(name: &str) -> Result<Self> {
        Self::try_from(name)
    }
}

impl fmt::Debug for CommentaryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

impl fmt::Display for CommentaryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[cfg(test)]
mod tests {
    use crate::error::FitsError;
    use crate::header_model::commentary_name::CommentaryName;

    /// Any printable ASCII up to 8 bytes names a commentary record, the blank name
    /// included; trailing blanks, a 9th byte and other bytes do not.
    #[test]
    fn a_name_is_up_to_eight_bytes_of_printable_ascii() {
        for name in ["", "COMMENT", "ab cd", "a=b/'x'", "CONTINUE"] {
            let parsed = CommentaryName::try_from(name).unwrap();
            assert_eq!(parsed.as_str(), name);
            assert_eq!(name.parse::<CommentaryName>().unwrap(), parsed);
            assert_eq!(CommentaryName::try_from(name.to_string()).unwrap(), parsed);
        }
        for bad in ["ABCDEFGHI", "TRAIL ", "tab\t", "caf\u{e9}"] {
            assert!(
                matches!(CommentaryName::try_from(bad), Err(FitsError::InvalidKeyword { name }) if name == bad),
                "{bad:?}"
            );
        }
    }
}
