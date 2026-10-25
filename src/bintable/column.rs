//! One binary-table column descriptor and the decode its `TFORMn` selects.

use crate::bintable::column_data::ColumnData;
use crate::bintable::element_kind::Cell;
use crate::bintable::tdim;
use crate::bintable::tform::{Storage, Tform};
use crate::column::{ColumnKeywords, Keyed, Named};
use crate::fits_error::Result;
use crate::header_model::keywords;
use crate::image_model::scaling::Scaling;
use std::iter;

/// One column of a binary table: its `TFORMn` format, the keywords both table forms read
/// (name, unit, `TSCALn`, `TZEROn`, `TNULLn`, `TDISPn`), its `TDIMn`, and its byte offset
/// within a row. A keyword whose value did not read is kept as its defect: its accessor and
/// each read that needs it fail, and the column's other reads do not.
#[derive(Debug)]
pub struct Column {
    pub tform: Tform,
    pub(crate) keywords: ColumnKeywords<i64>,
    pub(crate) tdim: Keyed<Option<Vec<usize>>>,
    /// The column's 1-based number, the `n` of its keywords.
    pub(crate) number: usize,
    /// Byte offset of this column from the start of a row: the sum of the widths
    /// before it, so derived and not public.
    pub(crate) byte_offset: usize,
}

/// `clone_from` reuses the strings and shape of `self`, so a selection refilled with a
/// table's columns makes no allocation.
impl Clone for Column {
    fn clone(&self) -> Self {
        Self {
            tform: self.tform,
            keywords: self.keywords.clone(),
            tdim: self.tdim.clone(),
            number: self.number,
            byte_offset: self.byte_offset,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        let Self {
            tform,
            keywords,
            tdim,
            number,
            byte_offset,
        } = source;
        self.tform = *tform;
        self.keywords.clone_from(keywords);
        self.tdim.clone_from(tdim);
        self.number = *number;
        self.byte_offset = *byte_offset;
    }
}

impl Named for Column {
    fn name(&self) -> Result<Option<&str>> {
        self.keywords.name()
    }
}

impl Column {
    /// Byte offset of this column from the start of a row.
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    /// The `TTYPEn` name, `None` when absent or empty.
    pub fn name(&self) -> Result<Option<&str>> {
        self.keywords.name()
    }

    /// The `TUNITn` unit, `None` when absent or empty.
    pub fn unit(&self) -> Result<Option<&str>> {
        self.keywords.unit()
    }

    /// `TSCALn`, 1 when absent; applied by
    /// [`ColumnReader::physical_into`](crate::table::ColumnReader::physical_into).
    pub fn tscale(&self) -> Result<f64> {
        self.keywords.tscale()
    }

    /// `TZEROn`, 0 when absent; applied by
    /// [`ColumnReader::physical_into`](crate::table::ColumnReader::physical_into).
    pub fn tzero(&self) -> Result<f64> {
        self.keywords.tzero()
    }

    /// `TNULLn`, the integer value denoting an undefined element, if declared.
    pub fn tnull(&self) -> Result<Option<i64>> {
        Ok(self.keywords.tnull()?.copied())
    }

    /// The `TDIMn` array shape (e.g. `'(4,4)'` → `[4, 4]`), if declared — reshapes the
    /// `repeat` elements of each row into a multidimensional array (§7.3.2).
    pub fn tdim(&self) -> Result<Option<&[usize]>> {
        match &self.tdim {
            Ok(shape) => Ok(shape.as_deref()),
            Err(defect) => Err(defect.error()),
        }
    }

    /// The raw `TDISPn` display recommendation (§7.3.4), if declared.
    pub fn tdisp(&self) -> Result<Option<&str>> {
        self.keywords.tdisp()
    }

    /// `TSCALn`/`TZEROn`/`TNULLn` as the [`Scaling`] they mean — exactly what an
    /// image's `BSCALE`/`BZERO`/`BLANK` do.
    pub(super) fn scaling(&self) -> Result<Scaling> {
        Ok(Scaling {
            bscale: self.tscale()?,
            bzero: self.tzero()?,
            blank: self.tnull()?,
        })
    }

    /// Decode `row_count` fixed-width cells of this column into a typed,
    /// row-flattened [`ColumnData`].
    pub(super) fn decode_cells<'a>(
        &self,
        cells: impl ExactSizeIterator<Item = &'a [u8]>,
        row_count: usize,
    ) -> Result<ColumnData> {
        let mut out = ColumnData::U8(Vec::new());
        self.decode_cells_into(cells, row_count, &mut out)?;
        Ok(out)
    }

    /// [`Column::decode_cells`] into `out`, whose buffer it reuses when `out` already
    /// holds values of this column's type.
    pub(super) fn decode_cells_into<'a>(
        &self,
        cells: impl ExactSizeIterator<Item = &'a [u8]>,
        row_count: usize,
        out: &mut ColumnData,
    ) -> Result<()> {
        debug_assert_eq!(self.tform.storage(), Storage::Fixed, "a fixed-width column");
        let count = self.tform.repeat();
        self.tform.element().decode_cells_into(
            cells.map(|bytes| Cell { bytes, count }),
            row_count * count,
            out,
        )
    }

    /// Decode one fixed-width cell of this column — the single-row form of
    /// [`Column::decode_cells`].
    pub(crate) fn decode_cell(&self, bytes: &[u8]) -> Result<ColumnData> {
        debug_assert_eq!(bytes.len(), self.tform.byte_width());
        self.decode_cells(iter::once(bytes), 1)
    }

    /// Decode one `P`/`Q` row's heap array of `count` elements, whose bytes the row's
    /// descriptor gave.
    pub(crate) fn decode_vla_cell(&self, bytes: &[u8], count: usize) -> Result<ColumnData> {
        self.validate_vla_tdim(count)?;
        let element = self.tform.element();
        debug_assert_eq!(Some(bytes.len()), element.payload_len(count));
        element.decode_cells(iter::once(Cell { bytes, count }), count)
    }

    /// The `TDIMn` extent check for this column's `P`/`Q` heap array.
    pub(super) fn validate_vla_tdim(&self, element_count: usize) -> Result<()> {
        match self.tdim()? {
            Some(dims) => {
                tdim::validate_vla_extent(dims, element_count, keywords::TDIM.indexed(self.number)?)
            }
            None => Ok(()),
        }
    }
}
