//! [`AsciiText`]: an ASCII-table text column's fields.

use crate::ragged::Ragged;

/// The fields of an ASCII-table `Aw` column, in row order: every row's text in one
/// buffer, and which rows are undefined (`TNULLn`). An undefined row has no text.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AsciiText {
    text: Ragged<String>,
    defined: Vec<bool>,
}

impl AsciiText {
    /// The number of rows.
    pub const fn len(&self) -> usize {
        self.defined.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.defined.is_empty()
    }

    /// Row `r`'s text, or `None` for an undefined row.
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn get(&self, r: usize) -> Option<&str> {
        self.defined[r].then(|| self.text.row(r))
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = Option<&str>> {
        (0..self.len()).map(|r| self.get(r))
    }

    /// Whether any row is undefined.
    pub(crate) fn has_null(&self) -> bool {
        self.defined.contains(&false)
    }
}

impl<S: AsRef<str>> FromIterator<Option<S>> for AsciiText {
    fn from_iter<I: IntoIterator<Item = Option<S>>>(rows: I) -> AsciiText {
        let mut text = String::new();
        let mut ends = Vec::new();
        let mut defined = Vec::new();
        for row in rows {
            if let Some(row) = &row {
                text.push_str(row.as_ref());
            }
            ends.push(text.len());
            defined.push(row.is_some());
        }
        AsciiText {
            text: Ragged::<String>::new(text, ends),
            defined,
        }
    }
}
