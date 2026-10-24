//! [`RecordName`]: what names a header record.

use std::fmt;

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

impl fmt::Display for RecordName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Keyword(keyword) => fmt::Display::fmt(keyword, f),
            Self::Hierarch(name) => write!(f, "HIERARCH {name}"),
            Self::Commentary(name) => fmt::Display::fmt(name, f),
        }
    }
}

impl From<Keyword> for RecordName {
    fn from(keyword: Keyword) -> Self {
        Self::Keyword(keyword)
    }
}
