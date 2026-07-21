//! Binary-table writer schema, validation, and encoding.

use std::io::Write;
use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use num_complex::Complex;

use crate::block::ZERO_FILL;
#[cfg(feature = "compression")]
use crate::compress::{Compression, compress_table};
use crate::data::{U64_OFFSET, U64_OFFSET_INTEGER};
use crate::endian::{extend_be, validate_pq_descriptor, write_pq_descriptor};
use crate::error::{FitsError, Result};
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::header::card::validate_ascii;
use crate::keyword::key;
#[cfg(feature = "compression")]
use crate::table_impl::BinTable;
use crate::table_impl::{CharacterField, ColumnData};
use crate::writer::{FitsWriter, fits_i64, merge_header_template, validate_scaling};

/// An element type accepted by a binary-table writer column.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnType {
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
        kind: ColumnType,
        rows: Vec<ColumnData>,
        wide: bool,
    },
    VlaBits {
        rows: Vec<BitVec<u8, Msb0>>,
        wide: bool,
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
    pub fn scalar(name: impl Into<String>, data: ColumnData) -> WriteColumn {
        WriteColumn::fixed(name, data, 1)
    }

    /// A fixed-width column of `repeat` elements per row.
    ///
    /// This is the explicit-schema constructor for vector columns and empty
    /// templates. Prefer [`WriteColumn::scalar`] when every row has one value.
    pub fn fixed(name: impl Into<String>, data: ColumnData, repeat: usize) -> WriteColumn {
        WriteColumn {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Fixed { data, repeat },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// A variable-length `P` column whose heap element type is inferred from its
    /// first row and checked against every remaining row.
    pub fn vla(name: impl Into<String>, rows: Vec<ColumnData>) -> Result<WriteColumn> {
        let name = name.into();
        let kind = rows.first().map(ColumnType::from_data).ok_or_else(|| {
            FitsError::EmptyVlaNeedsType {
                column: name.clone(),
            }
        })?;
        WriteColumn::vla_typed(name, kind, rows)
    }

    /// Explicit-schema VLA constructor. Use this for an empty VLA column or when a
    /// predeclared heap type is part of the schema.
    pub fn vla_typed(
        name: impl Into<String>,
        kind: ColumnType,
        rows: Vec<ColumnData>,
    ) -> Result<WriteColumn> {
        let name = name.into();
        if let Some(row) = rows.iter().position(|data| !kind.matches(data)) {
            return Err(FitsError::TypeMismatch {
                name: format!("VLA column {name:?} row {row}"),
                expected: kind.name(),
            });
        }
        Ok(WriteColumn {
            name,
            unit: None,
            values: WriteColumnData::Vla {
                kind,
                rows,
                wide: false,
            },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        })
    }

    /// An `X` column of `bit_count` bits per row and its row-packed bytes.
    pub fn bits(name: impl Into<String>, bytes: Vec<u8>, bit_count: usize) -> WriteColumn {
        WriteColumn {
            name: name.into(),
            unit: None,
            values: WriteColumnData::Bits { bytes, bit_count },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// A variable-length `PX` bit-array column, with one MSB-first [`BitVec`] per
    /// table row. Each vector's bit length becomes its descriptor element count;
    /// [`WriteColumn::wide`] changes the descriptors to `QX`.
    pub fn vla_bits(name: impl Into<String>, rows: Vec<BitVec<u8, Msb0>>) -> WriteColumn {
        WriteColumn {
            name: name.into(),
            unit: None,
            values: WriteColumnData::VlaBits { rows, wide: false },
            tdim: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// Attach a unit (`TUNITn`).
    pub fn with_unit(mut self, unit: impl Into<String>) -> WriteColumn {
        self.unit = Some(unit.into());
        self
    }

    /// Attach a `TDIMn` array shape (fastest axis first).
    pub fn with_tdim(mut self, shape: Vec<usize>) -> WriteColumn {
        self.tdim = Some(shape);
        self
    }

    /// Use 64-bit `Q` descriptors for this VLA column.
    pub fn wide(mut self) -> Result<WriteColumn> {
        match &mut self.values {
            WriteColumnData::Vla { wide, .. } | WriteColumnData::VlaBits { wide, .. } => {
                *wide = true;
            }
            WriteColumnData::Fixed { data, .. } => {
                return Err(FitsError::NotAVla {
                    code: ColumnType::from_data(data).letter(),
                });
            }
            WriteColumnData::Bits { .. } => return Err(FitsError::NotAVla { code: 'X' }),
        };
        Ok(self)
    }

    /// Emit `TSCALn`/`TZEROn` so the stored `data` reads back as
    /// `TZEROn + TSCALn × stored` physically.
    pub fn scaled(mut self, tscale: f64, tzero: f64) -> WriteColumn {
        self.tscale = Some(tscale);
        self.tzero = Some(tzero);
        self
    }

    /// Emit `TNULLn`, the stored integer denoting an undefined element.
    pub fn with_null(mut self, tnull: i64) -> WriteColumn {
        self.tnull = Some(tnull);
        self
    }

    fn inferred_rows(&self) -> Result<Option<usize>> {
        match &self.values {
            WriteColumnData::Fixed { data, repeat } => {
                if matches!(data, ColumnData::Character(_)) {
                    return Ok(Some(data.element_count()));
                }
                if *repeat == 0 {
                    return Ok(None);
                }
                let count = data.element_count();
                if count % repeat != 0 {
                    return Err(FitsError::TableRowCountMismatch {
                        column: self.name.clone(),
                        expected: count.div_ceil(*repeat),
                        got: count / repeat,
                    });
                }
                Ok(Some(count / repeat))
            }
            WriteColumnData::Vla { rows, .. } => Ok(Some(rows.len())),
            WriteColumnData::VlaBits { rows, .. } => Ok(Some(rows.len())),
            WriteColumnData::Bits { bytes, bit_count } => {
                let width = bit_count.div_ceil(8);
                if width == 0 {
                    return Ok(None);
                }
                if bytes.len() % width != 0 {
                    return Err(FitsError::TableRowCountMismatch {
                        column: self.name.clone(),
                        expected: bytes.len().div_ceil(width),
                        got: bytes.len() / width,
                    });
                }
                Ok(Some(bytes.len() / width))
            }
        }
    }
}

/// Validated binary-table write value. Row count is inferred from column payloads
/// unless an explicit count is supplied for an empty or zero-width schema.
#[derive(Debug, Clone, Default)]
pub struct TableBuilder {
    pub(crate) nrows: Option<usize>,
    pub(crate) columns: Vec<WriteColumn>,
}

impl TableBuilder {
    pub fn new() -> TableBuilder {
        TableBuilder::default()
    }

    /// Declare the row count for an empty/predeclared schema.
    pub fn with_rows(nrows: usize) -> TableBuilder {
        TableBuilder {
            nrows: Some(nrows),
            columns: Vec::new(),
        }
    }

    /// Add a column and immediately validate its inferred row count against the
    /// columns already present.
    pub fn push(&mut self, column: WriteColumn) -> Result<&mut Self> {
        match (self.nrows, column.inferred_rows()?) {
            (Some(expected), Some(got)) if expected != got => {
                return Err(FitsError::TableRowCountMismatch {
                    column: column.name.clone(),
                    expected,
                    got,
                });
            }
            (None, Some(rows)) => self.nrows = Some(rows),
            (None, None) => {
                return Err(FitsError::TableRowCountUndetermined {
                    column: column.name.clone(),
                });
            }
            _ => {}
        }
        self.columns.push(column);
        Ok(self)
    }

    pub fn column(mut self, column: WriteColumn) -> Result<TableBuilder> {
        self.push(column)?;
        Ok(self)
    }

    /// Build from an explicit row count and complete column list.
    pub fn explicit(
        nrows: usize,
        columns: impl IntoIterator<Item = WriteColumn>,
    ) -> Result<TableBuilder> {
        let mut table = TableBuilder::with_rows(nrows);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }
}

impl<W: Write> FitsWriter<W> {
    /// Write a binary table as a `BINTABLE` extension. A dataless primary HDU is
    /// written automatically first if nothing has been written yet (a table can
    /// never be the primary HDU). Fixed-width and variable-length (`P`/`Q`) columns
    /// are both supported, including jagged `PX`/`QX` bit arrays — VLA columns
    /// write a heap after the main table.
    pub fn write_table(&mut self, table: &TableBuilder) -> Result<()> {
        self.write_table_template(table, None)
    }

    /// Write a binary table while preserving the non-structural cards from `header`.
    /// Mandatory table-layout and checksum cards are regenerated from `columns`.
    pub fn write_table_with_header(&mut self, table: &TableBuilder, header: &Header) -> Result<()> {
        self.write_table_template(table, Some(header))
    }

    fn write_table_template(
        &mut self,
        table: &TableBuilder,
        template: Option<&Header>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let nrows = table.nrows.unwrap_or(0);
        let columns = &table.columns;
        validate_table_field_count(columns.len())?;
        fits_i64(nrows)?;
        let mut layouts = Vec::with_capacity(columns.len());
        let mut row_len = 0usize;
        for col in columns {
            let layout = validate_column(col, nrows)?;
            row_len = row_len
                .checked_add(layout.row_width)
                .ok_or(FitsError::DataUnitOverflow)?;
            layouts.push(layout);
        }
        fits_i64(row_len)?;
        let mut heap_len = 0usize;
        for r in 0..nrows {
            for col in columns {
                match &col.values {
                    WriteColumnData::Vla { kind, rows, wide } => {
                        let cell = &rows[r];
                        heap_len = next_vla_heap_len(
                            heap_len,
                            *wide,
                            encoded_element_count(*kind, cell)?,
                            cell_byte_len(*kind, cell)?,
                        )?;
                    }
                    WriteColumnData::VlaBits { rows, wide } => {
                        let bits = &rows[r];
                        heap_len =
                            next_vla_heap_len(heap_len, *wide, bits.len(), bits.len().div_ceil(8))?;
                    }
                    _ => {}
                }
            }
        }
        fits_i64(heap_len)?;
        self.scratch.clear();
        let main_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let total_len = main_len
            .checked_add(heap_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.scratch.reserve_exact(total_len);
        for r in 0..nrows {
            for col in columns {
                match &col.values {
                    WriteColumnData::Vla { wide, .. } | WriteColumnData::VlaBits { wide, .. } => {
                        self.scratch
                            .resize(self.scratch.len() + if *wide { 16 } else { 8 }, 0)
                    }
                    _ => pack_cell(&mut self.scratch, col, r),
                }
            }
        }
        assert_eq!(self.scratch.len(), main_len, "main table layout");
        for r in 0..nrows {
            let mut column_offset = 0usize;
            for (col, layout) in columns.iter().zip(&layouts) {
                match &col.values {
                    WriteColumnData::Vla { kind, rows, wide } => {
                        let cell = &rows[r];
                        let descriptor_offset = r * row_len + column_offset;
                        write_vla_descriptor(
                            &mut self.scratch,
                            main_len,
                            descriptor_offset,
                            *wide,
                            encoded_element_count(*kind, cell)?,
                        )?;
                        append_be(&mut self.scratch, cell);
                    }
                    WriteColumnData::VlaBits { rows, wide } => {
                        let bits = &rows[r];
                        let descriptor_offset = r * row_len + column_offset;
                        write_vla_descriptor(
                            &mut self.scratch,
                            main_len,
                            descriptor_offset,
                            *wide,
                            bits.len(),
                        )?;
                        append_bits(&mut self.scratch, bits);
                    }
                    _ => {}
                }
                column_offset += layout.row_width;
            }
        }
        let header = bintable_header(nrows, row_len, columns, &layouts, heap_len, template)?;
        self.finish_hdu(header, ZERO_FILL, true)
    }

    /// Write a `BINTABLE` as a tiled-compressed table (§10.3). `header` is the
    /// original table's header (column metadata is copied from it), `table` its
    /// parsed data, `rows_per_tile` the tile height, and `compression` the default
    /// per-column codec (`GZIP_1`/`GZIP_2`/`RICE_1`/`NOCOMPRESS`). Fixed-width and
    /// P/Q variable-length columns are supported. Requires the `compression`
    /// feature.
    #[cfg(feature = "compression")]
    pub fn write_compressed_table(
        &mut self,
        header: &Header,
        table: &BinTable,
        rows_per_tile: usize,
        compression: Compression,
    ) -> Result<()> {
        self.ensure_writable()?;
        let zheader = compress_table(header, table, rows_per_tile, compression, &mut self.scratch)?;
        self.finish_hdu(zheader, ZERO_FILL, true)
    }
}

/// `BINTABLE` extension header (§7.3.1) for the given columns.
fn bintable_header(
    nrows: usize,
    row_len: usize,
    columns: &[WriteColumn],
    layouts: &[ColumnLayout],
    heap_len: usize,
    template: Option<&Header>,
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "BINTABLE")
        .comment_internal("XTENSION", "binary table extension");
    header.set_internal("BITPIX", 8).set_internal("NAXIS", 2);
    header
        .set_internal("NAXIS1", fits_i64(row_len)?)
        .comment_internal("NAXIS1", "width of table in bytes");
    header
        .set_internal("NAXIS2", fits_i64(nrows)?)
        .comment_internal("NAXIS2", "number of rows");
    header
        .set_internal("PCOUNT", fits_i64(heap_len)?)
        .set_internal("GCOUNT", 1);
    header
        .set_internal("TFIELDS", fits_i64(columns.len())?)
        .comment_internal("TFIELDS", "number of columns");
    for (i, (col, layout)) in columns.iter().zip(layouts).enumerate() {
        let n = i + 1;
        header.set_internal(key!("TFORM{n}").as_str(), layout.tform.as_str());
        header.set_internal(key!("TTYPE{n}").as_str(), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set_internal(key!("TUNIT{n}").as_str(), unit.as_str());
        }
        if let Some(shape) = &col.tdim {
            let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            header.set_internal(key!("TDIM{n}").as_str(), format!("({})", dims.join(",")));
        }
        if let Some(tscale) = col.tscale {
            header.set_internal(key!("TSCAL{n}").as_str(), tscale);
        }
        if let Some(tzero) = col.tzero {
            let stores_i64 = match &col.values {
                WriteColumnData::Fixed { data, .. } => matches!(data, ColumnData::I64(_)),
                WriteColumnData::Vla { kind, .. } => *kind == ColumnType::I64,
                WriteColumnData::VlaBits { .. } | WriteColumnData::Bits { .. } => false,
            };
            if stores_i64 && col.tscale.unwrap_or(1.0) == 1.0 && tzero == U64_OFFSET {
                header.set_internal(key!("TZERO{n}").as_str(), U64_OFFSET_INTEGER);
            } else {
                header.set_internal(key!("TZERO{n}").as_str(), tzero);
            }
        }
        if let Some(tnull) = col.tnull {
            header.set_internal(key!("TNULL{n}").as_str(), tnull);
        }
    }
    merge_header_template(&mut header, template);
    Ok(header)
}

#[derive(Debug)]
struct ColumnLayout {
    row_width: usize,
    tform: String,
}

impl ColumnType {
    fn from_data(data: &ColumnData) -> ColumnType {
        match data {
            ColumnData::Logical(_) => ColumnType::Logical,
            ColumnData::Bytes(_) => ColumnType::Byte,
            ColumnData::I16(_) => ColumnType::I16,
            ColumnData::I32(_) => ColumnType::I32,
            ColumnData::I64(_) => ColumnType::I64,
            ColumnData::F32(_) => ColumnType::F32,
            ColumnData::F64(_) => ColumnType::F64,
            ColumnData::ComplexF32(_) => ColumnType::ComplexF32,
            ColumnData::ComplexF64(_) => ColumnType::ComplexF64,
            ColumnData::Character(_) => ColumnType::Character,
        }
    }

    fn matches(self, data: &ColumnData) -> bool {
        self == ColumnType::from_data(data)
    }

    fn name(self) -> &'static str {
        match self {
            ColumnType::Logical => "logical column data",
            ColumnType::Byte => "byte column data",
            ColumnType::I16 => "i16 column data",
            ColumnType::I32 => "i32 column data",
            ColumnType::I64 => "i64 column data",
            ColumnType::F32 => "f32 column data",
            ColumnType::F64 => "f64 column data",
            ColumnType::ComplexF32 => "complex-f32 column data",
            ColumnType::ComplexF64 => "complex-f64 column data",
            ColumnType::Character => "character column data",
        }
    }

    fn letter(self) -> char {
        match self {
            ColumnType::Logical => 'L',
            ColumnType::Byte => 'B',
            ColumnType::I16 => 'I',
            ColumnType::I32 => 'J',
            ColumnType::I64 => 'K',
            ColumnType::F32 => 'E',
            ColumnType::F64 => 'D',
            ColumnType::ComplexF32 => 'C',
            ColumnType::ComplexF64 => 'M',
            ColumnType::Character => 'A',
        }
    }

    fn elem_size(self) -> usize {
        match self {
            ColumnType::Logical | ColumnType::Byte | ColumnType::Character => 1,
            ColumnType::I16 => 2,
            ColumnType::I32 | ColumnType::F32 => 4,
            ColumnType::I64 | ColumnType::F64 | ColumnType::ComplexF32 => 8,
            ColumnType::ComplexF64 => 16,
        }
    }
}

fn next_vla_heap_len(
    heap_len: usize,
    wide: bool,
    element_count: usize,
    byte_len: usize,
) -> Result<usize> {
    let count = u64::try_from(element_count).map_err(|_| FitsError::DataUnitOverflow)?;
    let offset = u64::try_from(heap_len).map_err(|_| FitsError::DataUnitOverflow)?;
    validate_pq_descriptor(wide, count, offset)?;
    heap_len
        .checked_add(byte_len)
        .ok_or(FitsError::DataUnitOverflow)
}

fn write_vla_descriptor(
    out: &mut [u8],
    main_len: usize,
    descriptor_offset: usize,
    wide: bool,
    element_count: usize,
) -> Result<()> {
    let width = if wide { 16 } else { 8 };
    let count = u64::try_from(element_count).map_err(|_| FitsError::DataUnitOverflow)?;
    let heap_offset = out
        .len()
        .checked_sub(main_len)
        .expect("VLA heap follows the complete main table");
    let heap_offset = u64::try_from(heap_offset).map_err(|_| FitsError::DataUnitOverflow)?;
    write_pq_descriptor(
        &mut out[descriptor_offset..descriptor_offset + width],
        wide,
        count,
        heap_offset,
    )
}

fn validate_column(col: &WriteColumn, nrows: usize) -> Result<ColumnLayout> {
    validate_ascii(&col.name, "binary column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "binary column unit")?;
    }
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let kind = ColumnType::from_data(data);
            validate_binary_metadata(col, kind.letter())?;
            let expected = nrows
                .checked_mul(*repeat)
                .ok_or(FitsError::DataUnitOverflow)?;
            match data {
                ColumnData::Character(values) => {
                    if values.len() != nrows {
                        return Err(FitsError::RowWidthMismatch {
                            computed: values.len(),
                            declared: nrows,
                        });
                    }
                    for value in values {
                        validate_character(value, "binary character cell")?;
                        if value.bytes.len() > *repeat {
                            return Err(FitsError::RowWidthMismatch {
                                computed: value.bytes.len(),
                                declared: *repeat,
                            });
                        }
                    }
                }
                _ if data.element_count() != expected => {
                    return Err(FitsError::RowWidthMismatch {
                        computed: data.element_count(),
                        declared: expected,
                    });
                }
                _ => {}
            }
            validate_tdim(col.tdim.as_deref(), *repeat)?;
            Ok(ColumnLayout {
                row_width: repeat
                    .checked_mul(kind.elem_size())
                    .ok_or(FitsError::DataUnitOverflow)?,
                tform: format!("{repeat}{}", kind.letter()),
            })
        }
        WriteColumnData::Vla { kind, rows, wide } => {
            validate_binary_metadata(col, kind.letter())?;
            if rows.len() != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: rows.len(),
                    declared: nrows,
                });
            }
            let mut max_elements = 0usize;
            for cell in rows {
                debug_assert!(
                    kind.matches(cell),
                    "validated VLA kind must match every cell"
                );
                if let ColumnData::Character(values) = cell {
                    if values.len() > 1 {
                        return Err(FitsError::RowWidthMismatch {
                            computed: values.len(),
                            declared: 1,
                        });
                    }
                    for value in values {
                        validate_character(value, "binary VLA character cell")?;
                    }
                }
                let count = encoded_element_count(*kind, cell)?;
                max_elements = max_elements.max(count);
                validate_vla_tdim(col.tdim.as_deref(), count)?;
            }
            let descriptor = if *wide { 'Q' } else { 'P' };
            Ok(ColumnLayout {
                row_width: if *wide { 16 } else { 8 },
                tform: format!("1{descriptor}{}({max_elements})", kind.letter()),
            })
        }
        WriteColumnData::VlaBits { rows, wide } => {
            validate_binary_metadata(col, 'X')?;
            if rows.len() != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: rows.len(),
                    declared: nrows,
                });
            }
            let mut max_bits = 0usize;
            for bits in rows {
                max_bits = max_bits.max(bits.len());
                validate_vla_tdim(col.tdim.as_deref(), bits.len())?;
            }
            let descriptor = if *wide { 'Q' } else { 'P' };
            Ok(ColumnLayout {
                row_width: if *wide { 16 } else { 8 },
                tform: format!("1{descriptor}X({max_bits})"),
            })
        }
        WriteColumnData::Bits { bytes, bit_count } => {
            validate_binary_metadata(col, 'X')?;
            let row_width = bit_count.div_ceil(8);
            let expected = nrows
                .checked_mul(row_width)
                .ok_or(FitsError::DataUnitOverflow)?;
            if bytes.len() != expected {
                return Err(FitsError::RowWidthMismatch {
                    computed: bytes.len(),
                    declared: expected,
                });
            }
            validate_tdim(col.tdim.as_deref(), *bit_count)?;
            Ok(ColumnLayout {
                row_width,
                tform: format!("{bit_count}X"),
            })
        }
    }
}

