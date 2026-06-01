//! Stack-allocated FITS keyword formatting.
//!
//! Indexed keywords (`NAXIS3`, `PV2_15`, `CD1_2`, `CTYPE1`) are looked up
//! constantly while reading and writing. Building each with `format!` heap-
//! allocates a throwaway `String` per lookup — a single [`crate::Wcs`] parse does
//! ~90 of them. A conforming keyword is at most 8 bytes, so [`KeyBuf`] formats it
//! into a fixed stack buffer instead; use the [`key!`] macro exactly like
//! `format!` and call `.as_str()` on the result.

use core::fmt::Write;

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
    pub(crate) fn new() -> KeyBuf {
        KeyBuf {
            buf: [0; KEY_CAP],
            len: 0,
        }
    }

    /// The formatted keyword as a string slice.
    pub(crate) fn as_str(&self) -> &str {
        // Only ASCII keyword bytes are ever written, so this is always valid UTF-8.
        std::str::from_utf8(&self.buf[..self.len]).expect("keyword bytes are ASCII")
    }
}

impl Write for KeyBuf {
    fn write_str(&mut self, s: &str) -> core::fmt::Result {
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
/// lookup: `header.get_real(key!("PV{}_{m}{a}", lat + 1).as_str())`.
macro_rules! key {
    ($($arg:tt)*) => {{
        let mut k = $crate::keyword::KeyBuf::new();
        // `KeyBuf::write_str` never returns `Err` (it asserts on the impossible
        // overflow), so this write is infallible for any conforming keyword.
        core::fmt::Write::write_fmt(&mut k, format_args!($($arg)*))
            .expect("KeyBuf keyword write is infallible");
        k
    }};
}

pub(crate) use key;

#[cfg(test)]
mod tests {
    use super::*;

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
