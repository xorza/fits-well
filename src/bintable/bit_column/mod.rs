//! A borrowed 2-D view of an `X` (bit-array) column.

use std::ops::Index;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::view::BitView;

use crate::bintable::element_kind::ElementKind;
use crate::bintable::table_view::TableView;
use crate::bintable::tform::Storage;

/// A binary table's `X` (bit-array) column as a borrowed, 2-D bit view — from
/// [`ColumnReader::bits`](crate::table::ColumnReader::bits)
/// (rectangular, `row_count × repeat`) or
/// [`ColumnReader::vla_bits`](crate::table::ColumnReader::vla_bits)
/// (jagged `PX`/`QX`). Bits are MSB-first (§7.3.2) and viewed in place over the data
/// unit (zero-copy), so this borrows the table and can't outlive it.
///
/// Index a row (`flags[row]` → a [`BitSlice`]), a bit by nesting (`flags[row][col]`)
/// or by cell (`flags[(row, col)]`), reach for the checked [`get`](Self::get), or take
/// a row with the source lifetime via [`row`](Self::row). Rows are full `bitvec`
/// slices — `count_ones()`, `iter_ones()`, `.to_bitvec()` to own, etc.
///
/// ```
/// # fn read(table: &fits_well::table::Table, row: usize) -> fits_well::Result<()> {
/// let flags = table.column("DQ")?.bits()?;
/// let bit = flags[(row, 3)];             // bool (panics out of range)
/// let bit = flags[row][3];               // same, via the row slice
/// let bit = flags.get(row, 3);           // Option<bool> (checked)
/// let set = flags[row].count_ones();     // bitvec ops on the row
/// # Ok(())
/// # }
/// ```
#[derive(Debug, Clone, Copy)]
pub struct BitColumn<'a> {
    table: TableView<'a>,
    index: usize,
}

impl<'a> BitColumn<'a> {
    pub(crate) const fn new(table: TableView<'a>, index: usize) -> Self {
        BitColumn { table, index }
    }

    /// The number of rows.
    pub const fn len(&self) -> usize {
        self.table.extent.row_count
    }

    /// Whether the column has no rows.
    pub const fn is_empty(&self) -> bool {
        self.table.extent.row_count == 0
    }

    /// Row `r`'s bits as a borrowed [`BitSlice`], MSB-first — resolved on demand from
    /// the data unit (no per-row storage). Index it (`row[c]`), iterate it, or
    /// `.to_bitvec()` to own it. Panics if `r >= len()`.
    pub fn row(&self, r: usize) -> &'a BitSlice<u8, Msb0> {
        assert!(
            r < self.table.extent.row_count,
            "row {r} out of bounds ({} rows)",
            self.table.extent.row_count
        );
        let col = &self.table.columns[self.index];
        if col.tform.storage() == Storage::Fixed {
            // Fixed `rX`: the row's cell, truncated to `repeat` bits.
            &self.table.cell(col, r).view_bits::<Msb0>()[..col.tform.repeat()]
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
                .pq_payload(descriptor, ElementKind::Bit)
                .expect("vla_bits validated every heap span");
            &cell.view_bits::<Msb0>()[..descriptor.count]
        }
    }

    /// The bit at `(row, col)`, MSB-first — `None` if either index is out of range.
    pub fn get(&self, row: usize, col: usize) -> Option<bool> {
        if row >= self.table.extent.row_count {
            return None;
        }
        let bits = self.row(row);
        (col < bits.len()).then(|| bits[col])
    }

    /// Iterate the rows, each a borrowed [`BitSlice`], resolved on demand.
    pub const fn iter(&self) -> BitRows<'a> {
        BitRows {
            column: *self,
            next: 0,
        }
    }
}

impl<'a> IntoIterator for BitColumn<'a> {
    type Item = &'a BitSlice<u8, Msb0>;
    type IntoIter = BitRows<'a>;

    fn into_iter(self) -> BitRows<'a> {
        self.iter()
    }
}

impl<'a> IntoIterator for &BitColumn<'a> {
    type Item = &'a BitSlice<u8, Msb0>;
    type IntoIter = BitRows<'a>;

    fn into_iter(self) -> BitRows<'a> {
        self.iter()
    }
}

/// The rows of a [`BitColumn`], in row order, from [`BitColumn::iter`].
#[derive(Debug, Clone)]
pub struct BitRows<'a> {
    column: BitColumn<'a>,
    next: usize,
}

impl<'a> Iterator for BitRows<'a> {
    type Item = &'a BitSlice<u8, Msb0>;

    fn next(&mut self) -> Option<&'a BitSlice<u8, Msb0>> {
        if self.next == self.column.len() {
            return None;
        }
        self.next += 1;
        Some(self.column.row(self.next - 1))
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        let left = self.column.len() - self.next;
        (left, Some(left))
    }
}

impl ExactSizeIterator for BitRows<'_> {}

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

#[cfg(test)]
mod tests;
