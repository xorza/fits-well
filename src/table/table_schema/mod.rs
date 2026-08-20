//! A binary table's structure as the header alone determines it.

use crate::column;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::column::Column;
use crate::table_impl::tdim;
use crate::table_impl::tform::Tform;

/// Binary-table schema parsed entirely from the header, without reading the data
/// unit. Byte offsets are relative to the start of the data unit.
#[derive(Debug, Clone)]
pub struct TableSchema {
    pub nrows: usize,
    /// Byte width of one row (`NAXIS1`).
    pub row_len: usize,
    pub heap_offset: usize,
    pub heap_end: usize,
    pub columns: Vec<Column>,
}

impl TableSchema {
    /// Parse the complete binary-table schema without touching its data unit.
    pub fn parse(header: &Header) -> Result<TableSchema> {
        let row_len = header.required_usize("NAXIS1", "NAXIS1")?;
        let nrows = header.required_usize("NAXIS2", "NAXIS2")?;
        // §7.3.1: `0 ≤ TFIELDS ≤ 999` — also a guard, since `tfields` sizes the
        // column `Vec` and drives the `TFORMn` loop (an absurd value would abort).
        let tfields = header.required_usize("TFIELDS", "TFIELDS")?;
        validate_table_field_count(tfields)?;

        let mut columns = Vec::with_capacity(tfields);
        let mut offset = 0;
        for n in 1..=tfields {
            let tform_value = header.required_text(key!("TFORM{n}").as_str(), "TFORMn")?;
            let tform = Tform::parse(tform_value)?;
            let shape = header
                .get_text(key!("TDIM{n}").as_str())?
                .map(tdim::parse)
                .transpose()?;
            // A fixed column's cell holds exactly `repeat` elements; a `P`/`Q` cell's
            // count is per-row and is checked as each row's descriptor is read.
            if let Some(dims) = &shape
                && !tform.kind.is_descriptor()
            {
                tdim::validate_extent(dims, tform.repeat)?;
            }
            columns.push(Column {
                name: header
                    .get_text(key!("TTYPE{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                unit: header
                    .get_text(key!("TUNIT{n}").as_str())?
                    .map(str::to_string)
                    .filter(|s| !s.is_empty()),
                tform,
                tscale: header.get_real(key!("TSCAL{n}").as_str())?.unwrap_or(1.0),
                tzero: header.get_real(key!("TZERO{n}").as_str())?.unwrap_or(0.0),
                tnull: header.get_integer(key!("TNULL{n}").as_str())?,
                tdim: shape,
                tdisp: header
                    .get_text(key!("TDISP{n}").as_str())?
                    .map(str::to_string),
                byte_offset: offset,
            });
            offset = offset.saturating_add(tform.byte_width());
        }
        if offset != row_len {
            return Err(FitsError::RowWidthMismatch {
                computed: offset,
                declared: row_len,
            });
        }

        // `nrows · row_len` from untrusted axes: check once (guards a 32-bit-usize
        // overflow that `data_extent`'s u64 math wouldn't catch) and reuse.
        let main_table = nrows.checked_mul(row_len).ok_or(FitsError::UnexpectedEof)?;
        let heap_offset = header.optional_usize("THEAP", "THEAP", main_table)?;
        // §6.6: the heap follows the main table, so THEAP must be ≥ its size.
        if heap_offset < main_table {
            return Err(FitsError::KeywordOutOfRange { name: "THEAP" });
        }
        // PCOUNT counts the gap-plus-heap bytes after the main table, so the real
        // heap ends here — anything past it is block fill (§6.6).
        let pcount = header.optional_usize("PCOUNT", "PCOUNT", 0)?;
        let heap_end = main_table
            .checked_add(pcount)
            .ok_or(FitsError::UnexpectedEof)?;
        if heap_offset > heap_end {
            return Err(FitsError::KeywordOutOfRange { name: "THEAP" });
        }
        Ok(TableSchema {
            nrows,
            row_len,
            heap_offset,
            heap_end,
            columns,
        })
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7. Resolvable from the header alone, so a caller
    /// holding only a [`crate::FitsReader::table_schema`] can locate a column
    /// without reading the data unit.
    pub fn column_index(&self, name: &str) -> Option<usize> {
        column::index_of(&self.columns, name)
    }

    /// [`TableSchema::column_index`], reporting an absent column as
    /// [`FitsError::ColumnNotFound`].
    pub(crate) fn column_index_checked(&self, name: &str) -> Result<usize> {
        column::checked_index_of(&self.columns, name)
    }

    /// Bounds-check a zero-based column index against this schema's column count.
    pub(super) fn validate_column_index(&self, index: usize) -> Result<()> {
        column::validate_index(index, self.columns.len())
    }
}

#[cfg(test)]
mod tests;
