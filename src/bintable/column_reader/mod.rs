//! A decode handle bound to one column of a binary table.

use std::ops::Range;

use num_complex::Complex;

use crate::bintable::bit_column::BitColumn;
use crate::bintable::column::Column;
use crate::bintable::column_data::ColumnData;
use crate::bintable::element_kind::{Cell, ElementKind};
use crate::bintable::table_view::TableView;
use crate::bintable::tform::Storage;
use crate::bintable::vla_column::VlaColumn;
use crate::fits_error::ExpectedColumn;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::unsigned_data::UnsignedData;
use crate::ragged::Ragged;

/// A handle to one column of a [`Table`], from [`Table::column`]. Decode through it without
/// re-passing the column descriptor, into buffers the caller keeps across reads:
/// [`raw_into`](Self::raw_into) for the typed values, [`physical_into`](Self::physical_into)
/// for the scaled plane, [`unsigned_into`](Self::unsigned_into)/[`complex`](Self::complex)/
/// [`bits`](Self::bits) for the special kinds, and [`vla_into`](Self::vla_into) (+
/// [`vla_physical_into`](Self::vla_physical_into)/[`vla_unsigned`](Self::vla_unsigned)/
/// [`vla_complex`](Self::vla_complex)/[`vla_bits`](Self::vla_bits)) for variable-length
/// `P`/`Q` columns. Borrows the table, so it cannot outlive it.
///
/// [`Table`]: crate::table::Table
/// [`Table::column`]: crate::table::Table::column
#[derive(Debug, Clone, Copy)]
pub struct ColumnReader<'a> {
    table: TableView<'a>,
    index: usize,
}

impl<'a> ColumnReader<'a> {
    pub(crate) const fn new(table: TableView<'a>, index: usize) -> Self {
        ColumnReader { table, index }
    }

