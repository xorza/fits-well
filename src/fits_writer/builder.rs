//! Binary-table writer schema, validation, and encoding.

use std::io::Write;
use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use num_complex::Complex;

use crate::bintable::character_field::CharacterField;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor::DescriptorWidth;
#[cfg(feature = "compression")]
use crate::bintable::descriptor::PqDescriptor;
use crate::bintable::tdim;
use crate::bintable::tform_kind::TformKind;
use crate::block::ZERO_FILL;
#[cfg(feature = "compression")]
use crate::compress::CompressionConfig;
#[cfg(feature = "compression")]
use crate::compress::table::{self, TableCells};
use crate::endian::{extend_be, validate_pq_descriptor, write_pq_descriptor};
use crate::error::{FitsError, Result};
#[cfg(feature = "compression")]
use crate::fits_writer::merge_header_template;
use crate::fits_writer::{
    FitsWriter, Opening, ROW_COUNT_FIXED, accept_row_count, validate_scaling,
};
use crate::hdu::validate_table_field_count;
use crate::header_model::card::validate_ascii;
use crate::header_model::{Header, keywords, value};
use crate::image_model::{U64_OFFSET, U64_OFFSET_INTEGER};
use crate::ragged::Ragged;

/// The element type of a binary-table writer column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ColumnType {
    Logical,
    Byte,
    I16,
    I32,
    I64,
    F32,
    F64,
    ComplexF32,
    ComplexF64,
    Character,
}

/// The mutually exclusive payload layouts of a binary-table writer column.
#[derive(Debug, Clone)]
enum WriteColumnData {
    Fixed {
        data: ColumnData,
        repeat: usize,
    },
    Vla {
        rows: Ragged<ColumnData>,
        width: DescriptorWidth,
    },
    VlaBits {
        rows: Ragged<BitVec<u8, Msb0>>,
        width: DescriptorWidth,
    },
    Bits {
        bytes: Vec<u8>,
        bit_count: usize,
    },
}

/// One column to write into a binary table.
#[derive(Debug, Clone)]
pub struct WriteColumn {
    name: String,
    unit: Option<String>,
    values: WriteColumnData,
    tdim: Option<Vec<usize>>,
    /// `TSCALn`/`TZEROn` to emit: `data` holds the stored values, and a reader's
    /// `ColumnReader::physical` recovers `TZEROn + TSCALn × stored`.
    tscale: Option<f64>,
    tzero: Option<f64>,
    /// `TNULLn`: the stored integer marking an undefined element.
    tnull: Option<i64>,
}

impl WriteColumn {
    /// A scalar column. Its row count is the payload length.
    pub fn scalar(name: impl Into<String>, data: ColumnData) -> Self {
        Self::fixed(name, data, 1)
    }

