//! [`HierarchName`]: the name of a `HIERARCH` record (ESO convention).

use std::fmt;
use std::hash::{Hash, Hasher};
use std::str::FromStr;

use crate::error::FitsError;
use crate::error::Result;

/// The name of a `HIERARCH name = value` record: one or more tokens of printable ASCII
/// other than `=`, kept joined by one blank, as the blanks between tokens carry no
/// meaning. Two names are equal without regard to ASCII case, as cfitsio and astropy
/// compare them; a name keeps the case it was read with.
#[derive(Debug, Clone)]
pub struct HierarchName(Box<str>);

impl HierarchName {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl FromStr for HierarchName {
    type Err = FitsError;

    /// The tokens of `text`; [`FitsError::InvalidKeyword`] for no token, or a byte
    /// outside printable ASCII or an `=` in one.
    fn from_str(text: &str) -> Result<Self> {
        let invalid = || FitsError::InvalidKeyword {
            name: text.to_string(),
        };
        let mut name = String::with_capacity(text.len());
        for token in text.split_ascii_whitespace() {
            if !token
                .bytes()
                .all(|byte| byte.is_ascii_graphic() && byte != b'=')
            {
                return Err(invalid());
            }
            if !name.is_empty() {
                name.push(' ');
            }
            name.push_str(token);
        }
        if name.is_empty() {
            return Err(invalid());
        }
        Ok(Self(name.into_boxed_str()))
    }
}

impl TryFrom<&str> for HierarchName {
    type Error = FitsError;

    fn try_from(text: &str) -> Result<Self> {
        text.parse()
    }
}

impl TryFrom<&String> for HierarchName {
    type Error = FitsError;

    fn try_from(name: &String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl TryFrom<String> for HierarchName {
    type Error = FitsError;

    fn try_from(name: String) -> Result<Self> {
        Self::try_from(name.as_str())
    }
}

impl PartialEq for HierarchName {
    fn eq(&self, other: &Self) -> bool {
        self.0.eq_ignore_ascii_case(&other.0)
    }
}

impl Eq for HierarchName {}

impl Hash for HierarchName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        for byte in self.0.bytes() {
            state.write_u8(byte.to_ascii_uppercase());
        }
    }
}

impl fmt::Display for HierarchName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[cfg(test)]
mod tests {
    use crate::error::FitsError;
    use crate::header_model::hierarch_name::HierarchName;
    use std::collections::HashSet;

    #[test]
    fn a_name_is_its_tokens_equal_without_regard_to_case() {
        let name: HierarchName = "  ESO  DET\tCHIP1 NAME ".parse().unwrap();
        assert_eq!(name.as_str(), "ESO DET CHIP1 NAME");
        let lower: HierarchName = "eso det chip1 name".parse().unwrap();
        assert_eq!(name, lower);
        assert_eq!(lower.as_str(), "eso det chip1 name");
        assert_eq!(
            HashSet::from([name]).len(),
            HashSet::from([lower.clone(), lower]).len()
        );
        // A token past 8 bytes, and characters outside the ESO token rule.
        for name in ["ESO INS OPTI-3 ID", "LONGKEYWORDNAME", "my.key #1"] {
            let parsed = name.parse::<HierarchName>().unwrap();
            assert_eq!(parsed.as_str(), name);
            assert_eq!(HierarchName::try_from(name.to_string()).unwrap(), parsed);
        }
        for bad in ["", "   ", "A=B", "ESO DÉT"] {
            assert!(
                matches!(bad.parse::<HierarchName>(), Err(FitsError::InvalidKeyword { name }) if name == bad),
                "{bad:?}"
            );
        }
    }
}
