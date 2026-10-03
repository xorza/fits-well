//! Stack-allocated FITS keyword formatting, and the inverse index parse.
//!
//! Indexed keywords (`NAXIS3`, `PV2_15`, `CD1_2`, `CTYPE1`) are looked up
//! constantly while reading and writing. Building each with `format!` heap-
//! allocates a throwaway `String` per lookup — a single [`crate::world_coordinates::Wcs`] parse does
//! ~90 of them. A conforming keyword is at most 8 bytes, so [`KeyBuf`] formats it
//! into a fixed stack buffer instead; use the [`key!`] macro exactly like
//! `format!` and call `.as_str()` on the result.

use core::fmt::{self, Write};
use std::str;

/// The alternate-description suffix — the `a` of the §8 and Table 22 keyword
/// families. Empty for the primary description, the single letter for an alternate.
///
/// Held in a fixed stack buffer so it can be interpolated into a [`key!`] *and*
/// borrowed as a `&str` (the keyword scans strip it) without the throwaway `String`
/// a `char::to_string` would allocate on every WCS or time parse. Formatting it
/// directly also means each keyword has one spelling rather than a present/absent
/// pair.
#[derive(Debug, Clone, Copy)]
pub(crate) struct AltSuffix {
    buf: [u8; 4],
    len: usize,
}

impl AltSuffix {
    pub(crate) const fn new(alternate: Option<char>) -> AltSuffix {
        let mut suffix = AltSuffix {
            buf: [0; 4],
            len: 0,
        };
        if let Some(alternate) = alternate {
            suffix.len = alternate.encode_utf8(&mut suffix.buf).len();
        }
        suffix
    }

    pub(crate) const fn as_str(&self) -> &str {
        match str::from_utf8(self.buf.split_at(self.len).0) {
            Ok(text) => text,
            Err(_) => panic!("encode_utf8 writes valid UTF-8"),
        }
    }

    /// Whether an alternate description is selected. Several Table 22 families spell
    /// their root differently for the primary description, which has no suffix.
    pub(crate) const fn is_alternate(&self) -> bool {
        self.len != 0
    }
}

impl fmt::Display for AltSuffix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// The 1-based index of an indexed FITS keyword: `index("NAXIS3", "NAXIS")` is
/// `Some(3)`. The suffix must be a non-empty digit run with no leading zero, so a
/// card parses as at most one index (`NAXIS03` is not `NAXIS3`) — the same rule the
/// structural-keyword, compressed-column, and `ZNAMEi` scans each need. Callers
/// apply their own bound on the result.
pub(crate) fn index(keyword: &str, prefix: &str) -> Option<usize> {
    let suffix = keyword.strip_prefix(prefix)?;
    if suffix.is_empty()
        || suffix.starts_with('0')
        || !suffix.bytes().all(|byte| byte.is_ascii_digit())
    {
        return None;
    }
    suffix.parse().ok()
}

/// Capacity of a [`KeyBuf`] — generously above the 8-byte FITS keyword limit (and
/// the longer binary-table-WCS compound forms like `TPC12_34`), so a conforming
/// keyword never overflows it.
const KEY_CAP: usize = 24;

/// A stack buffer holding a formatted keyword for lookup — no heap allocation.
#[derive(Debug)]
pub(crate) struct KeyBuf {
    buf: [u8; KEY_CAP],
    len: usize,
}

impl KeyBuf {
    pub(crate) const fn new() -> KeyBuf {
        KeyBuf {
            buf: [0; KEY_CAP],
            len: 0,
        }
    }

    pub(crate) const fn as_str(&self) -> &str {
        match str::from_utf8(self.buf.split_at(self.len).0) {
            Ok(text) => text,
            Err(_) => panic!("KeyBuf holds only whole `&str` writes"),
        }
    }
}

impl Write for KeyBuf {
    fn write_str(&mut self, s: &str) -> fmt::Result {
        let end = self.len + s.len();
        // A FITS keyword is ≤ 8 bytes; exceeding KEY_CAP means a caller built an
        // impossible keyword — a logic error, not bad file input.
        assert!(
            end <= KEY_CAP,
            "formatted FITS keyword exceeds {KEY_CAP} bytes"
        );
        self.buf[self.len..end].copy_from_slice(s.as_bytes());
        self.len = end;
        Ok(())
    }
}

/// Format an indexed FITS keyword into a stack [`KeyBuf`] — like `format!`, but
/// with no heap allocation. Call `.as_str()` on the result to feed a `Header`
/// lookup: `header.get_real(key!("PV{}_{m}{a}", lat + 1).as_str())?`.
macro_rules! key {
    ($($arg:tt)*) => {{
        let mut k = $crate::keyword::KeyBuf::new();
        core::fmt::Write::write_fmt(&mut k, format_args!($($arg)*))
            .expect("KeyBuf keyword write is infallible");
        k
    }};
}

pub(crate) use key;

#[cfg(test)]
mod tests {
    use crate::keyword::*;

    #[test]
    fn formats_indexed_keywords_without_allocating() {
        // Mirrors real call sites: inline `{i}`/`{m}`/`{a}` capture scope variables.
        let (i, j, m) = (1usize, 2usize, 15usize);
        let a = "";
        assert_eq!(key!("NAXIS{i}").as_str(), "NAXIS1");
        assert_eq!(key!("PV{}_{m}{a}", j).as_str(), "PV2_15");
        let a = "A";
        assert_eq!(key!("CD{}_{}{a}", i, j).as_str(), "CD1_2A");
        assert_eq!(key!("ZNAXIS{i}").as_str(), "ZNAXIS1");
    }

    #[test]
    #[should_panic(expected = "exceeds")]
    fn overlong_keyword_panics() {
        // A keyword far past the 8-byte limit (only reachable by a caller bug).
        let _ = key!("{}", "X".repeat(KEY_CAP + 1)).as_str();
    }
}
