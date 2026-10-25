//! [`Keyword`], its stack-allocated formatting, and the index of an indexed keyword.
//!
//! Indexed keywords (`NAXIS3`, `PV2_15`, `CD1_2`, `CTYPE1`) are looked up
//! constantly while reading and writing. Building each with `format!` heap-
//! allocates a throwaway `String` per lookup — a single [`crate::world_coordinates::Wcs`] parse does
//! ~90 of them. A conforming keyword is at most 8 bytes, so `Keyword::try_from` on
//! `format_args!` formats it into a fixed stack buffer and checks it instead.

use core::fmt;
use std::str::FromStr;

use crate::fits_error::{FitsError, Result};
use crate::inline_name::{InlineName, NameRule};

/// A FITS keyword name: 1 to 8 bytes of `A`–`Z`, `0`–`9`, `-` and `_` (§4.1.2.1),
/// held inline, so it is `Copy` and makes no allocation.
///
/// A keyword is checked when it is made, so a header lookup cannot miss because of the
/// spelling of its argument. Make a constant with [`Keyword::new`], which a `const`
/// checks at compile time; a name read at run time with `TryFrom<&str>`; and an
/// indexed one, such as `NAXIS3`, with [`Keyword::indexed`].
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct Keyword {
    name: InlineName,
}

impl Keyword {
    /// The keyword `name`.
    ///
    /// # Panics
    /// When `name` is no FITS keyword; in a `const`, the build fails instead.
    pub const fn new(name: &str) -> Self {
        match Self::checked(name) {
            Some(keyword) => keyword,
            None => panic!("a FITS keyword is 1 to 8 bytes of A-Z, 0-9, '-' and '_'"),
        }
    }

    const fn checked(name: &str) -> Option<Self> {
        match InlineName::checked(name, NameRule::Keyword) {
            Some(name) => Some(Self { name }),
            None => None,
        }
    }

    /// This root with the decimal `index` after it: `CTYPE` and 2 give `CTYPE2`. Errors
    /// with [`FitsError::InvalidKeyword`] when the result is longer than 8 bytes.
    pub fn indexed(self, index: usize) -> Result<Self> {
        Self::try_from(format_args!("{self}{index}"))
    }

    /// The keyword the formatted `name` spells, or `None` when the name is longer than
    /// a keyword, as a wide axis or column number makes it: no card can hold it.
    ///
    /// # Panics
    /// When the name fits but holds a byte no keyword has: the pattern that formats it
    /// is wrong, which no header can cause.
    pub(crate) fn fitting(name: fmt::Arguments<'_>) -> Option<Self> {
        let formatted = InlineName::formatted(name)?;
        let text = formatted.as_str();
        Some(Self::checked(text).unwrap_or_else(|| panic!("{text:?} is no keyword")))
    }

    pub const fn as_str(&self) -> &str {
        self.name.as_str()
    }

    /// The 1-based index of this keyword in the family of `root`: `NAXIS3` after `NAXIS`
    /// is `Some(3)`. The suffix must be a non-empty digit run with no leading zero, so a
    /// card parses as at most one index (`NAXIS03` is not `NAXIS3`). Callers apply their
    /// own bound on the result.
    pub(crate) fn index_after(self, root: Self) -> Option<usize> {
        let suffix = self.as_str().strip_prefix(root.as_str())?;
        if suffix.is_empty()
            || suffix.starts_with('0')
            || !suffix.bytes().all(|byte| byte.is_ascii_digit())
        {
            return None;
        }
        suffix.parse().ok()
    }
}

impl TryFrom<&str> for Keyword {
    type Error = FitsError;

    fn try_from(name: &str) -> Result<Self> {
        Self::checked(name).ok_or_else(|| FitsError::InvalidKeyword {
            name: name.to_string(),
        })
    }
}

impl TryFrom<&String> for Keyword {
    type Error = FitsError;

