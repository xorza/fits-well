//! [`RecordName`]: what names a header record.

use std::fmt;
use std::str::FromStr;

use crate::fits_error::{FitsError, Result};
use crate::header_model::card::validate_valued_keyword;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::hierarch_name::HierarchName;
use crate::keyword::Keyword;

/// The name of a header record: a valued record's [`Keyword`], a `HIERARCH` record's
/// [`HierarchName`], or a commentary record's [`CommentaryName`].
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum RecordName {
    Keyword(Keyword),
    Hierarch(HierarchName),
    Commentary(CommentaryName),
}

impl RecordName {
    /// Whether a valued record can carry this name; [`FitsError::ReservedName`] for a
    /// commentary name and the control and commentary keywords.
    pub(crate) fn validate_valued(&self) -> Result<()> {
        match self {
            Self::Keyword(keyword) => validate_valued_keyword(*keyword),
            Self::Hierarch(_) => Ok(()),
            Self::Commentary(_) => Err(FitsError::ReservedName { name: self.clone() }),
        }
    }
}

impl FromStr for RecordName {
    type Err = FitsError;

    /// `HIERARCH`, a blank and a [`HierarchName`], as the record spells it, or else a
    /// [`Keyword`]; [`FitsError::InvalidKeyword`] for text that is neither. A string
    /// never names a commentary record, whose name a [`CommentaryName`] gives.
    fn from_str(name: &str) -> Result<Self> {
        match name.strip_prefix("HIERARCH ") {
            Some(hierarch) => {
                hierarch
                    .parse()
                    .map(Self::Hierarch)
                    .map_err(|_| FitsError::InvalidKeyword {
                        name: name.to_string(),
                    })
            }
            None => Keyword::try_from(name).map(Self::Keyword),
        }
    }
}

impl TryFrom<&str> for RecordName {
    type Error = FitsError;

    fn try_from(name: &str) -> Result<Self> {
        name.parse()
    }
}

impl TryFrom<&String> for RecordName {
    type Error = FitsError;

    fn try_from(name: &String) -> Result<Self> {
        name.parse()
    }
}

impl TryFrom<String> for RecordName {
    type Error = FitsError;

    fn try_from(name: String) -> Result<Self> {
        name.parse()
    }
}

impl From<Keyword> for RecordName {
    fn from(keyword: Keyword) -> Self {
        Self::Keyword(keyword)
    }
}

impl From<HierarchName> for RecordName {
    fn from(name: HierarchName) -> Self {
        Self::Hierarch(name)
    }
}

impl From<CommentaryName> for RecordName {
    fn from(name: CommentaryName) -> Self {
        Self::Commentary(name)
    }
}

impl fmt::Display for RecordName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keyword(keyword) => fmt::Display::fmt(keyword, f),
            Self::Hierarch(name) => write!(f, "HIERARCH {name}"),
            Self::Commentary(name) => fmt::Display::fmt(name, f),
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::fits_error::FitsError;
    use crate::header_model::commentary_name::CommentaryName;
    use crate::header_model::keywords;
    use crate::header_model::record_name::RecordName;
    use crate::keyword::Keyword;
    use std::assert_matches;

    /// A string names a keyword, or a `HIERARCH` record after the prefix and a blank;
    /// each prints back as it parsed.
    #[test]
    fn string_names_a_keyword_or_a_hierarch_record() {
        let exptime: RecordName = "EXPTIME".parse().unwrap();
        assert_eq!(
            exptime,
            RecordName::Keyword(Keyword::try_from("EXPTIME").unwrap())
        );
        let hierarch: RecordName = "HIERARCH ESO  DET DIT".parse().unwrap();
        assert_eq!(
            hierarch,
            RecordName::Hierarch("ESO DET DIT".parse().unwrap())
        );
        assert_eq!(hierarch.to_string(), "HIERARCH ESO DET DIT");
        assert_eq!(
            RecordName::try_from("HIERARCH ESO DET DIT".to_string()).unwrap(),
            hierarch
        );
        // `HIERARCH` alone is a keyword the standard leaves free.
        assert_matches!(
            "HIERARCH".parse::<RecordName>(),
            Ok(RecordName::Keyword(keyword)) if keyword == "HIERARCH"
        );
        for bad in ["HIERARCH ", "HIERARCH A=B", "exptime", "TOOLONGKEY"] {
            assert_matches!(bad.parse::<RecordName>(), Err(FitsError::InvalidKeyword { name }) if name == bad,
                "{bad:?}"
            );
        }
        for reserved in [
            RecordName::from(CommentaryName::try_from("ab cd").unwrap()),
            RecordName::from(keywords::HISTORY),
        ] {
            assert_matches!(reserved.validate_valued(), Err(FitsError::ReservedName { name }) if name == reserved,
                "{reserved}"
            );
        }
        assert!(exptime.validate_valued().is_ok() && hierarch.validate_valued().is_ok());
    }
}
