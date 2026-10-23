//! [`HduSelector`]: which HDU a read names.

/// Which HDU a read names: its 0-based position in the file, or an extension by its
/// `EXTNAME`, compared case-insensitively, and its `EXTVER` and `EXTLEVEL` when given
/// (each 1 where the card is absent, §4.4.2.6). A primary HDU with no `EXTNAME` is
/// named `PRIMARY`, as astropy names it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HduSelector<'a> {
    Index(usize),
    Extension {
        name: &'a str,
        version: Option<i64>,
        level: Option<i64>,
    },
}

impl From<usize> for HduSelector<'_> {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl<'a> From<&'a str> for HduSelector<'a> {
    fn from(name: &'a str) -> Self {
        Self::Extension {
            name,
            version: None,
            level: None,
        }
    }
}

impl<'a> From<(&'a str, i64)> for HduSelector<'a> {
    fn from((name, version): (&'a str, i64)) -> Self {
        Self::Extension {
            name,
            version: Some(version),
            level: None,
        }
    }
}