    fn try_from(name: &String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl TryFrom<String> for Keyword {
    type Error = FitsError;

    fn try_from(name: String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl FromStr for Keyword {
    type Err = FitsError;

    fn from_str(name: &str) -> Result<Self> {
        Self::try_from(name)
    }
}

impl PartialEq<str> for Keyword {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Keyword {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

/// The keyword a `format_args!` pattern spells, formatted on the stack with no heap
/// allocation: `Keyword::try_from(format_args!("PV{axis}_{m}"))`. Errors with
/// [`FitsError::InvalidKeyword`] when the text is no keyword.
impl TryFrom<fmt::Arguments<'_>> for Keyword {
    type Error = FitsError;

    fn try_from(text: fmt::Arguments<'_>) -> Result<Self> {
        let Some(formatted) = InlineName::formatted(text) else {
            return Err(FitsError::InvalidKeyword {
                name: text.to_string(),
            });
        };
        Self::try_from(formatted.as_str())
    }
}

impl fmt::Display for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl fmt::Debug for Keyword {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self.as_str(), f)
    }
}

#[cfg(test)]
mod tests {
    use crate::keyword::*;
    use std::assert_matches;

    #[test]
    fn formats_keywords_without_allocating() {
        // Mirrors real call sites: inline `{i}`/`{m}`/`{a}` capture scope variables.
        let (i, j, m) = (1usize, 2usize, 15usize);
        let a = "";
        assert_eq!(
            Keyword::try_from(format_args!("NAXIS{i}"))
                .unwrap()
                .as_str(),
            "NAXIS1"
        );
        assert_eq!(
            Keyword::try_from(format_args!("PV{j}_{m}{a}"))
                .unwrap()
                .as_str(),
            "PV2_15"
        );
        let a = "A";
        assert_eq!(
            Keyword::try_from(format_args!("CD{i}_{j}{a}"))
                .unwrap()
                .as_str(),
            "CD1_2A"
        );
        // The long table-WCS spelling with a wide column is 9 bytes: no keyword.
        assert_matches!(
            Keyword::try_from(format_args!("TPV{}_{m}", 123)),
            Err(FitsError::InvalidKeyword { name }) if name == "TPV123_15"
        );
    }

    #[test]
    fn keyword_is_one_to_eight_bytes_of_the_fits_set() {
        for name in ["NAXIS", "DATE-OBS", "CD1_1", "A", "12345678"] {
            let keyword = Keyword::try_from(name).unwrap();
            assert_eq!(keyword.as_str(), name);
            assert_eq!(keyword.to_string(), name);
            assert_eq!(format!("{keyword:?}"), format!("{name:?}"));
            assert_eq!(name.parse::<Keyword>().unwrap(), keyword);
        }
        for name in ["", "naxis", "NAXIS1234", "CCD TEMP", "A.B", "\u{c4}"] {
            assert_matches!(Keyword::try_from(name), Err(FitsError::InvalidKeyword { name: n }) if n == name,
                "{name:?}"
            );
        }
    }

    #[test]
    fn indexed_appends_the_decimal_index() {
        assert_eq!(
            Keyword::new("CTYPE").indexed(2).unwrap(),
            Keyword::new("CTYPE2")
        );
        assert_eq!(
            Keyword::new("NAXIS").indexed(999).unwrap(),
            Keyword::new("NAXIS999")
        );
        // `NAXIS1000` is 9 bytes.
        assert_matches!(
            Keyword::new("NAXIS").indexed(1000),
            Err(FitsError::InvalidKeyword { name }) if name == "NAXIS1000"
        );
    }

    #[test]
    #[should_panic(expected = "1 to 8 bytes")]
    fn new_panics_on_an_invalid_name() {
        let _ = Keyword::new(std::hint::black_box("naxis"));
    }

    /// A formatted name that does not fit is absent, and one that fits but holds a
    /// byte no keyword has is a bug in its pattern, which panics.
    #[test]
    fn fitting_is_absent_only_past_eight_bytes() {
        let column = 17;
        assert_eq!(
            Keyword::fitting(format_args!("TCTYP{column}")),
            Some(Keyword::new("TCTYP17"))
        );
        let column = 1000;
        assert_eq!(Keyword::fitting(format_args!("TCTYP{column}")), None);
    }

    #[test]
    #[should_panic(expected = "\"tctyp17\" is no keyword")]
    fn fitting_panics_on_a_pattern_of_no_keyword() {
        let column = 17;
        let _ = Keyword::fitting(format_args!("tctyp{column}"));
    }

    #[test]
    fn overlong_formatted_text_is_no_keyword() {
        // A caller's column number can make the text any length: 27 bytes here.
        let column = usize::MAX;
        assert_matches!(
            Keyword::try_from(format_args!("1CTYP{column}A")),
            Err(FitsError::InvalidKeyword { name }) if name == format!("1CTYP{column}A")
        );
    }
}
