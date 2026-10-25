//! A variable-length (`P`/`Q`) column's per-row heap arrays.

use crate::bintable::element_kind::ElementKind;
use crate::bintable::table_view::TableView;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// A handle to one `P`/`Q` column of a binary table, resolving a row's heap array on
/// demand. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaColumn<'a> {
    table: TableView<'a>,
    index: usize,
    element_type: ElementKind,
}

/// One row's heap array: the bytes the row's descriptor addresses, the element count
/// it declared, and the heap element type from the column's `rPt` format.
#[derive(Debug, Clone, Copy)]
pub(crate) struct VlaCell<'a> {
    pub(crate) bytes: &'a [u8],
    pub(crate) element_count: usize,
    #[cfg(feature = "compression")]
    pub(crate) element_type: ElementKind,
}

impl<'a> VlaColumn<'a> {
    pub(crate) const fn new(table: TableView<'a>, index: usize, element_type: ElementKind) -> Self {
        VlaColumn {
            table,
            index,
            element_type,
        }
    }

    pub(crate) const fn element_type(&self) -> ElementKind {
        self.element_type
    }

    pub(crate) fn cell(&self, row: usize) -> Result<VlaCell<'a>> {
        debug_assert!(row < self.table.extent.row_count, "a row of the table");
        let col = &self.table.columns[self.index];
        let descriptor = self.table.pq_descriptor(col, row)?;
        col.validate_vla_tdim(descriptor.count)?;
        let bytes = self.table.pq_payload(descriptor, self.element_type).ok_or(
            FitsError::DescriptorPastHeap {
                column: self.index,
                row,
            },
        )?;
        Ok(VlaCell {
            bytes,
            element_count: descriptor.count,
            #[cfg(feature = "compression")]
            element_type: self.element_type,
        })
    }
}