fn validate_binary_metadata(col: &WriteColumn, stored_type: char) -> Result<()> {
    validate_scaling(
        col.tscale,
        col.tzero,
        !matches!(stored_type, 'A' | 'L' | 'X'),
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
        return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
    }
    Ok(())
}

fn cell_byte_len(kind: ColumnType, cell: &ColumnData) -> Result<usize> {
    encoded_element_count(kind, cell)?
        .checked_mul(kind.elem_size())
        .ok_or(FitsError::DataUnitOverflow)
}

fn encoded_element_count(kind: ColumnType, cell: &ColumnData) -> Result<usize> {
    debug_assert!(kind.matches(cell), "column kind must match its data");
    if let ColumnData::Character(values) = cell {
        values.iter().try_fold(0usize, |len, value| {
            len.checked_add(value.bytes.len())
                .ok_or(FitsError::DataUnitOverflow)
        })
    } else {
        Ok(cell.element_count())
    }
}

fn validate_tdim(shape: Option<&[usize]>, element_count: usize) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_tdim_shape(shape)?;
    validate_tdim_product(shape, element_count)
}

fn validate_vla_tdim(shape: Option<&[usize]>, element_count: usize) -> Result<()> {
    let Some(shape) = shape else {
        return Ok(());
    };
    validate_tdim_shape(shape)?;
    if element_count == 0 {
        return Ok(());
    }
    validate_tdim_product(shape, element_count)
}

