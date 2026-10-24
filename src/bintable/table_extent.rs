//! [`TableExtent`]: where a binary table's rows and heap lie in its data unit.

/// A binary table's geometry: `row_count` rows of `row_len` bytes, and the heap from
/// `heap_offset` to `heap_end`, relative to the start of the data unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct TableExtent {
    pub(crate) row_count: usize,
    /// `NAXIS1`.
    pub(crate) row_len: usize,
    /// `THEAP`, else the end of the main table.
    pub(crate) heap_offset: usize,
    /// The main table plus `PCOUNT`: past it is block fill (§6.6).
    pub(crate) heap_end: usize,
}

impl TableExtent {
    /// This extent for a compacted selection of `row_count` rows, whose heap follows the
    /// rows directly and ends at `data_len`.
    pub(crate) fn compacted(self, row_count: usize, data_len: usize) -> Self {
        let heap_offset = row_count * self.row_len;
        assert!(
            heap_offset <= data_len,
            "a compacted selection holds its rows"
        );
        Self {
            row_count,
            row_len: self.row_len,
            heap_offset,
            heap_end: data_len,
        }
    }
}
