//! A borrowed 2-D view of an `X` (bit-array) column.

use std::ops::Index;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::view::BitView;

use crate::table_impl::BinTable;
use crate::table_impl::tform_kind::TformKind;

/// A binary table's `X` (bit-array) column as a borrowed, 2-D bit view — from
/// [`ColumnReader::bits`](crate::table::ColumnReader::bits)
/// (rectangular, `nrows × repeat`) or
/// [`ColumnReader::vla_bits`](crate::table::ColumnReader::vla_bits)
/// (jagged `PX`/`QX`). Bits are MSB-first (§7.3.2) and viewed in place over the data
/// unit (zero-copy), so this borrows the table and can't outlive it.
///
/// Index a row (`flags[row]` → a [`BitSlice`]), a bit by nesting (`flags[row][col]`)
/// or by cell (`flags[(row, col)]`), reach for the checked [`get`](Self::get), or take
/// a row with the source lifetime via [`row`](Self::row). Rows are full `bitvec`
/// slices — `count_ones()`, `iter_ones()`, `.to_bitvec()` to own, etc.
///
/// ```ignore
/// let flags = table.column_by_name("DQ")?.bits()?;
/// let bit = flags[(row, 3)];             // bool (panics out of range)
/// let bit = flags[row][3];               // same, via the row slice
/// let bit = flags.get(row, 3);           // Option<bool> (checked)
/// let set = flags[row].count_ones();     // bitvec ops on the row
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BitColumn<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> BitColumn<'a> {
    pub(super) fn new(table: &'a BinTable, index: usize) -> BitColumn<'a> {
        BitColumn { table, index }
    }

    /// The number of rows.
    pub fn nrows(&self) -> usize {
        self.table.schema.nrows
    }

    /// Whether the column has no rows.
    pub fn is_empty(&self) -> bool {
        self.table.schema.nrows == 0
    }

    /// Row `r`'s bits as a borrowed [`BitSlice`], MSB-first — resolved on demand from
    /// the data unit (no per-row storage). Index it (`row[c]`), iterate it, or
    /// `.to_bitvec()` to own it. Panics if `r >= nrows()`.
    pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0> {
        assert!(
            r < self.table.schema.nrows,
            "row {r} out of bounds ({} rows)",
            self.table.schema.nrows
        );
        let col = &self.table.schema.columns[self.index];
        if col.tform.kind == TformKind::Bit {
            // Fixed `rX`: the row's cell, truncated to `repeat` bits.
            &self.table.cell(col, r).view_bits::<Msb0>()[..col.tform.repeat]
        } else {
            // Variable-length `PX`/`QX`: follow the descriptor into the heap. The span
            // was bounds-checked by `vla_bits`, so the lookup can't fail here.
            let descriptor = self
                .table
                .pq_descriptor(col, r)
                .expect("vla_bits validated every descriptor");
            if descriptor.count == 0 {
                return BitSlice::empty();
            }
            let cell = self
                .table
                .pq_payload(descriptor, TformKind::Bit)
                .expect("vla_bits validated every heap span");
            &cell.view_bits::<Msb0>()[..descriptor.count]
        }
    }

    /// The bit at `(row, col)`, MSB-first — `None` if either index is out of range.
    pub fn get(&self, row: usize, col: usize) -> Option<bool> {
        if row >= self.table.schema.nrows {
            return None;
        }
        let bits = self.row(row);
        (col < bits.len()).then(|| bits[col])
    }

    /// Iterate the rows, each a borrowed [`BitSlice`], resolved on demand.
    pub fn iter(&self) -> impl ExactSizeIterator<Item = &'a BitSlice<u8, Msb0>> + '_ {
        (0..self.table.schema.nrows).map(move |r| self.row(r))
    }
}

/// `bits[row]` is row `row`'s [`BitSlice`] (panics out of range, like slice indexing);
/// `bits[row][col]` is the bit. Use [`BitColumn::get`] for the checked element.
impl Index<usize> for BitColumn<'_> {
    type Output = BitSlice<u8, Msb0>;

    fn index(&self, row: usize) -> &BitSlice<u8, Msb0> {
        self.row(row)
    }
}

/// `bits[(row, col)]` is the bit at that cell (panics out of range) — the matrix-style
/// counterpart of [`BitColumn::get`].
impl Index<(usize, usize)> for BitColumn<'_> {
    type Output = bool;

    fn index(&self, (row, col): (usize, usize)) -> &bool {
        &self.row(row)[col]
    }
}
