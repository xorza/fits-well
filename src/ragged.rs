//! [`Ragged`]: rows of varying length held in one buffer.

use std::ops::Range;

use crate::error::Result;

/// Rows of varying length in one buffer: every row's values in `values`, in row
/// order, and where each row ends. Row `r` spans `ends[r - 1]..ends[r]` of the
/// values (from 0 for the first row), so a column of any number of rows costs two
/// allocations, not one per row.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Ragged<V> {
    values: V,
    ends: Vec<usize>,
}

/// The values a [`Ragged`] holds in one buffer, which lend a run of themselves as one
/// row: a `Vec`'s slice, a string's `str`, a column's `ColumnView`.
pub trait RowValues {
    /// One row, borrowed.
    type Row<'a>
    where
        Self: 'a;

    /// The number of values.
    fn len(&self) -> usize;

    fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// The values in `range`.
    fn row(&self, range: Range<usize>) -> Self::Row<'_>;

    /// Whether a row may end at `end`: at any index, or for text at a character
    /// boundary.
    fn is_row_end(&self, end: usize) -> bool {
        end <= self.len()
    }
}

impl<V> Ragged<V> {
    /// The number of rows.
    pub const fn len(&self) -> usize {
        self.ends.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.ends.is_empty()
    }

    /// Where row `r` lies in [`Ragged::values`].
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn range(&self, r: usize) -> Range<usize> {
        let start = if r == 0 { 0 } else { self.ends[r - 1] };
        start..self.ends[r]
    }

    /// Where each row ends in [`Ragged::values`], in row order.
    pub fn ends(&self) -> &[usize] {
        &self.ends
    }

    /// Every row's values, in row order.
    pub const fn values(&self) -> &V {
        &self.values
    }

    pub fn into_values(self) -> V {
        self.values
    }
}

impl<V: RowValues> Ragged<V> {
    /// Rows over `values`, row `r` ending at `ends[r]`.
    ///
    /// # Panics
    /// Unless `ends` never decreases, ends at the value count, and each end may end a
    /// row of `values`, as a text row ends on a character boundary.
    pub fn new(values: V, ends: Vec<usize>) -> Self {
        let count = values.len();
        assert!(
            ends.is_sorted() && ends.last().copied().unwrap_or(0) == count,
            "row ends must rise to the value count {count}"
        );
        assert!(
            ends.iter().all(|&end| values.is_row_end(end)),
            "row ends must fall on row boundaries"
        );
        Self { values, ends }
    }

    /// Row `r`'s values.
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn row(&self, r: usize) -> V::Row<'_> {
        self.values.row(self.range(r))
    }

    /// Every row, in row order.
    pub const fn iter(&self) -> Rows<'_, V> {
        Rows {
            ragged: self,
            next: 0,
        }
    }

    /// Refill these rows in place: `fill` writes the values and each row's end into
    /// this `Ragged`'s own buffers, so a refill of the same size allocates nothing.
    pub(crate) fn refill(
        &mut self,
        fill: impl FnOnce(&mut V, &mut Vec<usize>) -> Result<()>,
    ) -> Result<()> {
        fill(&mut self.values, &mut self.ends)?;
        debug_assert!(
            self.ends.is_sorted() && self.ends.last().copied().unwrap_or(0) == self.values.len(),
            "a refill writes rising row ends up to the value count"
        );
        Ok(())
    }
}

impl<'r, V: RowValues> IntoIterator for &'r Ragged<V> {
    type Item = V::Row<'r>;
    type IntoIter = Rows<'r, V>;

    fn into_iter(self) -> Rows<'r, V> {
        self.iter()
    }
}

/// The rows of a [`Ragged`], in row order, from [`Ragged::iter`].
#[derive(Debug, Clone)]
pub struct Rows<'r, V> {
    ragged: &'r Ragged<V>,
    next: usize,
}

impl<'r, V: RowValues> Iterator for Rows<'r, V> {
    type Item = V::Row<'r>;