    /// A fixed-width column of `repeat` elements per row.
    ///
    /// This is the explicit-schema constructor for vector columns and empty
    /// templates. Prefer [`WriteColumn::scalar`] when every row has one value.
    pub fn fixed(name: impl Into<String>, data: ColumnData, repeat: usize) -> Self {
        Self {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Fixed { data, repeat },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// An `A` column of `width`-byte fields, one per row. A shorter field is padded
    /// with spaces; a longer one is an error.
    pub fn characters(
        name: impl Into<String>,
        fields: impl IntoIterator<Item = impl AsRef<[u8]>>,
        width: usize,
    ) -> Result<Self> {
        let name = name.into();
        let mut bytes = Vec::new();
        for (row, field) in fields.into_iter().enumerate() {
            let field = field.as_ref();
            if field.len() > width {
                return Err(FitsError::CharacterFieldTooWide {
                    column: name,
                    row,
                    length: field.len(),
                    width,
                });
            }
            bytes.extend_from_slice(field);
            bytes.resize(bytes.len() + width - field.len(), b' ');
        }
        Ok(Self::fixed(name, ColumnData::Character(bytes), width))
    }

    /// A variable-length `P` column: row `r` stores `rows.range(r)` of the values,
    /// and the values' type is the heap element type. [`WriteColumn::wide`] changes
    /// the descriptors to `Q`.
    pub fn vla(name: impl Into<String>, rows: Ragged<ColumnData>) -> Self {
        Self {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Vla {
                rows,
                width: DescriptorWidth::P,
            },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// An `X` column of `bit_count` bits per row and its row-packed bytes.
    pub fn bits(name: impl Into<String>, bytes: Vec<u8>, bit_count: usize) -> Self {
        Self {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Bits { bytes, bit_count },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// A variable-length `PX` bit-array column: row `r` stores `rows.row(r)`,
    /// MSB-first, and its bit length is the descriptor element count.
    /// [`WriteColumn::wide`] changes the descriptors to `QX`.
    pub fn vla_bits(name: impl Into<String>, rows: Ragged<BitVec<u8, Msb0>>) -> Self {
        Self {
            name: name.into(),
            unit: None,
            values: WriteColumnData::VlaBits {
                rows,
                width: DescriptorWidth::P,
            },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// Attach a unit (`TUNITn`).
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Attach a `TDIMn` array shape (fastest axis first).
    #[must_use]
    pub fn with_tdim(mut self, shape: Vec<usize>) -> Self {
        self.tdim = Some(shape);
        self
    }

    /// Use 64-bit `Q` descriptors for this VLA column.
    ///
    /// # Panics
    /// Unless the column is variable-length — from [`WriteColumn::vla`] or
    /// [`WriteColumn::vla_bits`].
    #[must_use]
    pub fn wide(mut self) -> Self {
        match &mut self.values {
            WriteColumnData::Vla { width, .. } | WriteColumnData::VlaBits { width, .. } => {
                *width = DescriptorWidth::Q;
            }
            WriteColumnData::Fixed { .. } | WriteColumnData::Bits { .. } => {
                panic!("only a variable-length column takes `Q` descriptors")
            }
        }
        self
    }

    /// Emit `TSCALn`/`TZEROn` so the stored `data` reads back as
    /// `TZEROn + TSCALn × stored` physically.
    #[must_use]
    pub const fn scaled(mut self, tscale: f64, tzero: f64) -> Self {
        self.tscale = Some(tscale);
        self.tzero = Some(tzero);
        self
    }

    /// Emit `TNULLn`, the stored integer denoting an undefined element.
    #[must_use]
    pub const fn with_null(mut self, tnull: i64) -> Self {
        self.tnull = Some(tnull);
        self
    }

    const fn is_array(&self) -> bool {
        matches!(
            self.values,
            WriteColumnData::Vla { .. } | WriteColumnData::VlaBits { .. }
        )
    }

    /// The array of a variable-length column at `row`; `None` for a fixed-width column.
    fn array_len(&self, row: usize) -> Option<ArrayLen> {
        match &self.values {
            WriteColumnData::Vla { rows, width } => {
                let count = rows.range(row).len();
                let element_size = ColumnType::from_data(rows.values()).elem_size();
                Some(ArrayLen {
                    width: *width,
                    count,
                    bytes: count * element_size,
                })
            }
            WriteColumnData::VlaBits { rows, width } => {
                let count = rows.range(row).len();
                Some(ArrayLen {
                    width: *width,
                    count,
                    bytes: count.div_ceil(8),
                })
            }
            WriteColumnData::Fixed { .. } | WriteColumnData::Bits { .. } => None,
        }
    }

    /// The row count the payload implies: `None` for an empty zero-width column,
    /// which fits any row count.
    fn inferred_rows(&self) -> Result<Option<usize>> {
        let (count, row_width) = match &self.values {
            WriteColumnData::Fixed { data, repeat } => (data.element_count(), *repeat),
            WriteColumnData::Bits { bytes, bit_count } => (bytes.len(), bit_count.div_ceil(8)),
            WriteColumnData::Vla { rows, .. } => return Ok(Some(rows.len())),
            WriteColumnData::VlaBits { rows, .. } => return Ok(Some(rows.len())),
        };
        match count.checked_rem(row_width) {
            Some(0) => Ok(Some(count / row_width)),
            None if count == 0 => Ok(None),
            _ => Err(FitsError::PartialTableRow {
                column: self.name.clone(),
                count,
                row_width,
            }),
        }
    }
}

/// Validated binary-table write value. Row count is inferred from column payloads
/// unless an explicit count is supplied for an empty or zero-width schema.
#[derive(Debug, Clone, Default)]
pub struct Builder {
    pub(super) nrows: Option<usize>,
    pub(super) columns: Vec<WriteColumn>,
}

impl Builder {
    pub const fn new() -> Self {
        Self {
            nrows: None,
            columns: Vec::new(),
        }
    }

    /// Declare the row count for an empty/predeclared schema.
    pub const fn with_rows(nrows: usize) -> Self {
        Self {
            nrows: Some(nrows),
            columns: Vec::new(),
        }
    }

    /// Add a column and immediately validate its inferred row count against the
    /// columns already present.
    pub fn push(&mut self, column: WriteColumn) -> Result<&mut Self> {
        let inferred = column.inferred_rows()?;
        accept_row_count(&mut self.nrows, &column.name, inferred)?;
        self.columns.push(column);
        Ok(self)
    }

    pub fn column(mut self, column: WriteColumn) -> Result<Self> {
        self.push(column)?;
        Ok(self)
    }

    /// Build from an explicit row count and complete column list.
    pub fn explicit(nrows: usize, columns: impl IntoIterator<Item = WriteColumn>) -> Result<Self> {
        let mut table = Self::with_rows(nrows);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }

    /// The extent of the table this writes, checked before any byte is.
    fn layout(&self) -> Result<TableLayout> {
        let nrows = self.nrows.unwrap_or(0);
        validate_table_field_count(self.columns.len())?;
        value::fits_i64(nrows)?;
        let mut columns = Vec::with_capacity(self.columns.len());
        let mut row_len = 0usize;
        for (i, col) in self.columns.iter().enumerate() {
            let layout = validate_column(col, nrows, i + 1)?;
            row_len = row_len
                .checked_add(layout.row_width)
                .ok_or(FitsError::DataUnitOverflow)?;
            columns.push(layout);
        }
        value::fits_i64(row_len)?;
        let mut heap_len = 0usize;
        for array in self.heap_arrays(nrows) {
            heap_len = next_vla_heap_len(heap_len, array)?;
        }
        value::fits_i64(heap_len)?;
        Ok(TableLayout {
            nrows,
            row_len,
            heap_len,
            columns,
        })
    }

    /// The variable-length arrays of `nrows` rows in the order the heap holds them: row by
    /// row, each variable-length column in turn.
    fn heap_arrays(&self, nrows: usize) -> impl Iterator<Item = ArrayLen> + '_ {
        (0..self.heap_rows(nrows))
            .flat_map(move |r| self.columns.iter().filter_map(move |col| col.array_len(r)))
    }

    /// The rows of `nrows` that put an array in the heap: all of them when a column is
    /// variable-length, else none, so a table of fixed-width columns walks no row.
    fn heap_rows(&self, nrows: usize) -> usize {
        if self.columns.iter().any(WriteColumn::is_array) {
            nrows
        } else {
            0
        }
    }

    /// The `BINTABLE` header of the table laid out as `layout`.
    fn header(&self, layout: &TableLayout) -> Result<Header> {
        bintable_header(
            layout.nrows,
            layout.row_len,
            &self.columns,
            &layout.columns,
            layout.heap_len,
        )
    }

    /// The `BINTABLE` header, with the unpadded data unit (rows, then heap) written to
    /// `out`.
    fn encode(&self, out: &mut Vec<u8>) -> Result<Header> {
        let layout = self.layout()?;
        let TableLayout {
            nrows,
            row_len,
            heap_len,
            ..
        } = layout;
        let columns = &self.columns;
        out.clear();
        let main_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let total_len = main_len
            .checked_add(heap_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        out.reserve_exact(total_len);
        for r in 0..nrows {
            for col in columns {
                match &col.values {
                    WriteColumnData::Vla { width, .. } | WriteColumnData::VlaBits { width, .. } => {
                        out.resize(out.len() + width.bytes(), 0);
                    }
                    _ => pack_cell(out, col, r),
                }
            }
        }
        debug_assert_eq!(out.len(), main_len, "main table layout");
        for r in 0..self.heap_rows(nrows) {
            let mut column_offset = 0usize;
            for (col, column) in columns.iter().zip(&layout.columns) {
                match &col.values {
                    WriteColumnData::Vla { rows, width } => {
                        let range = rows.range(r);
                        let descriptor_offset = r * row_len + column_offset;
                        write_vla_descriptor(
                            out,
                            main_len,
                            descriptor_offset,
                            *width,
                            range.len(),
                        )?;
                        append_cells(out, rows.values(), range);
                    }
                    WriteColumnData::VlaBits { rows, width } => {
                        let bits = rows.row(r);
                        let descriptor_offset = r * row_len + column_offset;
                        write_vla_descriptor(out, main_len, descriptor_offset, *width, bits.len())?;
                        append_bits(out, bits);
                    }
                    _ => {}
                }
                column_offset += column.row_width;
            }
        }
        self.header(&layout)
    }
}

pub(super) fn write_template<W: Write>(
    writer: &mut FitsWriter<W>,
    table: &Builder,
    template: Option<&Header>,
) -> Result<()> {
    writer.ensure_writable()?;
    let header = table.encode(&mut writer.scratch)?;
    writer.finish_hdu(header, template, ZERO_FILL, Opening::EmptyPrimary)
}

/// Compress the table `write_template` would write, with `config`, straight from the
/// builder's columns: no row-major copy of the table is made.
#[cfg(feature = "compression")]
pub(super) fn write_compressed<W: Write>(
    writer: &mut FitsWriter<W>,
    table: &Builder,
    config: &CompressionConfig,
    template: Option<&Header>,
) -> Result<()> {
    writer.ensure_writable()?;
    let layout = table.layout()?;
    let mut header = table.header(&layout)?;
    merge_header_template(&mut header, template);
    let cells = BuilderCells::new(table, layout.nrows);
    let container = table::compress_table(&header, &cells, config, &mut writer.scratch)?;
    writer.finish_hdu(container, None, ZERO_FILL, Opening::EmptyPrimary)
}

/// A builder's cells by column, and the heap offset each array has in the table
/// `write_table` writes, which the compressed table keeps in its descriptors.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct BuilderCells<'a> {
    columns: &'a [WriteColumn],
    /// For each column, its index among the variable-length ones.
    arrays: Vec<Option<usize>>,
    array_count: usize,
    /// Row-major: each row's heap offset for each variable-length column.
    heap_offsets: Vec<usize>,
}

#[cfg(feature = "compression")]
impl<'a> BuilderCells<'a> {
    /// The cells of `table`'s `nrows` rows, which [`Builder::layout`] checked.
    fn new(table: &'a Builder, nrows: usize) -> Self {
        let mut array_count = 0;
        let arrays: Vec<Option<usize>> = table
            .columns
            .iter()
            .map(|col| {
                col.is_array().then(|| {
                    array_count += 1;
                    array_count - 1
                })
            })
            .collect();
        let mut heap_offsets = Vec::with_capacity(nrows * array_count);
        let mut heap_len = 0;
        for array in table.heap_arrays(nrows) {
            heap_offsets.push(heap_len);
            heap_len += array.bytes;
        }
        Self {
            columns: &table.columns,
            arrays,
            array_count,
            heap_offsets,
        }
    }
}

#[cfg(feature = "compression")]
impl TableCells for BuilderCells<'_> {
    fn fixed(&self, column: usize, rows: Range<usize>, out: &mut Vec<u8>) {
        let col = &self.columns[column];
        match &col.values {
            WriteColumnData::Fixed { data, repeat } => {
                append_cells(out, data, rows.start * repeat..rows.end * repeat);
            }
            _ => rows.for_each(|r| pack_cell(out, col, r)),
        }
    }

    fn array(&self, column: usize, row: usize, out: &mut Vec<u8>) -> PqDescriptor {
        let slot = self.arrays[column].expect("an array is read from a variable-length column");
        let offset = self.heap_offsets[row * self.array_count + slot];
        let count = match &self.columns[column].values {
            WriteColumnData::Vla { rows, .. } => {
                let range = rows.range(row);
                let count = range.len();
                append_cells(out, rows.values(), range);
                count
            }
            WriteColumnData::VlaBits { rows, .. } => {
                let bits = rows.row(row);
                append_bits(out, bits);
                bits.len()
            }
            WriteColumnData::Fixed { .. } | WriteColumnData::Bits { .. } => {
                unreachable!("a fixed-width column holds no array")
            }
        };
        PqDescriptor { count, offset }
    }
}

/// `BINTABLE` extension header (§7.3.1) for the given columns.
fn bintable_header(
    nrows: usize,
    row_len: usize,
    columns: &[WriteColumn],
    layouts: &[ColumnLayout],
    heap_len: usize,
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set_internal(keywords::XTENSION, "BINTABLE")
        .set_comment_internal(keywords::XTENSION, "binary table extension");
    header
        .set_internal(keywords::BITPIX, 8)
        .set_internal(keywords::NAXIS, 2);
    header
        .set_internal(keywords::NAXIS.indexed(1)?, value::fits_i64(row_len)?)
        .set_comment_internal(keywords::NAXIS.indexed(1)?, "width of table in bytes");
    header
        .set_internal(keywords::NAXIS.indexed(2)?, value::fits_i64(nrows)?)
        .set_comment_internal(keywords::NAXIS.indexed(2)?, "number of rows");
    header
        .set_internal(keywords::PCOUNT, value::fits_i64(heap_len)?)
        .set_internal(keywords::GCOUNT, 1);
    header
        .set_internal(keywords::TFIELDS, value::fits_i64(columns.len())?)
        .set_comment_internal(keywords::TFIELDS, "number of columns");
    for (i, (col, layout)) in columns.iter().zip(layouts).enumerate() {
        let n = i + 1;
        header.set_internal(keywords::TFORM.indexed(n)?, layout.tform.as_str());
        header.set_internal(keywords::TTYPE.indexed(n)?, col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set_internal(keywords::TUNIT.indexed(n)?, unit.as_str());
        }
        if let Some(shape) = &col.tdim {
            let dims: Vec<String> = shape.iter().map(ToString::to_string).collect();
            header.set_internal(keywords::TDIM.indexed(n)?, format!("({})", dims.join(",")));
        }
        if let Some(tscale) = col.tscale {
            header.set_internal(keywords::TSCAL.indexed(n)?, tscale);
        }
        if let Some(tzero) = col.tzero {
            let stores_i64 = match &col.values {
                WriteColumnData::Fixed { data, .. } => matches!(data, ColumnData::I64(_)),
                WriteColumnData::Vla { rows, .. } => matches!(rows.values(), ColumnData::I64(_)),
                WriteColumnData::VlaBits { .. } | WriteColumnData::Bits { .. } => false,
            };
            if stores_i64 && col.tscale.unwrap_or(1.0) == 1.0 && tzero == U64_OFFSET {
                header.set_internal(keywords::TZERO.indexed(n)?, U64_OFFSET_INTEGER);
            } else {
                header.set_internal(keywords::TZERO.indexed(n)?, tzero);
            }
        }
        if let Some(tnull) = col.tnull {
            header.set_internal(keywords::TNULL.indexed(n)?, tnull);
        }
    }
    Ok(header)
}

#[derive(Debug)]
struct ColumnLayout {
    row_width: usize,
    tform: String,
}

/// The extent of the table a [`Builder`] writes.
#[derive(Debug)]
struct TableLayout {
    nrows: usize,
    row_len: usize,
    heap_len: usize,
    columns: Vec<ColumnLayout>,
}

impl ColumnType {
    const fn from_data(data: &ColumnData) -> Self {
        match data {
            ColumnData::Logical(_) => Self::Logical,
            ColumnData::Bytes(_) => Self::Byte,
            ColumnData::I16(_) => Self::I16,
            ColumnData::I32(_) => Self::I32,
            ColumnData::I64(_) => Self::I64,
            ColumnData::F32(_) => Self::F32,
            ColumnData::F64(_) => Self::F64,
            ColumnData::ComplexF32(_) => Self::ComplexF32,
            ColumnData::ComplexF64(_) => Self::ComplexF64,
            ColumnData::Character(_) => Self::Character,
        }
    }

    /// The `TFORMn` element kind this writer type stores as. `ColumnType` is exactly
    /// the subset of [`TformKind`] a caller may name as a *stored element* — it omits
    /// `X` (bit arrays have their own constructors, which carry a bit count rather
    /// than an element count) and the `P`/`Q` descriptors (a heap array cannot itself
    /// be a descriptor). Widths and letters come from `TformKind` so the two cannot
    /// drift.
    const fn tform_kind(self) -> TformKind {
        match self {
            Self::Logical => TformKind::Logical,
            Self::Byte => TformKind::Byte,
            Self::I16 => TformKind::I16,
            Self::I32 => TformKind::I32,
            Self::I64 => TformKind::I64,
            Self::F32 => TformKind::F32,
            Self::F64 => TformKind::F64,
            Self::ComplexF32 => TformKind::ComplexF32,
            Self::ComplexF64 => TformKind::ComplexF64,
            Self::Character => TformKind::Char,
        }
    }

    const fn letter(self) -> char {
        self.tform_kind().code()
    }

    const fn elem_size(self) -> usize {
        self.tform_kind().elem_size()
    }
}

/// The heap's length after `array`, which its descriptor addresses at `heap_len`.
fn next_vla_heap_len(heap_len: usize, array: ArrayLen) -> Result<usize> {
    let count = u64::try_from(array.count).map_err(|_| FitsError::DataUnitOverflow)?;
    let offset = u64::try_from(heap_len).map_err(|_| FitsError::DataUnitOverflow)?;
    validate_pq_descriptor(array.width, count, offset)?;
    heap_len
        .checked_add(array.bytes)
        .ok_or(FitsError::DataUnitOverflow)
}

/// One variable-length array's descriptor width, element count and bytes in the heap.
#[derive(Debug, Clone, Copy)]
struct ArrayLen {
    width: DescriptorWidth,
    count: usize,
    bytes: usize,
}

fn write_vla_descriptor(
    out: &mut [u8],
    main_len: usize,
    descriptor_offset: usize,
    width: DescriptorWidth,
    element_count: usize,
) -> Result<()> {
    let count = u64::try_from(element_count).map_err(|_| FitsError::DataUnitOverflow)?;
    let heap_offset = out
        .len()
        .checked_sub(main_len)
        .expect("VLA heap follows the complete main table");
    let heap_offset = u64::try_from(heap_offset).map_err(|_| FitsError::DataUnitOverflow)?;
    write_pq_descriptor(
        &mut out[descriptor_offset..descriptor_offset + width.bytes()],
        width,
        count,
        heap_offset,
    )
}

/// The layout of `col`, the table's column `n`, after checking it.
fn validate_column(col: &WriteColumn, nrows: usize, n: usize) -> Result<ColumnLayout> {
    let tdim_keyword = keywords::TDIM.indexed(n)?;
    validate_ascii(&col.name, "binary column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "binary column unit")?;
    }
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let kind = ColumnType::from_data(data);
            validate_binary_metadata(col, kind.letter(), n)?;
            assert_eq!(data.element_count(), nrows * repeat, "{ROW_COUNT_FIXED}");
            if let ColumnData::Character(bytes) = data
                && *repeat > 0
            {
                for field in bytes.chunks(*repeat) {
                    validate_character(CharacterField::new(field), "binary character cell")?;
                }
            }
            tdim::validate_declared(col.tdim.as_deref(), *repeat, tdim_keyword)?;
            Ok(ColumnLayout {
                row_width: repeat
                    .checked_mul(kind.elem_size())
                    .ok_or(FitsError::DataUnitOverflow)?,
                tform: format!("{repeat}{}", kind.letter()),
            })
        }
        WriteColumnData::Vla { rows, width } => {
            let kind = ColumnType::from_data(rows.values());
            validate_binary_metadata(col, kind.letter(), n)?;
            assert_eq!(rows.len(), nrows, "{ROW_COUNT_FIXED}");
            let mut max_elements = 0usize;
            for r in 0..rows.len() {
                let range = rows.range(r);
                if let ColumnData::Character(bytes) = rows.values() {
                    validate_character(
                        CharacterField::new(&bytes[range.clone()]),
                        "binary VLA character cell",
                    )?;
                }
                max_elements = max_elements.max(range.len());
                tdim::validate_declared_vla(col.tdim.as_deref(), range.len(), tdim_keyword)?;
            }
            Ok(ColumnLayout {
                row_width: width.bytes(),
                tform: format!("1{}{}({max_elements})", width.letter(), kind.letter()),
            })
        }
        WriteColumnData::VlaBits { rows, width } => {
            validate_binary_metadata(col, 'X', n)?;
            assert_eq!(rows.len(), nrows, "{ROW_COUNT_FIXED}");
            let mut max_bits = 0usize;
            for r in 0..rows.len() {
                let bits = rows.range(r).len();
                max_bits = max_bits.max(bits);
                tdim::validate_declared_vla(col.tdim.as_deref(), bits, tdim_keyword)?;
            }
            Ok(ColumnLayout {
                row_width: width.bytes(),
                tform: format!("1{}X({max_bits})", width.letter()),
            })
        }
        WriteColumnData::Bits { bytes, bit_count } => {
            validate_binary_metadata(col, 'X', n)?;
            let row_width = bit_count.div_ceil(8);
            assert_eq!(bytes.len(), nrows * row_width, "{ROW_COUNT_FIXED}");
            tdim::validate_declared(col.tdim.as_deref(), *bit_count, tdim_keyword)?;
            Ok(ColumnLayout {
                row_width,
                tform: format!("{bit_count}X"),
            })
        }
    }
}

fn validate_binary_metadata(col: &WriteColumn, stored_type: char, n: usize) -> Result<()> {
    validate_scaling(
        col.tscale,
        col.tzero,
        !matches!(stored_type, 'A' | 'L' | 'X'),
        n,
    )?;
    let Some(tnull) = col.tnull else {
        return Ok(());
    };
    let valid = match stored_type {
        'B' => u8::try_from(tnull).is_ok(),
        'I' => i16::try_from(tnull).is_ok(),
        'J' => i32::try_from(tnull).is_ok(),
        'K' => true,
        _ => false,
    };
    if !valid {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::TNULL.indexed(n)?,
        });
    }
    Ok(())
}

/// Append `data[range]` to `out` as stored big-endian bytes.
fn append_cells(out: &mut Vec<u8>, data: &ColumnData, range: Range<usize>) {
    match data {
        ColumnData::Logical(v) => out.extend(v[range].iter().map(|&b| match b {
            Some(true) => b'T',
            Some(false) => b'F',
            None => 0, // §7.3.3 null
        })),
        ColumnData::Bytes(v) | ColumnData::Character(v) => out.extend_from_slice(&v[range]),
        ColumnData::I16(v) => extend_be(out, &v[range], i16::to_be_bytes),
        ColumnData::I32(v) => extend_be(out, &v[range], i32::to_be_bytes),
        ColumnData::I64(v) => extend_be(out, &v[range], i64::to_be_bytes),
        ColumnData::F32(v) => extend_be(out, &v[range], f32::to_be_bytes),
        ColumnData::F64(v) => extend_be(out, &v[range], f64::to_be_bytes),
        ColumnData::ComplexF32(v) => {
            for &Complex { re, im } in &v[range] {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
        ColumnData::ComplexF64(v) => {
            for &Complex { re, im } in &v[range] {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
    }
}

fn append_bits(out: &mut Vec<u8>, bits: &BitSlice<u8, Msb0>) {
    // Repacking semantic bits guarantees that unused low padding is zero.
    for chunk in bits.chunks(8) {
        let byte = chunk
            .iter()
            .by_vals()
            .enumerate()
            .fold(0, |byte, (bit, set)| byte | (u8::from(set) << (7 - bit)));
        out.push(byte);
    }
}

fn pack_cell(out: &mut Vec<u8>, col: &WriteColumn, r: usize) {
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let base = r * *repeat;
            append_cells(out, data, base..base + *repeat);
        }
        WriteColumnData::Bits { bytes, bit_count } => {
            let width = bit_count.div_ceil(8);
            let start = r * width;
            let cell = &bytes[start..start + width];
            let trailing_bits = bit_count % 8;
            if trailing_bits == 0 {
                out.extend_from_slice(cell);
            } else {
                out.extend_from_slice(&cell[..width - 1]);
                out.push(cell[width - 1] & (u8::MAX << (8 - trailing_bits)));
            }
        }
        WriteColumnData::Vla { .. } | WriteColumnData::VlaBits { .. } => {
            unreachable!("VLA cells are descriptors")
        }
    }
}

fn validate_character(value: CharacterField<'_>, context: &'static str) -> Result<()> {
    if value
        .members()
        .iter()
        .all(|byte| (0x20..=0x7e).contains(byte))
    {
        Ok(())
    } else {
        Err(FitsError::InvalidAscii { context })
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::fits_writer::builder::WriteColumn;

    pub(crate) fn set_scaling(column: &mut WriteColumn, tscale: Option<f64>, tzero: Option<f64>) {
        column.tscale = tscale;
        column.tzero = tzero;
    }
}
