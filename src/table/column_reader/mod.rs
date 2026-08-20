//! A decode handle bound to one column of a binary table.

use num_complex::Complex;

use crate::data::unsigned_data::UnsignedData;
use crate::error::FitsError;
use crate::error::Result;
use crate::table_impl::BinTable;
use crate::table_impl::bit_column::BitColumn;
use crate::table_impl::column::Column;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::tform_kind::TformKind;
use crate::table_impl::vla_column::VlaCell;
use crate::table_impl::vla_column::VlaColumn;

/// A handle to one column of a [`BinTable`], from [`BinTable::column_by_idx`] or
/// [`BinTable::column_by_name`]. Decode through it without re-passing the column
/// descriptor: [`raw`](Self::raw) for the typed values, [`physical`](Self::physical)
/// for the scaled `f64` plane, [`unsigned`](Self::unsigned)/[`complex`](Self::complex)/
/// [`bits`](Self::bits) for the special kinds, and [`vla`](Self::vla) (+
/// [`vla_physical`](Self::vla_physical)/[`vla_unsigned`](Self::vla_unsigned)/
/// [`vla_complex`](Self::vla_complex)/[`vla_bits`](Self::vla_bits)) for variable-length
/// `P`/`Q` columns. Borrows the table, so it cannot outlive it.
#[derive(Debug, Clone, Copy)]
pub struct ColumnReader<'a> {
    table: &'a BinTable,
    index: usize,
}

impl<'a> ColumnReader<'a> {
    pub(super) fn new(table: &'a BinTable, index: usize) -> ColumnReader<'a> {
        ColumnReader { table, index }
    }

    /// The column's [`Column`] descriptor — name, `TFORMn`, `TSCALn`/`TZEROn`/`TNULLn`,
    /// `TDIMn`, `TDISPn`.
    pub fn descriptor(&self) -> &'a Column {
        &self.table.schema.columns[self.index]
    }

    /// The column descriptor, rejecting a variable-length (`P`/`Q`) column — the
    /// shared precondition of every fixed-width decode. Those columns carry heap
    /// descriptors rather than values, so they go through [`ColumnReader::vla`].
    fn fixed_descriptor(&self) -> Result<&'a Column> {
        let col = self.descriptor();
        if col.tform.kind.is_descriptor() {
            return Err(FitsError::VariableLengthColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(col)
    }

    /// Decode a fixed-width column into a typed, row-flattened [`ColumnData`]: `A` is
    /// one exact [`CharacterField`](crate::table::CharacterField)
    /// per row, every other fixed kind decodes from the concatenated cell bytes.
    /// Variable-length (`P`/`Q`) columns error here — use [`ColumnReader::vla`].
    pub fn raw(&self) -> Result<ColumnData> {
        let col = self.fixed_descriptor()?;
        Ok(col.decode_cells(self.table.cells(col), self.table.schema.nrows))
    }

    /// The numeric column scaled to its physical `f64` plane: `TZEROn + TSCALn × raw`,
    /// mapping integers equal to `TNULLn` to `NaN`. Errors for the non-numeric kinds
    /// (`A`/`L`/`X`/`C`/`M`) and variable-length columns.
    pub fn physical(&self) -> Result<Vec<f64>> {
        let col = self.fixed_descriptor()?;
        col.tform.kind.decode_physical(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            col.tscale,
            col.tzero,
            col.tnull,
        )
    }

    /// Exact typed integers when the column uses the FITS unsigned (or signed-byte)
    /// convention — `TSCALn == 1`, no `TNULLn`, `TZEROn` the matching sign-bit offset
    /// on a `B`/`I`/`J`/`K` column — without the `f64` rounding of
    /// [`physical`](Self::physical). `Ok(None)` for any other column; errors only for a
    /// variable-length column. Mirrors [`crate::image::Image::unsigned`].
    pub fn unsigned(&self) -> Result<Option<UnsignedData>> {
        let col = self.fixed_descriptor()?;
        let Some(kind) = col
            .tform
            .kind
            .unsigned_kind(col.tscale, col.tzero, col.tnull)
        else {
            return Ok(None);
        };
        Ok(Some(UnsignedData::from_be_cells(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            kind,
        )))
    }

    /// A `C`/`M` complex column as [`Complex<f64>`] values, applying `TSCALn` to both
    /// components and `TZEROn` to the real component (§7.3.2). Errors on non-complex columns.
    pub fn complex(&self) -> Result<Vec<Complex<f64>>> {
        let col = self.descriptor();
        col.tform.kind.decode_complex(
            self.table.cells(col),
            self.table.schema.nrows * col.tform.repeat,
            col.tscale,
            col.tzero,
        )
    }

    /// An `X` (bit-array) column as a borrowed 2-D [`BitColumn`] — `nrows × repeat`
    /// bits viewed in place over the data unit, MSB-first (bit 0 is the MSB of the
    /// first byte, §7.3.2), with no per-row allocation. Errors on any non-`X` column.
    pub fn bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        if col.tform.kind != TformKind::Bit {
            return Err(FitsError::NotABitColumn {
                code: col.tform.kind.code(),
            });
        }
        Ok(BitColumn::new(self.table, self.index))
    }

