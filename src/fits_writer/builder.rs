//! Binary-table writer schema, validation, and encoding.

use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use num_complex::Complex;

use crate::bintable::character_field::CharacterField;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tdim;
use crate::bintable::tform::{Storage, Tform};
use crate::bitpix::Bitpix;
use crate::column::validate_table_field_count;
#[cfg(feature = "compression")]
use crate::compression::CompressionConfig;
#[cfg(feature = "compression")]
use crate::compression::encoded_unit::EncodedUnit;
#[cfg(feature = "compression")]
use crate::compression::table::{self, TableCells};
use crate::fits_error::{FitsError, Result, TextOrigin};
use crate::fits_writer::column_scaling::{ColumnScaling, Scalable};
#[cfg(feature = "compression")]
use crate::fits_writer::merge_header_template;
use crate::fits_writer::{ROW_COUNT_FIXED, accept_row_count};
use crate::hdu::prologue::{Opening, Prologue, Xtension};
use crate::header_model::card::validate_ascii;
use crate::header_model::{Header, keywords, value};
use crate::image_model::{U64_OFFSET, U64_OFFSET_INTEGER};
use crate::ragged::Ragged;

/// The values of a binary-table writer column, in its cells or in the heap.
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
    scaling: Option<ColumnScaling>,
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
            scaling: None,
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

    /// A variable-length column of `width` descriptors, `P` or `Q`: row `r` stores
    /// `rows.range(r)` of the values, and the values' type is the heap element type,
    /// [`ColumnData::Bit`] for a `PX` or `QX` column.
    pub fn vla(name: impl Into<String>, rows: Ragged<ColumnData>, width: DescriptorWidth) -> Self {
        Self {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Vla { rows, width },
            tdim: None,
            scaling: None,
            tnull: None,
        }
    }

    /// An `X` column of `width` bits per row, one row of bits each, MSB-first. A row of
    /// another length is an error: unlike a text field, a bit row has no padding.
    pub fn bits(
        name: impl Into<String>,
        rows: impl IntoIterator<Item = impl AsRef<BitSlice<u8, Msb0>>>,
        width: usize,
    ) -> Result<Self> {
        let name = name.into();
        let mut values = BitVec::<u8, Msb0>::new();
        for (row, bits) in rows.into_iter().enumerate() {
            let bits = bits.as_ref();
            if bits.len() != width {
                return Err(FitsError::BitFieldLengthMismatch {
                    column: name,
                    row,
                    length: bits.len(),
                    width,
                });
            }
            values.extend_from_bitslice(bits);
        }
        Ok(Self::fixed(name, ColumnData::Bit(values), width))
    }

    /// Attach a unit (`TUNITn`).
    #[must_use]
    pub fn with_unit(mut self, unit: impl Into<String>) -> Self {
        self.unit = Some(unit.into());
        self
    }

    /// Attach a `TDIMn` array shape (fastest axis first).
    #[must_use]
    pub fn with_tdim(mut self, shape: impl Into<Vec<usize>>) -> Self {
        self.tdim = Some(shape.into());
        self
    }

    /// Emit `TSCALn`/`TZEROn` so the stored `data` reads back as
    /// `TZEROn + TSCALn × stored` physically.
    #[must_use]
    pub const fn with_scaling(mut self, tscale: f64, tzero: f64) -> Self {
        self.scaling = Some(ColumnScaling { tscale, tzero });
        self
    }

    /// Emit `TNULLn`, the stored integer denoting an undefined element.
    #[must_use]
    pub const fn with_null(mut self, tnull: i64) -> Self {
        self.tnull = Some(tnull);
        self
    }

    const fn is_array(&self) -> bool {
        matches!(self.values, WriteColumnData::Vla { .. })
    }

    /// The array of a variable-length column at `row`; `None` for a fixed-width column.
    fn array_len(&self, row: usize) -> Option<ArrayLen> {
        match &self.values {
            WriteColumnData::Vla { rows, width } => {
                let count = rows.range(row).len();
                Some(ArrayLen {
                    width: *width,
                    count,
                    bytes: rows
                        .values()
                        .element_kind()
                        .payload_len(count)
                        .expect("an array's bytes fit in memory, as its values do"),
                })
            }
            WriteColumnData::Fixed { .. } => None,
        }
    }

    /// The row count the payload implies: `None` for an empty zero-width column,
    /// which fits any row count.
    fn inferred_rows(&self) -> Result<Option<usize>> {
        let (count, row_width) = match &self.values {
            WriteColumnData::Fixed { data, repeat } => (data.len(), *repeat),
            WriteColumnData::Vla { rows, .. } => return Ok(Some(rows.len())),
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
    pub(super) row_count: Option<usize>,
    pub(super) columns: Vec<WriteColumn>,
}

impl Builder {
    pub const fn new() -> Self {
        Self {
            row_count: None,
            columns: Vec::new(),
        }
    }

    /// Declare the row count for an empty/predeclared schema.
    pub const fn with_rows(row_count: usize) -> Self {
        Self {
            row_count: Some(row_count),
            columns: Vec::new(),
        }
    }

    /// Add a column and immediately validate its inferred row count against the
    /// columns already present.
    pub fn push(&mut self, column: WriteColumn) -> Result<&mut Self> {
        let inferred = column.inferred_rows()?;
        accept_row_count(&mut self.row_count, &column.name, inferred)?;
        self.columns.push(column);
        Ok(self)
    }

    /// Build from an explicit row count and complete column list.
    pub fn with_columns(
        row_count: usize,
        columns: impl IntoIterator<Item = WriteColumn>,
    ) -> Result<Self> {
        let mut table = Self::with_rows(row_count);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }

    /// The extent of the table this writes, checked before any byte is.
    fn layout(&self) -> Result<TableLayout> {
        let row_count = self.row_count.unwrap_or(0);
        validate_table_field_count(self.columns.len())?;
        value::fits_i64(row_count)?;
        let mut columns = Vec::with_capacity(self.columns.len());
        let mut row_len = 0usize;
        for (i, col) in self.columns.iter().enumerate() {
            let layout = validate_column(col, row_count, i + 1)?;
            row_len = row_len
                .checked_add(layout.row_width)
                .ok_or(FitsError::DataUnitOverflow)?;
            columns.push(layout);
        }
        value::fits_i64(row_len)?;
        let mut heap_len = 0usize;
        for array in self.heap_arrays(row_count) {
            heap_len = next_vla_heap_len(heap_len, array)?;
        }
        value::fits_i64(heap_len)?;
        Ok(TableLayout {
            row_count,
            row_len,
            heap_len,
            columns,
        })
    }

    /// The variable-length arrays of `row_count` rows in the order the heap holds them: row by
    /// row, each variable-length column in turn.
    fn heap_arrays(&self, row_count: usize) -> impl Iterator<Item = ArrayLen> + '_ {
        (0..self.heap_rows(row_count))
            .flat_map(move |r| self.columns.iter().filter_map(move |col| col.array_len(r)))
    }

    /// The heap bytes of row `row`'s arrays.
    #[cfg(feature = "compression")]
    fn row_heap_len(&self, row: usize) -> usize {
        self.columns
            .iter()
            .filter_map(|col| col.array_len(row))
            .map(|array| array.bytes)
            .sum()
    }

    /// The rows of `row_count` that put an array in the heap: all of them when a column is
    /// variable-length, else none, so a table of fixed-width columns walks no row.
    fn heap_rows(&self, row_count: usize) -> usize {
        if self.columns.iter().any(WriteColumn::is_array) {
            row_count
        } else {
            0
        }
    }

    /// The `BINTABLE` extension header (§7.3.1) of the table laid out as `layout`.
    fn header(&self, layout: &TableLayout) -> Result<Header> {
        let mut header = Prologue {
            opening: Opening::Extension(Xtension::BinTable),
            bitpix: Bitpix::U8,
            axes: &[layout.row_len, layout.row_count],
            pcount: layout.heap_len,
        }
        .header()?;
        header
            .set_comment_internal(keywords::NAXIS.indexed(1)?, "width of table in bytes")
            .set_comment_internal(keywords::NAXIS.indexed(2)?, "number of rows");
        header
            .set_internal(keywords::TFIELDS, value::fits_i64(self.columns.len())?)
            .set_comment_internal(keywords::TFIELDS, "number of columns");
        for (i, (col, column)) in self.columns.iter().zip(&layout.columns).enumerate() {
            let n = i + 1;
            header.set_internal(keywords::TFORM.indexed(n)?, column.tform.as_str());
            header.set_internal(keywords::TTYPE.indexed(n)?, col.name.as_str());
            if let Some(unit) = &col.unit {
                header.set_internal(keywords::TUNIT.indexed(n)?, unit.as_str());
            }
            if let Some(shape) = &col.tdim {
                let dims: Vec<String> = shape.iter().map(ToString::to_string).collect();
                header.set_internal(keywords::TDIM.indexed(n)?, format!("({})", dims.join(",")));
            }
            if let Some(ColumnScaling { tscale, tzero }) = col.scaling {
                header.set_internal(keywords::TSCAL.indexed(n)?, tscale);
                let values = match &col.values {
                    WriteColumnData::Fixed { data, .. } => data,
                    WriteColumnData::Vla { rows, .. } => rows.values(),
                };
                let stores_i64 = matches!(values, ColumnData::I64(_));
                if stores_i64 && tscale == 1.0 && tzero == U64_OFFSET {
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

    /// The `BINTABLE` header, with the unpadded data unit (rows, then heap) written to
    /// `out`. The main table is written column by column, each cell at its row's
    /// stride, and the heap row by row, as it is laid out.
    pub(super) fn encode(&self, out: &mut Vec<u8>) -> Result<Header> {
        let layout = self.layout()?;
        let TableLayout {
            row_count,
            row_len,
            heap_len,
            ..
        } = layout;
        let main_len = row_count
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let total_len = main_len
            .checked_add(heap_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        out.clear();
        out.reserve_exact(total_len);
        out.resize(main_len, 0);
        let mut offset = 0;
        for (col, column) in self.columns.iter().zip(&layout.columns) {
            // An array column's descriptors are written with the heap.
            if let WriteColumnData::Fixed { data, repeat } = &col.values {
                let cells = Cells {
                    row_len,
                    offset,
                    row_count,
                    repeat: *repeat,
                };
                let values = Values {
                    data,
                    range: 0..row_count * repeat,
                };
                cells.scatter(values, out);
            }
            offset += column.row_width;
        }
        for r in 0..self.heap_rows(row_count) {
            let mut at = r * row_len;
            for (col, column) in self.columns.iter().zip(&layout.columns) {
                let cell = at..at + column.row_width;
                at = cell.end;
                let WriteColumnData::Vla { rows, width } = &col.values else {
                    continue;
                };
                let range = rows.range(r);
                let descriptor = PqDescriptor {
                    count: range.len(),
                    offset: out.len() - main_len,
                };
                descriptor.write(*width, &mut out[cell])?;
                let values = Values {
                    data: rows.values(),
                    range,
                };
                append_array(out, values);
            }
        }
        debug_assert_eq!(out.len(), total_len, "the data unit's layout");
        self.header(&layout)
    }
}

impl Builder {
    /// The table `FitsWriter::write_table` would write, with the non-structural cards of
    /// `template`, compressed with `config` straight from the builder's columns: no
    /// row-major copy of the table is made.
    #[cfg(feature = "compression")]
    pub(super) fn compress(
        &self,
        config: &CompressionConfig,
        template: Option<&Header>,
    ) -> Result<EncodedUnit> {
        let layout = self.layout()?;
        let mut header = self.header(&layout)?;
        if let Some(template) = template {
            merge_header_template(&mut header, template);
        }
        let cells = BuilderCells::new(self, layout.row_count);
        table::compress_table(&header, &cells, config)
    }
}

/// A builder's cells by column, and the heap offset each array has in the table
/// `write_table` writes, which the compressed table keeps in its descriptors.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct BuilderCells<'a> {
    columns: &'a [WriteColumn],
    /// Each row's heap offset: where the first of its arrays starts.
    row_starts: Vec<usize>,
}

#[cfg(feature = "compression")]
impl<'a> BuilderCells<'a> {
    /// The cells of `table`'s `row_count` rows, which [`Builder::layout`] checked.
    fn new(table: &'a Builder, row_count: usize) -> Self {
        let rows = table.heap_rows(row_count);
        let mut row_starts = Vec::with_capacity(rows);
        let mut heap_len = 0;
        for row in 0..rows {
            row_starts.push(heap_len);
            heap_len += table.row_heap_len(row);
        }
        Self {
            columns: &table.columns,
            row_starts,
        }
    }
}

#[cfg(feature = "compression")]
impl TableCells for BuilderCells<'_> {
    fn fixed(&self, column: usize, rows: Range<usize>, out: &mut Vec<u8>) {
        let WriteColumnData::Fixed { data, repeat } = &self.columns[column].values else {
            unreachable!("a variable-length column holds no fixed-width cell")
        };
        let values = Values {
            data,
            range: rows.start * repeat..rows.end * repeat,
        };
        append_cells(out, values, *repeat);
    }

    fn array(&self, column: usize, row: usize, out: &mut Vec<u8>) -> PqDescriptor {
        let before: usize = self.columns[..column]
            .iter()
            .filter_map(|col| col.array_len(row))
            .map(|array| array.bytes)
            .sum();
        let offset = self.row_starts[row] + before;
        let WriteColumnData::Vla { rows, .. } = &self.columns[column].values else {
            unreachable!("a fixed-width column holds no array")
        };
        let range = rows.range(row);
        let count = range.len();
        let values = Values {
            data: rows.values(),
            range,
        };
        append_array(out, values);
        PqDescriptor { count, offset }
    }
}

#[derive(Debug)]
struct ColumnLayout {
    row_width: usize,
    tform: String,
}

/// The extent of the table a [`Builder`] writes.
#[derive(Debug)]
struct TableLayout {
    row_count: usize,
    row_len: usize,
    heap_len: usize,
    columns: Vec<ColumnLayout>,
}

/// The heap's length after `array`, which its descriptor addresses at `heap_len`.
fn next_vla_heap_len(heap_len: usize, array: ArrayLen) -> Result<usize> {
    PqDescriptor {
        count: array.count,
        offset: heap_len,
    }
    .validate(array.width)?;
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

/// The layout of `col`, the table's column `n`, after checking it.
fn validate_column(col: &WriteColumn, row_count: usize, n: usize) -> Result<ColumnLayout> {
    let tdim_keyword = keywords::TDIM.indexed(n)?;
    validate_ascii(&col.name, || TextOrigin::ColumnName)?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, || TextOrigin::ColumnUnit)?;
    }
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let kind = data.element_kind();
            validate_binary_metadata(col, kind, n)?;
            assert_eq!(data.len(), row_count * repeat, "{ROW_COUNT_FIXED}");
            if let ColumnData::Character(bytes) = data
                && *repeat > 0
            {
                for field in bytes.chunks(*repeat) {
                    validate_character(CharacterField::new(field))?;
                }
            }
            tdim::validate_declared(col.tdim.as_deref(), *repeat, tdim_keyword)?;
            Ok(ColumnLayout {
                row_width: kind
                    .payload_len(*repeat)
                    .ok_or(FitsError::DataUnitOverflow)?,
                tform: Tform::new(*repeat, kind, Storage::Fixed).to_string(),
            })
        }
        WriteColumnData::Vla { rows, width } => {
            let kind = rows.values().element_kind();
            validate_binary_metadata(col, kind, n)?;
            assert_eq!(rows.len(), row_count, "{ROW_COUNT_FIXED}");
            let mut max_elements = 0usize;
            for r in 0..rows.len() {
                let range = rows.range(r);
                if let ColumnData::Character(bytes) = rows.values() {
                    validate_character(CharacterField::new(&bytes[range.clone()]))?;
                }
                max_elements = max_elements.max(range.len());
                tdim::validate_declared_vla(col.tdim.as_deref(), range.len(), tdim_keyword)?;
            }
            Ok(ColumnLayout {
                row_width: width.bytes(),
                tform: format!(
                    "{}({max_elements})",
                    Tform::new(1, kind, Storage::Array(*width))
                ),
            })
        }
    }
}

