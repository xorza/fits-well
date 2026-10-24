//! One binary-table column descriptor and the decode its `TFORMn` selects.

use crate::bintable::column_data::ColumnData;
use crate::bintable::element_kind::Cell;
use crate::bintable::tdim;
use crate::bintable::tform::{Storage, Tform};
use crate::column::{Keyed, Named};
use crate::error::Result;
use crate::header_model::keywords;
use crate::image_model::scaling::Scaling;
use std::iter;

/// One column of a binary table: its `TFORMn` format, optional name/unit, the
/// `TSCALn`/`TZEROn`/`TNULLn`/`TDIMn`/`TDISPn` keywords, and its byte offset within a
/// row. A keyword whose value did not read is kept as its defect: its accessor and
/// each read that needs it fail, and the column's other reads do not.
#[derive(Debug)]
pub struct Column {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub tform: Tform,
    pub(crate) tscale: Keyed<f64>,
    pub(crate) tzero: Keyed<f64>,
    pub(crate) tnull: Keyed<Option<i64>>,
    pub(crate) tdim: Keyed<Option<Vec<usize>>>,
    pub(crate) tdisp: Keyed<Option<String>>,
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
            name: self.name.clone(),
            unit: self.unit.clone(),
            tform: self.tform,
            tscale: self.tscale.clone(),
            tzero: self.tzero.clone(),
            tnull: self.tnull.clone(),
            tdim: self.tdim.clone(),
            tdisp: self.tdisp.clone(),
            number: self.number,
            byte_offset: self.byte_offset,
        }
    }

    fn clone_from(&mut self, source: &Self) {
        let Self {
            name,
            unit,
            tform,
            tscale,
            tzero,
            tnull,
            tdim,
            tdisp,
            number,
            byte_offset,
        } = source;
        self.name.clone_from(name);
        self.unit.clone_from(unit);
        self.tform = *tform;
        self.tscale.clone_from(tscale);
        self.tzero.clone_from(tzero);
        self.tnull.clone_from(tnull);
        self.tdim.clone_from(tdim);
        self.tdisp.clone_from(tdisp);
        self.number = *number;
        self.byte_offset = *byte_offset;
    }
}

impl Named for Column {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Column {
    /// Byte offset of this column from the start of a row.
    pub const fn byte_offset(&self) -> usize {
        self.byte_offset
    }

    /// `TSCALn`, 1 when absent; applied by
    /// [`ColumnReader::physical`](crate::table::ColumnReader::physical).
    pub fn tscale(&self) -> Result<f64> {
        self.tscale.clone().map_err(|defect| defect.error())
    }

    /// `TZEROn`, 0 when absent; applied by
    /// [`ColumnReader::physical`](crate::table::ColumnReader::physical).
    pub fn tzero(&self) -> Result<f64> {
        self.tzero.clone().map_err(|defect| defect.error())
    }

    /// `TNULLn`, the integer value denoting an undefined element, if declared.
    pub fn tnull(&self) -> Result<Option<i64>> {
        self.tnull.clone().map_err(|defect| defect.error())
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
        match &self.tdisp {
            Ok(tdisp) => Ok(tdisp.as_deref()),
            Err(defect) => Err(defect.error()),
        }
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
    ) -> ColumnData {
        let mut out = ColumnData::U8(Vec::new());
        self.decode_cells_into(cells, row_count, &mut out);
        out
    }

    /// [`Column::decode_cells`] into `out`, whose buffer it reuses when `out` already
    /// holds values of this column's type.
    pub(super) fn decode_cells_into<'a>(
        &self,
        cells: impl ExactSizeIterator<Item = &'a [u8]>,
        row_count: usize,
        out: &mut ColumnData,
    ) {
        debug_assert_eq!(self.tform.storage(), Storage::Fixed, "a fixed-width column");
        let count = self.tform.repeat();
        self.tform.element().decode_cells_into(
            cells.map(|bytes| Cell { bytes, count }),
            row_count * count,
            out,
        );
    }

    /// Decode one fixed-width cell of this column — the single-row form of
    /// [`Column::decode_cells`].
    pub(crate) fn decode_cell(&self, bytes: &[u8]) -> ColumnData {
        debug_assert_eq!(bytes.len(), self.tform.byte_width());
        self.decode_cells(iter::once(bytes), 1)
    }

    /// Decode one `P`/`Q` row's heap array of `count` elements, whose bytes the row's
    /// descriptor gave.
    pub(crate) fn decode_vla_cell(&self, bytes: &[u8], count: usize) -> Result<ColumnData> {
        self.validate_vla_tdim(count)?;
        let element = self.tform.element();
        debug_assert_eq!(Some(bytes.len()), element.payload_len(count));
        Ok(element.decode_cells(iter::once(Cell { bytes, count }), count))
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