    /// Decode a variable-length (`P`/`Q`) column: one [`ColumnData`] per row, each
    /// holding that row's heap array (which may be empty). Errors for fixed-width
    /// columns.
    pub fn vla(&self) -> Result<Vec<ColumnData>> {
        let column = self.vla_column()?;
        self.map_vla_rows(column, |cell| Ok(cell.element_type.decode_run(cell.bytes)))
    }

    /// Scale each row of a `P`/`Q` column to its physical plane: `TZEROn + TSCALn ×
    /// element`, mapping integers equal to `TNULLn` to `NaN` (§6.4 — scaling applies to
    /// the heap values). Errors for fixed-width or non-numeric-heap columns.
    pub fn vla_physical(&self) -> Result<Vec<Vec<f64>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        self.map_vla_rows(column, |cell| {
            cell.element_type.decode_physical(
                std::iter::once(cell.bytes),
                cell.element_count,
                col.tscale,
                col.tzero,
                col.tnull,
            )
        })
    }

    /// Exact typed integers for each row of a `P`/`Q` heap array using the FITS
    /// unsigned (or signed-byte) convention. The outer vector is table rows; each
    /// row's [`UnsignedData`] owns its jagged array. Returns `Ok(None)` unless the
    /// heap type and `TSCALn`/`TZEROn`/`TNULLn` metadata form that convention.
    /// Errors for fixed-width columns.
    pub fn vla_unsigned(&self) -> Result<Option<Vec<UnsignedData>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        let Some(kind) = column
            .element_type()
            .unsigned_kind(col.tscale, col.tzero, col.tnull)
        else {
            return Ok(None);
        };
        self.map_vla_rows(column, |cell| {
            Ok(UnsignedData::from_be_cells(
                std::iter::once(cell.bytes),
                cell.element_count,
                kind,
            ))
        })
        .map(Some)
    }

    /// Each row of a `PC`/`QC`/`PM`/`QM` heap array as [`Complex<f64>`], applying
    /// `TSCALn` to both components and `TZEROn` to only the real component (§7.3.2).
    /// Empty descriptors produce empty rows. Errors for fixed-width or non-complex
    /// heap columns.
    pub fn vla_complex(&self) -> Result<Vec<Vec<Complex<f64>>>> {
        let col = self.descriptor();
        let column = self.vla_column()?;
        let kind = column.element_type();
        if !matches!(kind, TformKind::ComplexF32 | TformKind::ComplexF64) {
            return Err(FitsError::NotAComplexColumn { code: kind.code() });
        }
        self.map_vla_rows(column, |cell| {
            kind.decode_complex(
                std::iter::once(cell.bytes),
                cell.element_count,
                col.tscale,
                col.tzero,
            )
        })
    }

    /// A variable-length `X` (`1PX`/`1QX`) column as a borrowed 2-D [`BitColumn`],
    /// MSB-first (§7.3.2/§7.3.5 — the descriptor's element count is the bit count). The
    /// rows are *jagged* (each its own length), so [`BitColumn::row`]`(r).len()` gives a
    /// row's width. Errors on any non-bit VLA.
    pub fn vla_bits(&self) -> Result<BitColumn<'a>> {
        let col = self.descriptor();
        match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32, Some(TformKind::Bit))
            | (TformKind::ArrayDesc64, Some(TformKind::Bit)) => {}
            _ => {
                return Err(FitsError::NotABitColumn {
                    code: col.tform.kind.code(),
                });
            }
        };
        // Validate every row's heap span up front (no allocation) so [`BitColumn::row`]
        // can resolve a row lazily and infallibly — the only place an overrun surfaces.
        for r in 0..self.table.schema.nrows {
            let descriptor = self.table.pq_descriptor(col, r)?;
            col.validate_vla_tdim(descriptor.count)?;
            self.table.pq_payload(descriptor, TformKind::Bit)?;
        }
        Ok(BitColumn::new(self.table, self.index))
    }

    fn vla_element_type(&self) -> Result<TformKind> {
        let col = self.descriptor();
        match (col.tform.kind, col.tform.vla_elem) {
            (TformKind::ArrayDesc32 | TformKind::ArrayDesc64, Some(element_type)) => {
                Ok(element_type)
            }
            _ => Err(FitsError::NotAVla {
                code: col.tform.kind.code(),
            }),
        }
    }

    pub(crate) fn vla_column(&self) -> Result<VlaColumn<'a>> {
        Ok(VlaColumn::new(
            self.table,
            self.index,
            self.vla_element_type()?,
        ))
    }

    fn map_vla_rows<T>(
        &self,
        column: VlaColumn<'a>,
        decode: impl Fn(VlaCell<'a>) -> Result<T>,
    ) -> Result<Vec<T>> {
        let mut rows = Vec::with_capacity(self.table.schema.nrows);
        for row in 0..self.table.schema.nrows {
            rows.push(decode(column.cell(row)?)?);
        }
        Ok(rows)
    }
}

#[cfg(test)]
mod tests;