    /// The column's [`Column`] descriptor — name, `TFORMn`, `TSCALn`/`TZEROn`/`TNULLn`,
    /// `TDIMn`, `TDISPn`.
    pub const fn descriptor(&self) -> &'a Column {
        &self.table.columns[self.index]
    }

    /// The column descriptor, rejecting a variable-length (`P`/`Q`) column — the
    /// shared precondition of every fixed-width decode. Those columns carry heap
    /// descriptors rather than values, so they go through [`ColumnReader::vla_into`].
    const fn fixed_descriptor(&self) -> Result<&'a Column> {
        let col = self.descriptor();
        match col.tform.storage() {
            Storage::Fixed => Ok(col),
            Storage::Array(_) => Err(self.mismatch(ExpectedColumn::Fixed)),
        }
    }

    /// The refusal of a read that wanted `expected` of this column.
    const fn mismatch(&self, expected: ExpectedColumn) -> FitsError {
        FitsError::ColumnKindMismatch {
            column: self.index,
            expected,
        }
    }

    /// Decode a fixed-width column into `out` as a typed, row-flattened [`ColumnData`],
    /// whose buffer it reuses when `out` already holds values of this column's type: an
    /// `A` column keeps every row's field bytes exactly, padding and terminator included —
    /// read one through [`CharacterField`](crate::table::CharacterField) — and every other
    /// fixed kind decodes from the concatenated cell bytes. Variable-length (`P`/`Q`)
    /// columns error here — use [`ColumnReader::vla_into`].
    pub fn raw_into(&self, out: &mut ColumnData) -> Result<()> {
        let col = self.fixed_descriptor()?;
        col.decode_cells_into(self.table.cells(col), self.table.extent.row_count, out)
    }

    /// Append the numeric column scaled to its physical plane to `out`: `TZEROn + TSCALn ×
    /// raw`, mapping integers equal to `TNULLn` to `NaN`. The arithmetic runs in `f64`, and
    /// an `f32` buffer narrows its result. Errors for the non-numeric kinds
    /// (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn physical_into<T: PhysicalSample>(&self, out: &mut Vec<T>) -> Result<()> {
        let col = self.fixed_descriptor()?;
        let element = col.tform.element();
        if element.bitpix().is_none() {
            return Err(self.mismatch(ExpectedColumn::Numeric));
        }
        element.decode_physical_into(
            self.table.cells(col),
            self.table.extent.row_count * col.tform.repeat(),
            &col.scaling()?,
            out,
        )
    }

    /// Exact typed integers into `out` when the column uses the FITS unsigned (or
    /// signed-byte) convention — `TSCALn == 1`, no `TNULLn`, `TZEROn` the matching sign-bit
    /// offset on a `B`/`I`/`J`/`K` column — without the `f64` rounding of
    /// [`physical_into`](Self::physical_into); `out`'s storage is reused when it holds the
    /// column's kind. `Ok(false)`, and `out` as it was, for any other column; errors only
    /// for a variable-length column. Mirrors [`crate::image::Borrowed::unsigned_into`].
    pub fn unsigned_into(&self, out: &mut UnsignedData) -> Result<bool> {
        let col = self.fixed_descriptor()?;
        let Some(kind) = col.tform.element().unsigned_kind(&col.scaling()?) else {
            return Ok(false);
        };
        if out.kind() != kind {
            *out = UnsignedData::empty(kind);
        }
        out.fill_from_be_cells(
            self.table.cells(col),
            self.table.extent.row_count * col.tform.repeat(),
        )?;
        Ok(true)
    }

    /// A `C`/`M` complex column as [`Complex<f64>`] values, applying `TSCALn` to both
    /// components and `TZEROn` to the real component (§7.3.2). Errors on non-complex columns.
    pub fn complex(&self) -> Result<Vec<Complex<f64>>> {
        let col = self.fixed_descriptor()?;
        let element = col.tform.element();
        if !element.is_complex() {
            return Err(self.mismatch(ExpectedColumn::Complex));
        }
        element.decode_complex(
            self.table.cells(col),
            self.table.extent.row_count * col.tform.repeat(),
            col.tscale()?,
            col.tzero()?,
        )
    }

    /// An `X` (bit-array) column as a borrowed 2-D [`BitColumn`] — `row_count × repeat`
    /// bits viewed in place over the data unit, MSB-first (bit 0 is the MSB of the
    /// first byte, §7.3.2), with no per-row allocation. Errors on any non-`X` column.
    pub fn bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        if col.tform.storage() != Storage::Fixed || col.tform.element() != ElementKind::Bit {
            return Err(self.mismatch(ExpectedColumn::Bits));
        }
        Ok(BitColumn::new(self.table, self.index))
    }

    /// Decode a variable-length (`P`/`Q`) column into `out`, whose buffers it reuses:
    /// every row's heap array, in one [`ColumnData`] with the row ends — a row may be
    /// empty. A `PX` row is its descriptor's count of bits. Errors for fixed-width columns.
    pub fn vla_into(&self, out: &mut Ragged<ColumnData>) -> Result<()> {
        let column = self.vla_column()?;
        let row_count = self.table.extent.row_count;
        let total = VlaCells::element_total(column, row_count)?;
        out.refill(|values, ends| {
            let mut cells = VlaCells::new(column, row_count, ends);
            let decoded = column
                .element_type()
                .decode_cells_into(&mut cells, total, values);
            cells.finish()?;
            decoded
        })
    }

    /// Each row of a `P`/`Q` column scaled to its physical plane into `out`, whose buffers
    /// it reuses: `TZEROn + TSCALn × element`, mapping integers equal to `TNULLn` to `NaN`
    /// (§6.4 — scaling applies to the heap values). Errors for fixed-width or
    /// non-numeric-heap columns.
    pub fn vla_physical_into<T: PhysicalSample>(&self, out: &mut Ragged<Vec<T>>) -> Result<()> {
        let column = self.vla_column()?;
        let element = column.element_type();
        if element.bitpix().is_none() {
            return Err(self.mismatch(ExpectedColumn::Numeric));
        }
        let scaling = self.descriptor().scaling()?;
        let row_count = self.table.extent.row_count;
        let total = VlaCells::element_total(column, row_count)?;
        out.refill(|values, ends| {
            values.clear();
            let mut cells = VlaCells::new(column, row_count, ends);
            let decoded = element.decode_physical_into(
                cells.by_ref().map(|cell| cell.bytes),
                total,
                &scaling,
                values,
            );
            cells.finish()?;
            decoded
        })
    }

    /// Exact typed integers for each row of a `P`/`Q` heap array using the FITS
    /// unsigned (or signed-byte) convention. Returns `Ok(None)` unless the heap type
    /// and `TSCALn`/`TZEROn`/`TNULLn` metadata form that convention. Errors for
    /// fixed-width columns.
    pub fn vla_unsigned(&self) -> Result<Option<Ragged<UnsignedData>>> {
        let column = self.vla_column()?;
        let Some(kind) = column
            .element_type()
            .unsigned_kind(&self.descriptor().scaling()?)
        else {
            return Ok(None);
        };
        let row_count = self.table.extent.row_count;
        let total = VlaCells::element_total(column, row_count)?;
        let mut ends = Vec::new();
        let mut cells = VlaCells::new(column, row_count, &mut ends);
        let values =
            UnsignedData::from_be_cells(cells.by_ref().map(|cell| cell.bytes), total, kind);
        cells.finish()?;
        Ok(Some(Ragged::<UnsignedData>::new(values?, ends)))
    }

    /// Each row of a `PC`/`QC`/`PM`/`QM` heap array as [`Complex<f64>`], applying
    /// `TSCALn` to both components and `TZEROn` to only the real component (§7.3.2).
    /// Empty descriptors produce empty rows. Errors for fixed-width or non-complex
    /// heap columns.
    pub fn vla_complex(&self) -> Result<Ragged<Vec<Complex<f64>>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        let element = column.element_type();
        if !element.is_complex() {
            return Err(self.mismatch(ExpectedColumn::Complex));
        }
        let (tscale, tzero) = (col.tscale()?, col.tzero()?);
        let row_count = self.table.extent.row_count;
        let total = VlaCells::element_total(column, row_count)?;
        let mut ends = Vec::new();
        let mut cells = VlaCells::new(column, row_count, &mut ends);
        let values =
            element.decode_complex(cells.by_ref().map(|cell| cell.bytes), total, tscale, tzero);
        cells.finish()?;
        Ok(Ragged::<Vec<Complex<f64>>>::new(values?, ends))
    }

    /// A variable-length `X` (`1PX`/`1QX`) column as a borrowed 2-D [`BitColumn`],
    /// MSB-first (§7.3.2/§7.3.5 — the descriptor's element count is the bit count). The
    /// rows are *jagged* (each its own length), so [`BitColumn::row`]`(r).len()` gives a
    /// row's width. Errors on any non-bit VLA.
    pub fn vla_bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        if !matches!(col.tform.storage(), Storage::Array(_))
            || col.tform.element() != ElementKind::Bit
        {
            return Err(self.mismatch(ExpectedColumn::Bits));
        }
        // Validate every row's heap span up front (no allocation) so [`BitColumn::row`]
        // can resolve a row lazily and infallibly — the only place an overrun surfaces.
        let column = self.vla_column()?;
        for row in 0..self.table.extent.row_count {
            column.cell(row)?;
        }
        Ok(BitColumn::new(self.table, self.index))
    }

    const fn vla_element_type(&self) -> Result<ElementKind> {
        let tform = self.descriptor().tform;
        match tform.storage() {
            Storage::Array(_) => Ok(tform.element()),
            Storage::Fixed => Err(self.mismatch(ExpectedColumn::Array)),
        }
    }

    pub(crate) fn vla_column(&self) -> Result<VlaColumn<'a>> {
        Ok(VlaColumn::new(
            self.table,
            self.index,
            self.vla_element_type()?,
        ))
    }
}

