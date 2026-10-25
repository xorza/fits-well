//! A binary table's structure as the header alone determines it.

use crate::bintable::column::Column;
use crate::bintable::table_extent::TableExtent;
use crate::bintable::tdim;
use crate::bintable::tform::{Storage, Tform};
use crate::column;
use crate::column::{ColumnKeywords, TableShape};
use crate::fits_error::FitsError;
use crate::fits_error::KeywordDefect;
use crate::fits_error::Result;
use crate::header_model::{Header, keywords};

/// Whether a schema reads the keywords that describe a column's values.
#[derive(Debug, Clone, Copy)]
enum ValueKeywords {
    Read,
    Skip,
}

/// Binary-table schema parsed entirely from the header, without reading the data
/// unit. Byte offsets are relative to the start of the data unit.
#[derive(Debug, Clone)]
pub struct Schema {
    pub(crate) extent: TableExtent,
    pub columns: Vec<Column>,
}

impl Schema {
    /// Parse the complete binary-table schema without touching its data unit.
    pub fn parse(header: &Header) -> Result<Self> {
        Self::parse_columns(header, ValueKeywords::Read)
    }

    /// The schema of a tiled-compressed table's `BINTABLE` container. Its `TDIMn`,
    /// `TSCALn`, `TZEROn` and `TNULLn` describe the uncompressed columns (§10.3.1), not
    /// the `1QB` cells that hold their compressed bytes, so they are not read.
    pub(crate) fn parse_container(header: &Header) -> Result<Self> {
        Self::parse_columns(header, ValueKeywords::Skip)
    }

    fn parse_columns(header: &Header, values: ValueKeywords) -> Result<Self> {
        let TableShape {
            row_len,
            row_count,
            field_count: tfields,
        } = TableShape::read(header)?;

        let mut columns = Vec::with_capacity(tfields);
        let mut offset = 0;
        for n in 1..=tfields {
            let tform_value = header.require_text(keywords::TFORM.indexed(n)?)?;
            let tform = tform_value.parse::<Tform>()?;
            let tdim_keyword = keywords::TDIM.indexed(n)?;
            let (tdim, column_keywords) = match values {
                ValueKeywords::Read => (
                    column::keyed(tdim_keyword, || {
                        let Some(value) = header.get_text(tdim_keyword)? else {
                            return Ok(None);
                        };
                        let shape = tdim::parse(value, tdim_keyword)?;
                        // A fixed column's cell holds exactly `repeat` elements; a `P`/`Q`
                        // cell's count is per-row and is checked as each row is read.
                        if tform.storage() == Storage::Fixed {
                            tdim::validate_extent(&shape, tform.repeat(), tdim_keyword)?;
                        }
                        Ok(Some(shape))
                    }),
                    ColumnKeywords::read(header, n, |tnull| header.get_integer(tnull))?,
                ),
                ValueKeywords::Skip => (Ok(None), ColumnKeywords::labels(header, n)?),
            };
            columns.push(Column {
                tform,
                keywords: column_keywords,
                tdim,
                number: n,
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

        // `row_count · row_len` from untrusted axes: check once (guards a 32-bit-usize
        // overflow that `HduKind::data_bytes`' u64 math wouldn't catch) and reuse.
        let main_table = row_count
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let heap_offset = header.optional_usize(keywords::THEAP, main_table)?;
        // §6.6: the heap follows the main table, so THEAP must be ≥ its size.
        if heap_offset < main_table {
            return Err(FitsError::InvalidKeywordValue {
                keyword: keywords::THEAP,
                defect: KeywordDefect::Structure,
            });
        }
        // PCOUNT counts the gap-plus-heap bytes after the main table, so the real
        // heap ends here — anything past it is block fill (§6.6).
        let pcount = header.optional_usize(keywords::PCOUNT, 0)?;
        let heap_end = main_table
            .checked_add(pcount)
            .ok_or(FitsError::DataUnitOverflow)?;
        if heap_offset > heap_end {
            return Err(FitsError::InvalidKeywordValue {
                keyword: keywords::THEAP,
                defect: KeywordDefect::Structure,
            });
        }
        Ok(Self {
            extent: TableExtent {
                row_count,
                row_len,
                heap_offset,
                heap_end,
            },
            columns,
        })
    }

    pub const fn row_count(&self) -> usize {
        self.extent.row_count
    }

    /// Byte width of one row (`NAXIS1`).
    pub const fn row_len(&self) -> usize {
        self.extent.row_len
    }

    /// The index of the first column whose `TTYPEn` matches `name`, compared
    /// case-insensitively per §6.7. Resolvable from the header alone, so a caller
    /// holding only a [`Hdu::table_schema`](crate::io::Hdu::table_schema) can locate a column
    /// without reading the data unit.
    pub fn column_index(&self, name: &str) -> Result<Option<usize>> {
        column::index_of(&self.columns, name)
    }

    /// [`Schema::column_index`], reporting an absent column as
    /// [`FitsError::ColumnNotFound`].
    pub(crate) fn column_index_checked(&self, name: &str) -> Result<usize> {
        column::checked_index_of(&self.columns, name)
    }

    /// This schema for a compacted selection of `row_count` rows, whose heap follows the
    /// rows directly and ends at `data_len`.
    pub(crate) fn compacted(&self, row_count: usize, data_len: usize) -> Self {
        Self {
            extent: self.extent.compacted(row_count, data_len),
            columns: self.columns.clone(),
        }
    }
}

#[cfg(test)]
mod tests;
