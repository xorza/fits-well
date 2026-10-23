//! One binary-table column descriptor and the decode its `TFORMn` selects.

use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor;
use crate::bintable::tdim;
use crate::bintable::tform::Tform;
use crate::bintable::tform_kind::TformKind;
use crate::column::Named;
use crate::error::Result;
use crate::header_model::keywords;
use crate::image_model::scaling::Scaling;
use std::iter;

/// One column of a binary table: its `TFORMn` format, optional name/unit, the
/// `TSCALn`/`TZEROn`/`TNULLn` metadata, and its byte offset within a row.
#[derive(Debug)]
pub struct Column {
    pub name: Option<String>,
    pub unit: Option<String>,
    pub tform: Tform,
    /// `TSCALn` (default 1.0); applied by
    /// [`ColumnReader::physical`](crate::table::ColumnReader::physical).
    pub tscale: f64,
    /// `TZEROn` (default 0.0); applied by
    /// [`ColumnReader::physical`](crate::table::ColumnReader::physical).
    pub tzero: f64,
    /// `TNULLn`, the integer value denoting an undefined element, if declared.
    pub tnull: Option<i64>,
    /// `TDIMn` array shape (e.g. `'(4,4)'` → `[4, 4]`), if declared — reshapes the
    /// `repeat` elements of each row into a multidimensional array (§7.3.2).
    pub tdim: Option<Vec<usize>>,
    /// Raw `TDISPn` display recommendation (§7.3.4), if declared.
    pub tdisp: Option<String>,
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
            tscale: self.tscale,
            tzero: self.tzero,
            tnull: self.tnull,
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
        self.tscale = *tscale;
        self.tzero = *tzero;
        self.tnull = *tnull;
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

    /// `TSCALn`/`TZEROn`/`TNULLn` as the [`Scaling`] they mean — exactly what an
    /// image's `BSCALE`/`BZERO`/`BLANK` do.
    pub(super) const fn scaling(&self) -> Scaling {
        Scaling {
            bscale: self.tscale,
            bzero: self.tzero,
            blank: self.tnull,
        }
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
        match self.tform.kind {
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
                unreachable!("raw rejects VLA columns before fixed decode")
            }
            // `X` packs `repeat` bits into `byte_width` bytes, so its element count is
            // the byte width rather than the repeat.
            kind @ (TformKind::U8 | TformKind::Bit) => {
                kind.decode_cells_into(cells, row_count * self.tform.byte_width(), out);
            }
            kind => kind.decode_cells_into(cells, row_count * self.tform.repeat, out),
        }
    }

    /// Decode one fixed-width cell of this column — the single-row form of
    /// [`Column::decode_cells`].
    pub(crate) fn decode_cell(&self, bytes: &[u8]) -> ColumnData {
        debug_assert_eq!(bytes.len(), self.tform.byte_width());
        self.decode_cells(iter::once(bytes), 1)
    }

    /// Decode one `P`/`Q` row's heap array, whose length the row's descriptor gave.
    pub(crate) fn decode_vla_cell(&self, bytes: &[u8], element_count: usize) -> Result<ColumnData> {
        self.validate_vla_tdim(element_count)?;
        let element_type = self
            .tform
            .vla_elem
            .expect("validated VLA format carries an element type");
        let expected_len = descriptor::payload_len(element_type, element_count)?;
        debug_assert_eq!(bytes.len(), expected_len);
        let kind = element_type.heap_kind();
        Ok(kind.decode_cells(iter::once(bytes), bytes.len() / kind.elem_size()))
    }

    /// The `TDIMn` extent check for this column's `P`/`Q` heap array.
    pub(super) fn validate_vla_tdim(&self, element_count: usize) -> Result<()> {
        match &self.tdim {
            Some(dims) => {
                tdim::validate_vla_extent(dims, element_count, keywords::TDIM.indexed(self.number)?)
            }
            None => Ok(()),
        }
    }
}