/// Errors when column `n`, stored as `kind`, sets a scaling or a `TNULLn` its values
/// cannot take.
fn validate_binary_metadata(col: &WriteColumn, kind: ElementKind, n: usize) -> Result<()> {
    let scalable = match kind {
        ElementKind::Character | ElementKind::Logical | ElementKind::Bit => Scalable::Unscaled,
        ElementKind::U8
        | ElementKind::I16
        | ElementKind::I32
        | ElementKind::I64
        | ElementKind::F32
        | ElementKind::F64
        | ElementKind::ComplexF32
        | ElementKind::ComplexF64 => Scalable::Numeric,
    };
    if let Some(scaling) = col.scaling {
        scaling.validate(scalable, n)?;
    }
    let Some(tnull) = col.tnull else {
        return Ok(());
    };
    if !kind
        .bitpix()
        .is_some_and(|bitpix| bitpix.holds_integer(tnull))
    {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::TNULL.indexed(n)?,
        });
    }
    Ok(())
}

/// Where a column's cells lie in a main table: `row_count` rows of `row_len` bytes,
/// each cell at `offset` in its row and `repeat` values long.
#[derive(Debug, Clone, Copy)]
struct Cells {
    row_len: usize,
    offset: usize,
    row_count: usize,
    repeat: usize,
}

