//! [`Ragged`]: rows of varying length held in one buffer.

use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;

use crate::bintable::column_data::ColumnData;
use crate::data::unsigned_data::UnsignedData;
use crate::error::FitsError;
use crate::error::Result;

/// Rows of varying length in one buffer: every row's values in `values`, in row
/// order, and where each row ends. Row `r` spans `ends[r - 1]..ends[r]` of the
/// values (from 0 for the first row), so a column of any number of rows costs two
/// allocations, not one per row.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ragged<V> {
    values: V,
    ends: Vec<usize>,
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

    /// # Panics
    /// Unless `ends` never decreases and its last entry is `count`, the length of
    /// `values` — the contract every public constructor states.
    fn checked(values: V, ends: Vec<usize>, count: usize) -> Ragged<V> {
        assert!(
            ends.is_sorted() && ends.last().copied().unwrap_or(0) == count,
            "row ends must rise to the value count {count}"
        );
        Ragged { values, ends }
    }
}

impl<T> Ragged<Vec<T>> {
    /// Rows over `values`, row `r` ending at `ends[r]`.
    ///
    /// # Panics
    /// Unless `ends` never decreases and ends at `values.len()`.
    pub fn new(values: Vec<T>, ends: Vec<usize>) -> Ragged<Vec<T>> {
        let count = values.len();
        Ragged::checked(values, ends, count)
    }

    /// Row `r`'s values.
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn row(&self, r: usize) -> &[T] {
        &self.values[self.range(r)]
    }

    pub fn rows(&self) -> impl ExactSizeIterator<Item = &[T]> {
        (0..self.len()).map(|r| self.row(r))
    }
}

impl<T, R: IntoIterator<Item = T>> FromIterator<R> for Ragged<Vec<T>> {
    fn from_iter<I: IntoIterator<Item = R>>(rows: I) -> Ragged<Vec<T>> {
        let mut values = Vec::new();
        let mut ends = Vec::new();
        for row in rows {
            values.extend(row);
            ends.push(values.len());
        }
        Ragged { values, ends }
    }
}

impl Ragged<String> {
    /// Rows over the text `values`, row `r` ending at byte `ends[r]`.
    ///
    /// # Panics
    /// Unless `ends` never decreases, ends at `values.len()`, and falls on character
    /// boundaries.
    pub fn new(values: String, ends: Vec<usize>) -> Ragged<String> {
        assert!(
            ends.iter().all(|&end| values.is_char_boundary(end)),
            "row ends must fall on character boundaries"
        );
        let count = values.len();
        Ragged::checked(values, ends, count)
    }

    /// Row `r`'s text.
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn row(&self, r: usize) -> &str {
        &self.values[self.range(r)]
    }
}

impl<S: AsRef<str>> FromIterator<S> for Ragged<String> {
    fn from_iter<I: IntoIterator<Item = S>>(rows: I) -> Ragged<String> {
        let mut values = String::new();
        let mut ends = Vec::new();
        for row in rows {
            values.push_str(row.as_ref());
            ends.push(values.len());
        }
        Ragged { values, ends }
    }
}

impl Ragged<BitVec<u8, Msb0>> {
    /// Rows over the bits `values`, row `r` ending at bit `ends[r]`.
    ///
    /// # Panics
    /// Unless `ends` never decreases and ends at `values.len()`.
    pub fn new(values: BitVec<u8, Msb0>, ends: Vec<usize>) -> Ragged<BitVec<u8, Msb0>> {
        let count = values.len();
        Ragged::checked(values, ends, count)
    }

    /// Row `r`'s bits.
    ///
    /// # Panics
    /// When `r` is not a row.
    pub fn row(&self, r: usize) -> &BitSlice<u8, Msb0> {
        &self.values[self.range(r)]
    }
}

impl<B: AsRef<BitSlice<u8, Msb0>>> FromIterator<B> for Ragged<BitVec<u8, Msb0>> {
    fn from_iter<I: IntoIterator<Item = B>>(rows: I) -> Self {
        let mut values = BitVec::new();
        let mut ends = Vec::new();
        for row in rows {
            values.extend_from_bitslice(row.as_ref());
            ends.push(values.len());
        }
        Ragged { values, ends }
    }
}

impl Ragged<ColumnData> {
    /// Rows over the typed `values`, row `r` ending at element `ends[r]`.
    ///
    /// # Panics
    /// Unless `ends` never decreases and ends at `values.element_count()`.
    pub fn new(values: ColumnData, ends: Vec<usize>) -> Ragged<ColumnData> {
        let count = values.element_count();
        Ragged::checked(values, ends, count)
    }

    /// Rows from one [`ColumnData`] per row, which must all hold one type. Errors
    /// for an empty list, which has no type, and for a row of another type.
    pub fn from_rows(rows: impl IntoIterator<Item = ColumnData>) -> Result<Ragged<ColumnData>> {
        let mut rows = rows.into_iter();
        let mut values = rows.next().ok_or(FitsError::EmptyVlaNeedsType)?;
        let mut ends = vec![values.element_count()];
        for (row, data) in rows.enumerate() {
            if !values.extend_from(&data) {
                return Err(FitsError::TypeMismatch {
                    name: format!("variable-length row {}", row + 1),
                    expected: values.type_name(),
                });
            }
            ends.push(values.element_count());
        }
        Ok(Ragged { values, ends })
    }
}

impl Ragged<UnsignedData> {
    pub(crate) fn new(values: UnsignedData, ends: Vec<usize>) -> Ragged<UnsignedData> {
        let count = values.len();
        Ragged::checked(values, ends, count)
    }
}

#[cfg(test)]
mod tests {
    use bitvec::bitvec;
    use bitvec::order::Msb0;
    use bitvec::vec::BitVec;

    use crate::ascii::ascii_text::AsciiText;
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
        assert_eq!(ragged.rows().collect::<Vec<_>>(), [&[][..], &[1, 2], &[3]]);
        let collected: Ragged<Vec<i32>> = [vec![], vec![1, 2], vec![3]].into_iter().collect();
        assert_eq!(collected, ragged);

        let text: Ragged<String> = ["ab", "", "c"].into_iter().collect();
        assert_eq!(text.ends(), [2, 2, 3]);
        assert_eq!(text.row(2), "c");

        let bits: Ragged<BitVec<u8, Msb0>> = [bitvec![u8, Msb0; 1, 0, 1], bitvec![u8, Msb0;]]
            .into_iter()
            .collect();
        assert_eq!(bits.ends(), [3, 3]);
        assert_eq!(bits.row(0), bitvec![u8, Msb0; 1, 0, 1]);
        assert!(bits.row(1).is_empty());

        let fields: AsciiText = [Some("x"), None, Some("yz")].into_iter().collect();
        assert_eq!(
            fields.iter().collect::<Vec<_>>(),
            [Some("x"), None, Some("yz")]
        );
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
}
