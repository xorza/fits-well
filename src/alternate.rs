//! [`Alternate`], the letter that selects an alternate coordinate description.

use std::fmt;
use std::str;

use crate::error::{FitsError, Result};

/// The letter of an alternate coordinate description, `A` to `Z` (§8.2), the `a` that
/// ends each keyword of that description: `CTYPE1A`. `None` where it is taken selects
/// the primary description.
///
/// A letter is checked when it is made, so it cannot spell another keyword: a digit
/// after `CTYPE1` would name axis 11. Make a constant with [`Alternate::new`], which a
/// `const` checks at compile time, and a letter read at run time with
/// `TryFrom<char>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alternate(u8);

impl Alternate {
    /// The alternate `letter`.
    ///
    /// # Panics
    /// When `letter` is not `A` to `Z`; in a `const`, the build fails instead.
    pub const fn new(letter: char) -> Self {
        match Self::checked(letter) {
            Some(alt) => alt,
            None => panic!("a WCS alternate is a letter A to Z"),
        }
    }

    const fn checked(letter: char) -> Option<Self> {
        if letter.is_ascii_uppercase() {
            Some(Self(letter as u8))
        } else {
            None
        }
    }

    pub const fn as_char(self) -> char {
        self.0 as char
    }

    pub(crate) const fn as_str(&self) -> &str {
        match str::from_utf8(std::slice::from_ref(&self.0)) {
            Ok(text) => text,
            Err(_) => panic!("a WCS alternate is ASCII"),
        }
    }
}

impl fmt::Display for Alternate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<char> for Alternate {
    type Error = FitsError;

    /// Errors with [`FitsError::InvalidAlternate`] unless `letter` is `A` to `Z`.
    fn try_from(letter: char) -> Result<Self> {
        Self::checked(letter).ok_or(FitsError::InvalidAlternate { alternate: letter })
    }
}

#[cfg(test)]
mod tests {
    use crate::alternate::*;

    #[test]
    fn an_alternate_is_one_letter_a_to_z() {
        for letter in ['A', 'M', 'Z'] {
            let alt = Alternate::try_from(letter).unwrap();
            assert_eq!(alt, Alternate::new(letter));
            assert_eq!(alt.as_char(), letter);
            assert_eq!(alt.as_str(), letter.to_string());
            assert_eq!(alt.to_string(), letter.to_string());
        }
        for letter in ['a', '1', ' ', '@', '[', '\u{c4}'] {
            assert!(
                matches!(Alternate::try_from(letter), Err(FitsError::InvalidAlternate { alternate }) if alternate == letter),
                "{letter:?}"
            );
        }
    }

    #[test]
    #[should_panic(expected = "A to Z")]
    fn new_panics_on_a_letter_outside_a_to_z() {
        let _ = Alternate::new(std::hint::black_box('a'));
    }
}