/// A run of a column's values: `data[range]`.
#[derive(Debug, Clone)]
struct Values<'a> {
    data: &'a ColumnData,
    range: Range<usize>,
}

impl Cells {
    /// Write `values` as stored big-endian bytes, a cell a row.
    fn scatter(self, values: Values<'_>, out: &mut [u8]) {
        let Values { data, range } = values;
        match data {
            ColumnData::Bit(bits) => self.scatter_bits(&bits[range], out),
            ColumnData::Logical(v) => self.scatter_with(&v[range], out, |value| {
                [match value {
                    Some(true) => b'T',
                    Some(false) => b'F',
                    None => 0, // §7.3.3 null
                }]
            }),
            ColumnData::U8(v) | ColumnData::Character(v) => {
                self.scatter_with(&v[range], out, |byte| [byte]);
            }
            ColumnData::I16(v) => self.scatter_with(&v[range], out, i16::to_be_bytes),
            ColumnData::I32(v) => self.scatter_with(&v[range], out, i32::to_be_bytes),
            ColumnData::I64(v) => self.scatter_with(&v[range], out, i64::to_be_bytes),
            ColumnData::F32(v) => self.scatter_with(&v[range], out, f32::to_be_bytes),
            ColumnData::F64(v) => self.scatter_with(&v[range], out, f64::to_be_bytes),
            ColumnData::ComplexF32(v) => {
                self.scatter_with(&v[range], out, |Complex { re, im }| {
                    let mut bytes = [0; 8];
                    bytes[..4].copy_from_slice(&re.to_be_bytes());
                    bytes[4..].copy_from_slice(&im.to_be_bytes());
                    bytes
                });
            }
            ColumnData::ComplexF64(v) => {
                self.scatter_with(&v[range], out, |Complex { re, im }| {
                    let mut bytes = [0; 16];
                    bytes[..8].copy_from_slice(&re.to_be_bytes());
                    bytes[8..].copy_from_slice(&im.to_be_bytes());
                    bytes
                });
            }
        }
    }