    fn next(&mut self) -> Option<V::Row<'r>> {
        if self.next == self.ragged.len() {
            return None;
        }
        self.next += 1;
        Some(self.ragged.row(self.next - 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.ragged.len() - self.next;
        (left, Some(left))
    }
}

impl<V: RowValues> ExactSizeIterator for Rows<'_, V> {}

impl<T> RowValues for Vec<T> {
    type Row<'a>
        = &'a [T]
    where
        T: 'a;

    fn len(&self) -> usize {
        self.as_slice().len()
    }

    fn row(&self, range: Range<usize>) -> &[T] {
        &self[range]
    }
}

impl RowValues for String {
    type Row<'a> = &'a str;

    fn len(&self) -> usize {
        self.as_str().len()
    }

    fn row(&self, range: Range<usize>) -> &str {
        &self[range]
    }

    fn is_row_end(&self, end: usize) -> bool {
        self.is_char_boundary(end)
    }
}

impl<T, R: IntoIterator<Item = T>> FromIterator<R> for Ragged<Vec<T>> {
    fn from_iter<I: IntoIterator<Item = R>>(rows: I) -> Self {
        let mut values = Vec::new();
        let mut ends = Vec::new();
        for row in rows {
            values.extend(row);
            ends.push(values.len());
        }
        Self { values, ends }
    }
}

#[cfg(test)]
mod tests {
    use crate::bintable::column_data::ColumnData;
    use crate::bintable::column_view::ColumnView;
    use crate::ragged::Ragged;

    /// Three rows of lengths 0, 2 and 1 end at 0, 2 and 3; collecting the rows
    /// builds the same buffer and ends as stating them.
    #[test]
    fn rows_end_where_their_values_end() {
        let ragged = Ragged::<Vec<i32>>::new(vec![1, 2, 3], vec![0, 2, 3]);
        assert_eq!(ragged.len(), 3);
        assert_eq!(
            [ragged.range(0), ragged.range(1), ragged.range(2)],
            [0..0, 0..2, 2..3]
        );
        assert_eq!(ragged.iter().collect::<Vec<_>>(), [&[][..], &[1, 2], &[3]]);
        assert_eq!(ragged.iter().len(), 3);
        assert_eq!((&ragged).into_iter().nth(1), Some(&[1, 2][..]));
        let collected: Ragged<Vec<i32>> = [vec![], vec![1, 2], vec![3]].into_iter().collect();
        assert_eq!(collected, ragged);

        let text = Ragged::<String>::new("abc".to_string(), vec![2, 2, 3]);
        assert_eq!(text.row(2), "c");
        assert_eq!(text.iter().collect::<Vec<_>>(), ["ab", "", "c"]);

        // A variable-length column's rows are borrowed views of its one buffer, which
        // own again as the rows they were built from.
        let column = Ragged::<ColumnData>::from_rows([
            ColumnData::I16(vec![4, 5]),
            ColumnData::I16(vec![]),
            ColumnData::I16(vec![6]),
        ])
        .unwrap();
        assert_eq!(column.row(0), ColumnView::I16(&[4, 5]));
        assert_eq!(column.row(0).as_i16(), Some(&[4, 5][..]));
        assert_eq!(column.row(0).as_i32(), None);
        assert!(column.row(1).is_empty());
        assert_eq!(
            column.iter().map(|row| row.to_data()).collect::<Vec<_>>(),
            [
                ColumnData::I16(vec![4, 5]),
                ColumnData::I16(vec![]),
                ColumnData::I16(vec![6])
            ]
        );
        assert_eq!(column.values().view().to_data(), *column.values());
    }

    #[test]
    #[should_panic(expected = "row ends must rise to the value count 3")]
    fn ends_short_of_the_values_are_refused() {
        let _ = Ragged::<Vec<i32>>::new(vec![1, 2, 3], vec![2]);
    }

    #[test]
    #[should_panic(expected = "row ends must rise to the value count 2")]
    fn falling_ends_are_refused() {
        let _ = Ragged::<Vec<i32>>::new(vec![1, 2], vec![2, 1, 2]);
    }

    #[test]
    #[should_panic(expected = "row ends must fall on row boundaries")]
    fn text_ends_inside_a_character_are_refused() {
        let _ = Ragged::<String>::new("é".to_string(), vec![1, 2]);
    }
}
