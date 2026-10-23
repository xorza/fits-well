//! [`KeywordSpelling`]: the full or the short spelling of a Table-22 keyword root.

/// A Table-22 keyword root's spelling: the full one, or the shorter one an alternate description
/// uses when its suffix takes the last character.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum KeywordSpelling {
    Full,
    Short,
}

impl KeywordSpelling {
    /// Both spellings, the full one first, as a reader tries them.
    pub(super) const BOTH: [Self; 2] = [Self::Full, Self::Short];
}
