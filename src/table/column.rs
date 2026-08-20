//! One binary-table column descriptor and the decode its `TFORMn` selects.

use crate::column::Named;
use crate::error::Result;
use crate::table::CharacterField;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::descriptor;
use crate::table_impl::tdim;
use crate::table_impl::tform::Tform;
use crate::table_impl::tform_kind::TformKind;

/// One column of a binary table: its `TFORMn` format, optional name/unit, the
/// `TSCALn`/`TZEROn`/`TNULLn` metadata, and its byte offset within a row.
#[derive(Debug, Clone)]
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
    /// Byte offset of this column from the start of a row.
    pub byte_offset: usize,
}

impl Named for Column {
    fn name(&self) -> Option<&str> {
        self.name.as_deref()
    }
}

impl Column {
    /// Decode `row_count` fixed-width cells of this column into a typed,
    /// row-flattened [`ColumnData`].
    pub(super) fn decode_cells<'a>(
        &self,
        cells: impl ExactSizeIterator<Item = &'a [u8]>,
        row_count: usize,
    ) -> ColumnData {
        match self.tform.kind {
            // One exact field per row, padding and all — the row *is* the field.
            TformKind::Char => ColumnData::Character(
                cells
                    .map(|cell| CharacterField::new(cell.to_vec()))
                    .collect(),
            ),
            TformKind::ArrayDesc32 | TformKind::ArrayDesc64 => {
                unreachable!("raw rejects VLA columns before fixed decode")
            }
            // `X` packs `repeat` bits into `byte_width` bytes, so its element count is
            // the byte width rather than the repeat.
            kind @ (TformKind::Byte | TformKind::Bit) => {
                kind.decode_cells(cells, row_count * self.tform.byte_width())
            }
            kind => kind.decode_cells(cells, row_count * self.tform.repeat),
        }
    }

    /// Decode one fixed-width cell of this column — the single-row form of
    /// [`Column::decode_cells`].
    pub(crate) fn decode_cell(&self, bytes: &[u8]) -> ColumnData {
        debug_assert_eq!(bytes.len(), self.tform.byte_width());
        self.decode_cells(std::iter::once(bytes), 1)
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
        Ok(element_type.decode_run(bytes))
    }

    /// The `TDIMn` extent check for this column's `P`/`Q` heap array.
    pub(super) fn validate_vla_tdim(&self, element_count: usize) -> Result<()> {
        match &self.tdim {
            Some(dims) => tdim::validate_vla_extent(dims, element_count),
            None => Ok(()),
        }
    }
}
