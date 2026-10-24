//! [`Record`]: one header record, and [`RecordView`], its borrowed form.

use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::defect::Defect;
use crate::header_model::hierarch_name::HierarchName;
use crate::header_model::record_name::RecordName;
use crate::header_model::value::Value;
use crate::keyword::Keyword;

/// One header record (§4.1), owned: what a header holds, in order, and what the ordered
/// removals return.
#[derive(Debug, Clone, PartialEq)]
pub enum Record {
    /// `KEYWORD = value [/ comment]`, with its inline comment.
    Valued {
        keyword: Keyword,
        value: Value,
        /// Trailing blanks are not significant and are stripped.
        comment: Option<String>,
    },
    /// A record without a value indicator: its name and its free text, whose leading
    /// blanks are content and trailing ones are stripped.
    Commentary {
        name: CommentaryName,
        text: Option<String>,
    },
    /// `HIERARCH name = value [/ comment]` (ESO convention), with its inline comment.
    Hierarch {
        name: HierarchName,
        value: Value,
        comment: Option<String>,
    },
    /// A record that did not parse: its name when it has one, its 80 bytes, and why.
    Malformed {
        name: Option<RecordName>,
        raw: Box<[u8; CARD_SIZE]>,
        defect: Defect,
    },
}

/// One header record, borrowed, as [`Header::iter`](crate::header::Header::iter) yields
/// it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum RecordView<'a> {
    /// `KEYWORD = value [/ comment]`, with its inline comment.
    Valued {
        keyword: Keyword,
        value: &'a Value,
        comment: Option<&'a str>,
    },
    /// `COMMENT`, `HISTORY`, the blank keyword, or any other record without a value
    /// indicator: its name and its free text.
    Commentary {
        name: CommentaryName,
        text: Option<&'a str>,
    },
    /// `HIERARCH name = value [/ comment]` (ESO convention), with its inline comment.
    Hierarch {
        name: &'a HierarchName,
        value: &'a Value,
        comment: Option<&'a str>,
    },
    /// A record that did not parse: its name when it has one, its 80 bytes, and why.
    Malformed {
        name: Option<&'a RecordName>,
        raw: &'a [u8; CARD_SIZE],
        defect: Defect,
    },
}

impl Record {
    /// This record, borrowed.
    pub fn view(&self) -> RecordView<'_> {
        match self {
            Self::Valued {
                keyword,
                value,
                comment,
            } => RecordView::Valued {
                keyword: *keyword,
                value,
                comment: comment.as_deref(),
            },
            Self::Commentary { name, text } => RecordView::Commentary {
                name: *name,
                text: text.as_deref(),
            },
            Self::Hierarch {
                name,
                value,
                comment,
            } => RecordView::Hierarch {
                name,
                value,
                comment: comment.as_deref(),
            },
            Self::Malformed { name, raw, defect } => RecordView::Malformed {
                name: name.as_ref(),
                raw,
                defect: *defect,
            },
        }
    }

    /// A commentary record carrying free text; an empty `text` is a physically blank
    /// record.
    pub(crate) fn commentary(name: CommentaryName, text: &str) -> Self {
        Self::Commentary {
            name,
            text: (!text.is_empty()).then(|| text.to_string()),
        }
    }

    /// The keyword the header's keyword index finds this record by: a valued record's,
    /// or a malformed record's that has one, whose lookup then fails.
    pub(crate) const fn keyword(&self) -> Option<Keyword> {
        match self {
            Self::Valued { keyword, .. }
            | Self::Malformed {
                name: Some(RecordName::Keyword(keyword)),
                ..
            } => Some(*keyword),
            Self::Commentary { .. } | Self::Hierarch { .. } | Self::Malformed { .. } => None,
        }
    }

    /// The name the header's `HIERARCH` index finds this record by: a `HIERARCH`
    /// record's, or a malformed record's that has one, whose lookup then fails.
    pub(crate) const fn hierarch_name(&self) -> Option<&HierarchName> {
        match self {
            Self::Hierarch { name, .. }
            | Self::Malformed {
                name: Some(RecordName::Hierarch(name)),
                ..
            } => Some(name),
            Self::Valued { .. } | Self::Commentary { .. } | Self::Malformed { .. } => None,
        }
    }

    /// Whether this record's name is the `keyword` a commentary or valued record has.
    pub(crate) fn is_named(&self, keyword: Keyword) -> bool {
        match self {
            Self::Commentary { name, .. } => name.as_str() == keyword.as_str(),
            _ => self.keyword() == Some(keyword),
        }
    }

    /// A valued or `HIERARCH` record's value; [`FitsError::InvalidValue`] for a malformed
    /// record.
    pub(crate) fn value_or_defect(&self) -> Result<&Value> {
        match self {
            Self::Valued { value, .. } | Self::Hierarch { value, .. } => Ok(value),
            Self::Malformed { name, defect, .. } => Err(FitsError::InvalidValue {
                name: name.clone(),
                defect: *defect,
            }),
            Self::Commentary { .. } => {
                unreachable!("the indexes hold valued, HIERARCH and malformed records only")
            }
        }
    }

    /// The inline `/`-comment of a valued or `HIERARCH` record.
    pub(crate) fn comment(&self) -> Option<&str> {
        match self {
            Self::Valued { comment, .. } | Self::Hierarch { comment, .. } => comment.as_deref(),
            Self::Commentary { .. } | Self::Malformed { .. } => None,
        }
    }
}

impl RecordView<'_> {
    /// The keyword the record is named by: a valued record's, or a malformed record's
    /// that has one.
    pub const fn keyword(self) -> Option<Keyword> {
        match self {
            Self::Valued { keyword, .. } => Some(keyword),
            Self::Malformed {
                name: Some(RecordName::Keyword(keyword)),
                ..
            } => Some(*keyword),
            Self::Commentary { .. } | Self::Hierarch { .. } | Self::Malformed { .. } => None,
        }
    }

    /// The record's name; `None` for a malformed record whose name is none.
    pub fn name(self) -> Option<RecordName> {
        match self {
            Self::Valued { keyword, .. } => Some(RecordName::Keyword(keyword)),
            Self::Commentary { name, .. } => Some(RecordName::Commentary(name)),
            Self::Hierarch { name, .. } => Some(RecordName::Hierarch(name.clone())),
            Self::Malformed { name, .. } => name.cloned(),
        }
    }
}
