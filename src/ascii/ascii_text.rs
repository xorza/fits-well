//! [`AsciiText`]: an ASCII-table text column's fields.

use crate::ragged::Ragged;

/// The fields of an ASCII-table `Aw` column, in row order: every row's text in one
/// buffer, and which rows are undefined (`TNULLn`). An undefined row has no text. A
/// field's trailing blanks pad it to its width and are dropped when it is made, as
/// cfitsio and astropy drop them; its leading blanks are text.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
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
    pub fn row(&self, r: usize) -> Option<&str> {
        self.defined[r].then(|| self.text.row(r))
    }

    pub const fn iter(&self) -> AsciiRows<'_> {
        AsciiRows {
            text: self,
            next: 0,
        }
    }

    /// This column refilled with `rows`, as [`FromIterator`] makes one, its buffers reused.
    pub(crate) fn refill<S: AsRef<str>>(&mut self, rows: impl IntoIterator<Item = Option<S>>) {
        let Self { text, defined } = self;
        defined.clear();
        text.refill(|text, ends| {
            text.clear();
            ends.clear();
            for row in rows {
                if let Some(row) = &row {
                    text.push_str(row.as_ref().trim_end_matches(' '));
                }
                ends.push(text.len());
                defined.push(row.is_some());
            }
            Ok(())
        })
        .expect("a refill that fails on no row");
    }

    /// Whether any row is undefined.
    pub(crate) fn has_null(&self) -> bool {
        self.defined.contains(&false)
    }
}

impl<S: AsRef<str>> FromIterator<Option<S>> for AsciiText {
    fn from_iter<I: IntoIterator<Item = Option<S>>>(rows: I) -> Self {
        let mut text = String::new();
        let mut ends = Vec::new();
        let mut defined = Vec::new();
        for row in rows {
            if let Some(row) = &row {
                text.push_str(row.as_ref().trim_end_matches(' '));
            }
            ends.push(text.len());
            defined.push(row.is_some());
        }
        Self {
            text: Ragged::<String>::new(text, ends),
            defined,
        }
    }
}

impl<'a> IntoIterator for &'a AsciiText {
    type Item = Option<&'a str>;
    type IntoIter = AsciiRows<'a>;

    fn into_iter(self) -> AsciiRows<'a> {
        self.iter()
    }
}

/// The rows of an [`AsciiText`], in row order, from [`AsciiText::iter`].
#[derive(Debug, Clone)]
pub struct AsciiRows<'a> {
    text: &'a AsciiText,
    next: usize,
}

impl<'a> Iterator for AsciiRows<'a> {
    type Item = Option<&'a str>;

    fn next(&mut self) -> Option<Option<&'a str>> {
        if self.next == self.text.len() {
            return None;
        }
        self.next += 1;
        Some(self.text.row(self.next - 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.text.len() - self.next;
        (left, Some(left))
    }
}

impl ExactSizeIterator for AsciiRows<'_> {}
