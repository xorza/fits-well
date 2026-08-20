//! A variable-length (`P`/`Q`) column's per-row heap arrays.

use crate::error::FitsError;
use crate::error::Result;
use crate::table_impl::BinTable;
use crate::table_impl::tform_kind::TformKind;

/// A handle to one `P`/`Q` column of a [`BinTable`], resolving a row's heap array on
/// demand. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaColumn<'a> {
    table: &'a BinTable,
    index: usize,
    element_type: TformKind,
}

/// One row's heap array: the bytes the row's descriptor addresses, the element count
/// it declared, and the heap element type from the column's `rPt` format.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaCell<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) element_count: usize,
    pub(crate) element_type: TformKind,
}

impl<'a> VlaColumn<'a> {
    pub(super) fn new(table: &'a BinTable, index: usize, element_type: TformKind) -> VlaColumn<'a> {
        VlaColumn {
            table,
            index,
            element_type,
        }
    }

    pub(crate) fn element_type(&self) -> TformKind {
        self.element_type
    }

    pub(crate) fn cell(&self, row: usize) -> Result<VlaCell<'a>> {
        if row >= self.table.schema.nrows {
            return Err(FitsError::UnexpectedEof);
        }
        let col = &self.table.schema.columns[self.index];
        let descriptor = self.table.pq_descriptor(col, row)?;
        col.validate_vla_tdim(descriptor.count)?;
        let bytes = self.table.pq_payload(descriptor, self.element_type)?;
        Ok(VlaCell {
            bytes,
            element_count: descriptor.count,
            element_type: self.element_type,
        })
    }
}
