//! [`InlineName`]: up to 8 bytes of a header record's name, held inline.

use std::fmt::{self, Write};
use std::str;

/// Up to 8 bytes of a record's name, held inline, so a name is `Copy` and makes no
/// allocation: the storage [`Keyword`](crate::keyword::Keyword) and
/// [`CommentaryName`](crate::header_model::commentary_name::CommentaryName) share, each
/// under its own [`NameRule`]. Formatted into, it is the stack buffer an indexed keyword
/// is spelled in.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct InlineName {
    bytes: [u8; 8],
    len: u8,
}

/// The bytes a name may hold, the rule of its record kind (§4.1.2.1).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum NameRule {
    /// 1 to 8 bytes of `A`–`Z`, `0`–`9`, `-` and `_`.
    Keyword,
    /// Up to 8 bytes of printable ASCII with no trailing blank; the blank name is `""`.
    Commentary,
}

impl NameRule {
    const fn admits(self, name: &[u8]) -> bool {
        if name.len() > 8 {
            return false;
        }
        let edges = match self {
            Self::Keyword => !name.is_empty(),
            Self::Commentary => name.is_empty() || name[name.len() - 1] != b' ',
        };
        if !edges {
            return false;
        }
        let mut i = 0;
        while i < name.len() {
            let byte = name[i];
            let allowed = match self {
                Self::Keyword => {
                    byte.is_ascii_uppercase()
                        || byte.is_ascii_digit()
                        || byte == b'-'
                        || byte == b'_'
                }
                Self::Commentary => matches!(byte, 0x20..=0x7e),
            };
            if !allowed {
                return false;
            }
            i += 1;
        }
        true
    }
}

impl InlineName {
    const EMPTY: Self = Self {
        bytes: [0; 8],
        len: 0,
    };

    /// `name` when `rule` admits it.
    pub(crate) const fn checked(name: &str, rule: NameRule) -> Option<Self> {
        let name = name.as_bytes();
        if !rule.admits(name) {
            return None;
        }
        let mut out = Self::EMPTY;
        let mut i = 0;
        while i < name.len() {
            out.bytes[i] = name[i];
            i += 1;
        }
        out.len = name.len() as u8;
        Some(out)
    }

    /// The text `text` formats to, unchecked, or `None` when it is longer than 8 bytes,
    /// which no name is.
    pub(crate) fn formatted(text: fmt::Arguments<'_>) -> Option<Self> {
        let mut out = Self::EMPTY;
        out.write_fmt(text).ok()?;
        Some(out)
    }

    pub(crate) const fn as_str(&self) -> &str {
        match str::from_utf8(self.bytes.split_at(self.len as usize).0) {
            Ok(text) => text,
            Err(_) => panic!("a name holds only whole `&str` writes"),
        }
    }
}

impl Write for InlineName {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let start = usize::from(self.len);
        let end = start + s.len();
        if end > self.bytes.len() {
            return Err(fmt::Error);
        }
        self.bytes[start..end].copy_from_slice(s.as_bytes());
        self.len = end as u8;
        Ok(())
    }
}

impl fmt::Debug for InlineName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}