fn validate_tdim_shape(shape: &[usize]) -> Result<()> {
    if shape.is_empty() || shape.contains(&0) {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

fn validate_tdim_product(shape: &[usize], element_count: usize) -> Result<()> {
    let product = shape
        .iter()
        .try_fold(1usize, |product, &len| product.checked_mul(len))
        .ok_or(FitsError::DataUnitOverflow)?;
    if product > element_count {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
    Ok(())
}

/// Append `data[range]` to `out` as big-endian bytes, for every fixed numeric /
/// logical / byte / complex kind. Character and ASCII text values are handled by
/// their format-specific callers, so they are no-ops here.
fn append_cells(out: &mut Vec<u8>, data: &ColumnData, range: Range<usize>) {
    match data {
        ColumnData::Logical(v) => out.extend(v[range].iter().map(|&b| match b {
            Some(true) => b'T',
            Some(false) => b'F',
            None => 0, // §7.3.3 null
        })),
        ColumnData::Bytes(v) => out.extend_from_slice(&v[range]),
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
        ColumnData::Character(_) => {}
    }
}

/// Append a whole column cell (a VLA row's array) to the heap, big-endian.
fn append_be(out: &mut Vec<u8>, cell: &ColumnData) {
    match cell {
        ColumnData::Character(values) => {
            for value in values {
                out.extend_from_slice(&value.bytes);
            }
        }
        _ => append_cells(out, cell, 0..cell.element_count()),
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
            match data {
                ColumnData::Character(values) => {
                    let bytes = &values[r].bytes;
                    out.extend_from_slice(bytes);
                    out.extend(std::iter::repeat_n(b' ', *repeat - bytes.len()));
                }
                data => append_cells(out, data, base..base + *repeat),
            }
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

fn validate_character(value: &CharacterField, context: &'static str) -> Result<()> {
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
pub(crate) mod test_support {
    use crate::writer::table::WriteColumn;

    pub(crate) fn set_scaling(column: &mut WriteColumn, tscale: Option<f64>, tzero: Option<f64>) {
        column.tscale = tscale;
        column.tzero = tzero;
    }
}
