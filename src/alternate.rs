//! [`Alternate`], which coordinate description a WCS or time read selects.

use std::fmt;
use std::str;

use crate::fits_error::{FitsError, Result};

/// Which coordinate description a read selects (§8.2): the primary one, or an
/// alternate one named by a letter `A` to `Z`, the `a` that ends each keyword of that
/// description: `CTYPE1A`.
///
/// Every method that selects a description takes `impl TryInto<Alternate>`: `None` or
/// [`Alternate::PRIMARY`] for the primary description, and `'A'`, `Some('A')` or
/// [`Alternate::new`] for an alternate one. A letter is checked when it is made, so it
/// cannot spell another keyword: a digit after `CTYPE1` would name axis 11. A bare
/// `None` infers its type because `Option<char>` is the one `Option` this converts
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Alternate(Option<u8>);

impl Alternate {
    /// The primary description, whose keywords have no suffix.
    pub const PRIMARY: Self = Self(None);

    /// The alternate description `letter` names.
    ///
    /// # Panics
    /// When `letter` is not `A` to `Z`; in a `const`, the build fails instead.
    pub const fn new(letter: char) -> Self {
        match Self::letter_checked(letter) {
            Some(alt) => alt,
            None => panic!("a WCS alternate is a letter A to Z"),
        }
    }

    const fn letter_checked(letter: char) -> Option<Self> {
        if letter.is_ascii_uppercase() {
            Some(Self(Some(letter as u8)))
        } else {
            None
        }
    }

    /// The letter of an alternate description; `None` for the primary one.
    pub const fn letter(self) -> Option<char> {
        match self.0 {
            Some(letter) => Some(letter as char),
            None => None,
        }
    }

    /// Whether this is an alternate description. Several Table 22 families spell their
    /// root differently for the primary description, which has no suffix.
    pub(crate) const fn is_alternate(self) -> bool {
        self.0.is_some()
    }

    /// The suffix this description's keywords end with: empty for the primary one.
    pub(crate) const fn as_str(&self) -> &str {
        let suffix = match &self.0 {
            Some(letter) => std::slice::from_ref(letter),
            None => &[],
        };
        match str::from_utf8(suffix) {
            Ok(text) => text,
            Err(_) => panic!("a WCS alternate is ASCII"),
        }
    }

    /// The description `alt` selects, its conversion error as a [`FitsError`].
    pub(crate) fn of(alt: impl TryInto<Self, Error: Into<FitsError>>) -> Result<Self> {
        alt.try_into().map_err(Into::into)
    }
}

/// The keyword suffix: empty for the primary description, the letter for an
/// alternate one.
impl fmt::Display for Alternate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

impl TryFrom<char> for Alternate {
    type Error = FitsError;

    /// Errors with [`FitsError::InvalidAlternate`] unless `letter` is `A` to `Z`.
    fn try_from(letter: char) -> Result<Self> {
        Self::letter_checked(letter).ok_or(FitsError::InvalidAlternate { alternate: letter })
    }
}

impl TryFrom<Option<char>> for Alternate {
    type Error = FitsError;

    /// The primary description for `None`; else as `TryFrom<char>`.
    fn try_from(letter: Option<char>) -> Result<Self> {
        letter.map_or(Ok(Self::PRIMARY), Self::try_from)
    }
}

#[cfg(test)]
mod tests {
    use crate::alternate::*;

    #[test]
    fn an_alternate_is_the_primary_description_or_one_letter_a_to_z() {
        for letter in ['A', 'M', 'Z'] {
            let alt = Alternate::try_from(letter).unwrap();
            assert_eq!(alt, Alternate::new(letter));
            assert_eq!(Alternate::try_from(Some(letter)).unwrap(), alt);
            assert_eq!(Alternate::of(letter).unwrap(), alt);
            assert_eq!(alt.letter(), Some(letter));
            assert!(alt.is_alternate());
            assert_eq!(alt.as_str(), letter.to_string());
            assert_eq!(alt.to_string(), letter.to_string());
        }
        assert_eq!(Alternate::try_from(None).unwrap(), Alternate::PRIMARY);
        assert_eq!(Alternate::of(None).unwrap(), Alternate::PRIMARY);
        assert_eq!(
            Alternate::of(Alternate::PRIMARY).unwrap(),
            Alternate::PRIMARY
        );
        assert_eq!(Alternate::PRIMARY.letter(), None);
        assert!(!Alternate::PRIMARY.is_alternate());
        assert_eq!(Alternate::PRIMARY.to_string(), "");
        for letter in ['a', '1', ' ', '@', '[', '\u{c4}'] {
            for result in [
                Alternate::try_from(letter),
                Alternate::try_from(Some(letter)),
            ] {
                assert!(
                    matches!(result, Err(FitsError::InvalidAlternate { alternate }) if alternate == letter),
                    "{letter:?}"
                );
            }
        }
    }

    #[test]
    #[should_panic(expected = "A to Z")]
    fn new_panics_on_a_letter_outside_a_to_z() {
        let _ = Alternate::new(std::hint::black_box('a'));
    }
}