/// The rows of a `P`/`Q` column in one pass, as a decode reads them: each row's cell
/// validated, its end pushed to `ends`, and its bytes given. The first invalid row ends
/// the pass and keeps its error for [`VlaCells::finish`].
#[derive(Debug)]
struct VlaCells<'e, 'a> {
    column: VlaColumn<'a>,
    rows: Range<usize>,
    end: usize,
    ends: &'e mut Vec<usize>,
    error: Option<FitsError>,
}

impl<'e, 'a> VlaCells<'e, 'a> {
    /// The `row_count` rows of `column`, their ends written to `ends`, cleared first.
    fn new(column: VlaColumn<'a>, row_count: usize, ends: &'e mut Vec<usize>) -> Self {
        ends.clear();
        ends.reserve(row_count);
        Self {
            column,
            rows: 0..row_count,
            end: 0,
            ends,
            error: None,
        }
    }

    /// The elements the heap arrays of the `row_count` rows of `column` hold in all,
    /// summed from their descriptors before any of them is decoded.
    fn element_total(column: VlaColumn<'a>, row_count: usize) -> Result<usize> {
        (0..row_count).try_fold(0usize, |total, row| {
            total
                .checked_add(column.cell(row)?.element_count)
                .ok_or(FitsError::DataUnitOverflow)
        })
    }

    /// The error of the row that ended the pass, if one did.
    fn finish(self) -> Result<()> {
        self.error.map_or(Ok(()), Err)
    }
}

impl<'a> Iterator for VlaCells<'_, 'a> {
    type Item = Cell<'a>;

    fn next(&mut self) -> Option<Cell<'a>> {
        if self.error.is_some() {
            return None;
        }
        let row = self.rows.next()?;
        match self.column.cell(row) {
            Ok(cell) => {
                self.end += cell.element_count;
                self.ends.push(self.end);
                Some(Cell {
                    bytes: cell.bytes,
                    count: cell.element_count,
                })
            }
            Err(error) => {
                self.error = Some(error);
                None
            }
        }
    }
}

#[cfg(test)]
mod tests;