    fn scatter_with<T: Copy, const N: usize>(
        self,
        values: &[T],
        out: &mut [u8],
        encode: impl Fn(T) -> [u8; N],
    ) {
        let repeat = self.repeat;
        let cell_len = repeat * N;
        if cell_len == 0 {
            return;
        }
        debug_assert_eq!(
            values.len(),
            self.row_count * repeat,
            "a value for each cell"
        );
        let rows = out.chunks_exact_mut(self.row_len);
        for (row, cell) in rows.zip(values.chunks_exact(repeat)) {
            let slots = row[self.offset..self.offset + cell_len]
                .as_chunks_mut::<N>()
                .0;
            for (slot, &value) in slots.iter_mut().zip(cell) {
                *slot = encode(value);
            }
        }
    }

    /// Write `bits`, the cells' repeat to a cell, MSB-first; the zeroed cells keep the
    /// unused low bits of their last byte zero.
    fn scatter_bits(self, bits: &BitSlice<u8, Msb0>, out: &mut [u8]) {
        let repeat = self.repeat;
        if repeat == 0 {
            return;
        }
        debug_assert_eq!(bits.len(), self.row_count * repeat, "a bit for each cell");
        let cell_len = repeat.div_ceil(8);
        let rows = out.chunks_exact_mut(self.row_len);
        for (row, cell) in rows.zip(bits.chunks_exact(repeat)) {
            let bytes = &mut row[self.offset..self.offset + cell_len];
            BitSlice::<u8, Msb0>::from_slice_mut(bytes)[..repeat].copy_from_bitslice(cell);
        }
    }
}

/// Append `values` to `out` as stored cells of `repeat` values each, every cell padded to
/// whole bytes.
fn append_cells(out: &mut Vec<u8>, values: Values<'_>, repeat: usize) {
    if repeat == 0 {
        return;
    }
    let cell_len = values
        .data
        .element_kind()
        .payload_len(repeat)
        .expect("a cell's bytes fit in memory, as its values do");
    let row_count = values.range.len() / repeat;
    let start = out.len();
    out.resize(start + row_count * cell_len, 0);
    let cells = Cells {
        row_len: cell_len,
        offset: 0,
        row_count,
        repeat,
    };
    cells.scatter(values, &mut out[start..]);
}

/// Append the heap array `values` to `out`: one cell of all its values.
fn append_array(out: &mut Vec<u8>, values: Values<'_>) {
    let len = values.range.len();
    append_cells(out, values, len);
}

fn validate_character(value: CharacterField<'_>) -> Result<()> {
    if value
        .members()
        .iter()
        .all(|byte| (0x20..=0x7e).contains(byte))
    {
        Ok(())
    } else {
        Err(FitsError::InvalidAscii {
            text: TextOrigin::Cell,
        })
    }
}
