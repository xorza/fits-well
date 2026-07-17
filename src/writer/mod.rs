//! Header and data-unit serialization.
//!
//! The high-level writers — [`FitsWriter::write_image`], `write_table`,
//! `write_ascii_table`, and the compressed forms — synthesize the mandatory header
//! and emit the data unit through one preflighted HDU transaction (which pads to the block grid and
//! embeds `CHECKSUM`/`DATASUM` when enabled), assembling each unit in the writer's
//! reused `scratch`. [`FitsWriter::write_raw_hdu`] is the low-level escape hatch for
//! callers supplying a complete header and already-encoded data unit themselves.

use std::borrow::Cow;
use std::io::Seek;
use std::io::SeekFrom;
use std::io::Write;
use std::ops::Range;

use bitvec::order::Msb0;
use bitvec::slice::BitSlice;
use bitvec::vec::BitVec;
use num_complex::Complex;

use crate::ascii::AsciiColumnData;
use crate::bitpix::Bitpix;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::SPACE_FILL;
use crate::block::ZERO_FILL;
use crate::checksum;
#[cfg(feature = "compression")]
use crate::compress::{Compression, CompressionOptions, compress_image, compress_table};
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::data::U64_OFFSET;
use crate::data::U64_OFFSET_INTEGER;
use crate::data::shape_product;
use crate::endian::extend_be;
use crate::endian::validate_pq_descriptor;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::HduPosition;
use crate::hdu::HduRole;
use crate::hdu::data_extent;
use crate::hdu::validate_table_field_count;
use crate::header::Header;
use crate::header::card::validate_ascii;
use crate::keyword::key;
#[cfg(feature = "compression")]
use crate::table_impl::BinTable;
use crate::table_impl::CharacterField;
use crate::table_impl::ColumnData;

/// 16-zero `CHECKSUM` value written before the real checksum is solved and
/// patched in (Appendix J.1).
const PLACEHOLDER_CHECKSUM: &str = "0000000000000000";

/// Serialize a header unit into reusable storage: every card rendered to 80 bytes,
/// the `END` record, then space padding to the next 2880-byte boundary.
pub(crate) fn render_header(header: &Header, buf: &mut Vec<u8>) -> Result<()> {
    let min_len = header
        .cards
        .len()
        .checked_add(1)
        .and_then(|records| records.checked_mul(CARD_SIZE))
        .ok_or(FitsError::DataUnitOverflow)?;
    buf.clear();
    buf.reserve(min_len);
    for card in &header.cards {
        card.render_into(buf)?;
    }
    let mut end = [SPACE_FILL; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    pad_to_block(buf, SPACE_FILL)
}

/// Round `buf` up to a whole number of 2880-byte blocks using `fill`.
fn pad_to_block(buf: &mut Vec<u8>, fill: u8) -> Result<()> {
    let rem = buf.len() % BLOCK_SIZE;
    if rem != 0 {
        let padded = buf
            .len()
            .checked_add(BLOCK_SIZE - rem)
            .ok_or(FitsError::DataUnitOverflow)?;
        buf.resize(padded, fill);
    }
    Ok(())
}

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

/// One column to write into an ASCII table: nullable typed data, the fixed field
/// width in characters, and the decimal count for floats.
#[derive(Debug, Clone)]
pub struct AsciiWriteColumn {
    pub(crate) name: String,
    pub(crate) unit: Option<String>,
    pub(crate) data: AsciiColumnData,
    pub(crate) width: usize,
    pub(crate) decimals: usize,
    /// Emit `TSCALn`/`TZEROn` (§7.2.2): `data` holds the stored field values and a
    /// reader recovers `TZEROn + TSCALn × field` physically.
    pub(crate) tscale: Option<f64>,
    pub(crate) tzero: Option<f64>,
    /// Emit `TNULLn`, the field text used to write `None` cells (§7.2.4).
    pub(crate) tnull: Option<String>,
}

impl AsciiWriteColumn {
    /// Construct an ASCII-table column. Integer and text columns ignore decimal
    /// precision; float columns default to zero digits after the decimal point.
    pub fn new(name: impl Into<String>, data: AsciiColumnData, width: usize) -> AsciiWriteColumn {
        AsciiWriteColumn {
            name: name.into(),
            unit: None,
            data,
            width,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    pub fn with_unit(mut self, unit: impl Into<String>) -> AsciiWriteColumn {
        self.unit = Some(unit.into());
        self
    }

    pub fn with_decimals(mut self, decimals: usize) -> AsciiWriteColumn {
        self.decimals = decimals;
        self
    }

    pub fn scaled(mut self, tscale: f64, tzero: f64) -> AsciiWriteColumn {
        self.tscale = Some(tscale);
        self.tzero = Some(tzero);
        self
    }

    pub fn with_null(mut self, tnull: impl Into<String>) -> AsciiWriteColumn {
        self.tnull = Some(tnull.into());
        self
    }

    fn row_count(&self) -> usize {
        match &self.data {
            AsciiColumnData::Text(values) => values.len(),
            AsciiColumnData::Integer(values) => values.len(),
            AsciiColumnData::Float(values) => values.len(),
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

/// Validated ASCII-table write value with the same inferred-row workflow as
/// [`TableBuilder`].
#[derive(Debug, Clone, Default)]
pub struct AsciiTableBuilder {
    pub(crate) nrows: Option<usize>,
    pub(crate) columns: Vec<AsciiWriteColumn>,
}

impl AsciiTableBuilder {
    pub fn new() -> AsciiTableBuilder {
        AsciiTableBuilder::default()
    }

    pub fn with_rows(nrows: usize) -> AsciiTableBuilder {
        AsciiTableBuilder {
            nrows: Some(nrows),
            columns: Vec::new(),
        }
    }

    pub fn push(&mut self, column: AsciiWriteColumn) -> Result<&mut Self> {
        let got = column.row_count();
        if let Some(expected) = self.nrows {
            if expected != got {
                return Err(FitsError::TableRowCountMismatch {
                    column: column.name.clone(),
                    expected,
                    got,
                });
            }
        } else {
            self.nrows = Some(got);
        }
        self.columns.push(column);
        Ok(self)
    }

    pub fn column(mut self, column: AsciiWriteColumn) -> Result<AsciiTableBuilder> {
        self.push(column)?;
        Ok(self)
    }

    pub fn explicit(
        nrows: usize,
        columns: impl IntoIterator<Item = AsciiWriteColumn>,
    ) -> Result<AsciiTableBuilder> {
        let mut table = AsciiTableBuilder::with_rows(nrows);
        for column in columns {
            table.push(column)?;
        }
        Ok(table)
    }
}

/// Writes FITS HDUs to a byte sink. The first HDU written becomes the primary
/// array; subsequent images/tables are written as extensions.
#[derive(Debug)]
pub struct FitsWriter<W> {
    sink: W,
    state: WriterState,
    checksum: bool,
    /// Reused buffer the data unit is assembled into before padding + writing, so
    /// writing many HDUs allocates no per-call staging. Each high-level write
    /// `clear`s it, builds the unit, and hands it to the HDU commit path.
    scratch: Vec<u8>,
    /// Reused block-padded header serialization, kept separate because checksum
    /// generation needs the header and data bytes alive at the same time.
    header_scratch: Vec<u8>,
}

/// Incremental image-HDU write for outputs too large to stage in memory. Chunks
/// are typed host-endian samples; the stream encodes and writes each one
/// immediately, then finalizes padding and checksums.
#[derive(Debug)]
pub struct ImageStream<'a, W: Write + Seek> {
    writer: &'a mut FitsWriter<W>,
    header: Header,
    header_offset: u64,
    expected_samples: usize,
    written_samples: usize,
    bitpix: Bitpix,
    checksum: StreamingChecksum,
    finished: bool,
}

#[derive(Debug, Default)]
struct StreamingChecksum {
    sum: u32,
    pending: [u8; 4],
    pending_len: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WriterState {
    Empty,
    Active,
    Failed,
}

impl<W: Write> FitsWriter<W> {
    pub fn new(sink: W) -> Self {
        FitsWriter {
            sink,
            state: WriterState::Empty,
            checksum: false,
            scratch: Vec::new(),
            header_scratch: Vec::new(),
        }
    }

    /// Enable `DATASUM`/`CHECKSUM` integrity keywords on every HDU written through
    /// this writer (§J), including [`FitsWriter::write_raw_hdu`].
    pub fn with_checksums(mut self) -> Self {
        self.checksum = true;
        self
    }

    /// Write one complete raw HDU after validating the header's role and exact
    /// unpadded data length. The block fill is derived from the HDU kind: spaces
    /// for an ASCII table and NULs for every other data unit.
    pub fn write_raw_hdu(&mut self, header: &Header, raw: &[u8]) -> Result<()> {
        self.ensure_writable()?;
        let position = match self.state {
            WriterState::Empty => HduPosition::Primary,
            WriterState::Active => HduPosition::Extension,
            WriterState::Failed => unreachable!("failed writer rejected above"),
        };
        let role = HduRole::from_header(header, position)?;
        let kind = HduKind::classify(header, role)?;
        let extent = data_extent(header, role)?;
        let expected =
            usize::try_from(extent.data_bytes).map_err(|_| FitsError::DataUnitTooLarge {
                bytes: extent.data_bytes,
            })?;
        if raw.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: raw.len(),
            });
        }
        self.scratch.clear();
        self.scratch.extend_from_slice(raw);
        let fill = if kind == HduKind::AsciiTable {
            SPACE_FILL
        } else {
            ZERO_FILL
        };
        self.finish_hdu(header.clone(), fill, false)
    }

    /// Write `image` as the primary HDU (first call) or an `IMAGE` extension
    /// (later calls). The mandatory header is synthesized (`SIMPLE`/`XTENSION`,
    /// `BITPIX`, `NAXISn`, plus `BSCALE`/`BZERO`/`BLANK` when scaling is
    /// non-trivial), followed by the big-endian data unit.
    pub fn write_image(&mut self, image: &Image) -> Result<()> {
        self.write_image_template(image, None)
    }

    /// Write an image while preserving the non-structural cards from `header`.
    /// Mandatory image-layout and checksum cards are regenerated from `image`.
    pub fn write_image_with_header(&mut self, image: &Image, header: &Header) -> Result<()> {
        self.write_image_template(image, Some(header))
    }

    fn write_image_template(&mut self, image: &Image, template: Option<&Header>) -> Result<()> {
        self.ensure_writable()?;
        let expected = image.validate_geometry()?;
        let encoded_len = expected
            .checked_mul(image.samples.bitpix().elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        let header = image_header(image, self.state == WriterState::Empty, template)?;
        self.scratch.clear();
        self.scratch.reserve_exact(encoded_len);
        image.samples.encode_into(&mut self.scratch);
        self.finish_hdu(header, ZERO_FILL, false)
    }

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

    /// Write an ASCII table as a `TABLE` extension (a dataless primary is written
    /// first if needed). Columns are packed left-to-right with no gaps; data is
    /// space-padded per §7.2.3.
    pub fn write_ascii_table(&mut self, table: &AsciiTableBuilder) -> Result<()> {
        self.write_ascii_table_template(table, None)
    }

    /// Write an ASCII table while preserving the non-structural cards from `header`.
    /// Mandatory table-layout and checksum cards are regenerated from `columns`.
    pub fn write_ascii_table_with_header(
        &mut self,
        table: &AsciiTableBuilder,
        header: &Header,
    ) -> Result<()> {
        self.write_ascii_table_template(table, Some(header))
    }

    fn write_ascii_table_template(
        &mut self,
        table: &AsciiTableBuilder,
        template: Option<&Header>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let nrows = table.nrows.unwrap_or(0);
        let columns = &table.columns;
        validate_table_field_count(columns.len())?;
        let mut tbcols = Vec::with_capacity(columns.len());
        let mut row_len = 0usize;
        for col in columns {
            validate_ascii_column(col)?;
            let count = match &col.data {
                AsciiColumnData::Text(values) => values.len(),
                AsciiColumnData::Integer(values) => values.len(),
                AsciiColumnData::Float(values) => values.len(),
            };
            if count != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: count,
                    declared: nrows,
                });
            }
            tbcols.push(row_len.checked_add(1).ok_or(FitsError::DataUnitOverflow)?); // 1-based start column
            row_len = row_len
                .checked_add(col.width)
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        let header = ascii_table_header(nrows, row_len, columns, &tbcols, template)?;
        self.scratch.clear();
        let total_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.scratch.reserve_exact(total_len);
        for r in 0..nrows {
            for col in columns {
                append_ascii_field(&mut self.scratch, col, r)?;
            }
        }
        self.finish_hdu(header, SPACE_FILL, true)
    }

    /// Write `image` as a tiled-compressed `BINTABLE` extension (§10.1), using the
    /// typed codec and shared [`CompressionOptions`]. Requires the `compression`
    /// feature. Integer images support
    /// `GZIP_1`/`GZIP_2`/`RICE_1`/`PLIO_1`/`HCOMPRESS_1`; float images are quantized
    /// (`SUBTRACTIVE_DITHER_1`) and compressed with `GZIP_1`/`GZIP_2`/`RICE_1`.
    /// `HCOMPRESS_1` needs a 2-D tile shape, and every `PLIO_1` mask sample must be
    /// in the lossless `0..=0xFF_FFFF` domain.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image(
        &mut self,
        image: &Image,
        compression: Compression,
        options: &CompressionOptions,
    ) -> Result<()> {
        self.write_compressed_image_template(image, compression, options, None)
    }

    /// Write a tiled-compressed image while preserving the non-structural cards
    /// from `header`. Container, compression, image-layout, and checksum cards are
    /// regenerated from `image` and `options`.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image_with_header(
        &mut self,
        image: &Image,
        compression: Compression,
        options: &CompressionOptions,
        header: &Header,
    ) -> Result<()> {
        self.write_compressed_image_template(image, compression, options, Some(header))
    }

    #[cfg(feature = "compression")]
    fn write_compressed_image_template(
        &mut self,
        image: &Image,
        compression: Compression,
        options: &CompressionOptions,
        template: Option<&Header>,
    ) -> Result<()> {
        self.ensure_writable()?;
        let mut header = compress_image(image, compression, options, &mut self.scratch)?;
        merge_header_template(&mut header, template);
        self.finish_hdu(header, ZERO_FILL, true)
    }

    /// Write a fixed-width `BINTABLE` as a tiled-compressed table (§10.3). `header`
    /// is the original table's header (column metadata is copied from it), `table`
    /// its parsed data, `rows_per_tile` the tile height, and `algo` the per-column
    /// codec (`GZIP_1`/`GZIP_2`/`RICE_1`). Requires the `compression` feature.
    /// The header must describe `table` exactly; original tables with `PCOUNT > 0`
    /// are rejected because this fixed-width path does not retain heap bytes.
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

    fn ensure_writable(&self) -> Result<()> {
        if self.state == WriterState::Failed {
            return Err(FitsError::WriterFailed);
        }
        Ok(())
    }

    /// Finish preflight for one HDU, then commit it and any required automatic
    /// primary without another fallible validation or encoding step between them.
    fn finish_hdu(&mut self, header: Header, fill: u8, automatic_primary: bool) -> Result<()> {
        let rem = self.scratch.len() % BLOCK_SIZE;
        if rem != 0 {
            let padded = self
                .scratch
                .len()
                .checked_add(BLOCK_SIZE - rem)
                .ok_or(FitsError::DataUnitOverflow)?;
            self.scratch.resize(padded, fill);
        }
        prepare_header(
            header,
            &self.scratch,
            self.checksum,
            &mut self.header_scratch,
        )?;

        let primary_header = if automatic_primary && self.state == WriterState::Empty {
            let mut primary_header = Vec::new();
            prepare_header(
                empty_primary_header(),
                &[],
                self.checksum,
                &mut primary_header,
            )?;
            Some(primary_header)
        } else {
            None
        };
        if let Some(primary_header) = primary_header {
            write_prepared(&mut self.sink, &mut self.state, &primary_header, &[])?;
        }
        write_prepared(
            &mut self.sink,
            &mut self.state,
            &self.header_scratch,
            &self.scratch,
        )
    }

    /// Consume the writer and return the underlying sink. HDUs are written eagerly,
    /// so an unbuffered sink (e.g. a `File`) holds the complete file. This does **not**
    /// flush: if the sink is a `BufWriter`, flush it (or rely on its `Drop`) before
    /// trusting the bytes, and check the flush result if you need write errors surfaced.
    pub fn into_inner(self) -> W {
        self.sink
    }
}

impl<W: Write + Seek> FitsWriter<W> {
    /// Begin a large identity-scaled image write. The returned stream must receive
    /// exactly the axis-product sample count and be finished successfully.
    pub fn stream_image(
        &mut self,
        shape: impl Into<Vec<usize>>,
        bitpix: Bitpix,
    ) -> Result<ImageStream<'_, W>> {
        self.stream_image_template(shape.into(), bitpix, Scaling::IDENTITY, None)
    }

    /// Begin a large image write with explicit scaling. Structural and checksum
    /// cards are generated from the supplied geometry, sample type, and scaling.
    pub fn stream_image_scaled(
        &mut self,
        shape: impl Into<Vec<usize>>,
        bitpix: Bitpix,
        scaling: Scaling,
    ) -> Result<ImageStream<'_, W>> {
        self.stream_image_template(shape.into(), bitpix, scaling, None)
    }

    /// Begin a large image write with explicit scaling and an informational header
    /// template. Structural and checksum cards are regenerated.
    pub fn stream_image_with_header(
        &mut self,
        shape: impl Into<Vec<usize>>,
        bitpix: Bitpix,
        scaling: Scaling,
        header: &Header,
    ) -> Result<ImageStream<'_, W>> {
        self.stream_image_template(shape.into(), bitpix, scaling, Some(header))
    }

    fn stream_image_template(
        &mut self,
        shape: Vec<usize>,
        bitpix: Bitpix,
        scaling: Scaling,
        template: Option<&Header>,
    ) -> Result<ImageStream<'_, W>> {
        self.ensure_writable()?;
        scaling.validate(bitpix)?;
        let expected_samples = shape_product(&shape)?;
        let header = image_header_parts(
            &shape,
            bitpix,
            scaling,
            self.state == WriterState::Empty,
            template,
        )?;
        let header_offset = self.sink.stream_position()?;
        let mut initial = header.clone();
        if self.checksum {
            initial.set_internal("DATASUM", "0");
            initial.set_internal("CHECKSUM", PLACEHOLDER_CHECKSUM);
        }
        render_header(&initial, &mut self.header_scratch)?;
        if let Err(error) = self.sink.write_all(&self.header_scratch) {
            self.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        self.state = WriterState::Active;
        Ok(ImageStream {
            writer: self,
            header,
            header_offset,
            expected_samples,
            written_samples: 0,
            bitpix,
            checksum: StreamingChecksum::default(),
            finished: false,
        })
    }
}

impl<W: Write + Seek> ImageStream<'_, W> {
    /// Append one typed chunk. Every chunk must have the stream's `BITPIX` type.
    pub fn write_chunk(&mut self, samples: &ImageData) -> Result<()> {
        if samples.bitpix() != self.bitpix {
            return Err(FitsError::TypeMismatch {
                name: "streaming image chunk".to_string(),
                expected: bitpix_name(self.bitpix),
            });
        }
        let total = self
            .written_samples
            .checked_add(samples.len())
            .ok_or(FitsError::DataUnitOverflow)?;
        if total > self.expected_samples {
            return Err(FitsError::DataSizeMismatch {
                expected: self.expected_samples,
                got: total,
            });
        }
        self.writer.scratch.clear();
        samples.encode_into(&mut self.writer.scratch);
        if let Err(error) = self.writer.sink.write_all(&self.writer.scratch) {
            self.writer.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        self.checksum.update(&self.writer.scratch);
        self.written_samples = total;
        Ok(())
    }

    /// Validate the final sample count, write FITS block padding, and patch checksum
    /// cards in place when enabled.
    pub fn finish(mut self) -> Result<()> {
        if self.written_samples != self.expected_samples {
            return Err(FitsError::DataSizeMismatch {
                expected: self.expected_samples,
                got: self.written_samples,
            });
        }
        let data_bytes = self
            .expected_samples
            .checked_mul(self.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        let padding = (BLOCK_SIZE - data_bytes % BLOCK_SIZE) % BLOCK_SIZE;
        let zeros = [0u8; BLOCK_SIZE];
        if let Err(error) = self.writer.sink.write_all(&zeros[..padding]) {
            self.writer.state = WriterState::Failed;
            return Err(FitsError::Io(error));
        }
        self.checksum.update(&zeros[..padding]);
        debug_assert_eq!(self.checksum.pending_len, 0);

        if self.writer.checksum {
            let end = self.writer.sink.stream_position()?;
            prepare_header_with_data_sum(
                self.header.clone(),
                self.checksum.sum,
                &mut self.writer.header_scratch,
            )?;
            self.writer.sink.seek(SeekFrom::Start(self.header_offset))?;
            if let Err(error) = self.writer.sink.write_all(&self.writer.header_scratch) {
                self.writer.state = WriterState::Failed;
                return Err(FitsError::Io(error));
            }
            self.writer.sink.seek(SeekFrom::Start(end))?;
        }
        self.finished = true;
        Ok(())
    }
}

impl<W: Write + Seek> Drop for ImageStream<'_, W> {
    fn drop(&mut self) {
        if !self.finished {
            self.writer.state = WriterState::Failed;
        }
    }
}

impl StreamingChecksum {
    fn update(&mut self, mut bytes: &[u8]) {
        if self.pending_len != 0 {
            let needed = 4 - self.pending_len;
            let copied = needed.min(bytes.len());
            self.pending[self.pending_len..self.pending_len + copied]
                .copy_from_slice(&bytes[..copied]);
            self.pending_len += copied;
            bytes = &bytes[copied..];
            if self.pending_len == 4 {
                self.sum = checksum::accumulate(&self.pending, self.sum);
                self.pending_len = 0;
            }
        }
        let aligned = bytes.len() / 4 * 4;
        if aligned != 0 {
            self.sum = checksum::accumulate(&bytes[..aligned], self.sum);
        }
        let tail = &bytes[aligned..];
        self.pending[..tail.len()].copy_from_slice(tail);
        self.pending_len = tail.len();
    }
}

fn bitpix_name(bitpix: Bitpix) -> &'static str {
    match bitpix {
        Bitpix::U8 => "u8 image data",
        Bitpix::I16 => "i16 image data",
        Bitpix::I32 => "i32 image data",
        Bitpix::I64 => "i64 image data",
        Bitpix::F32 => "f32 image data",
        Bitpix::F64 => "f64 image data",
    }
}

fn prepare_header(
    header: Header,
    padded_data: &[u8],
    with_checksum: bool,
    out: &mut Vec<u8>,
) -> Result<()> {
    if with_checksum {
        prepare_header_with_data_sum(header, checksum::accumulate(padded_data, 0), out)
    } else {
        render_header(&header, out)
    }
}

fn prepare_header_with_data_sum(
    mut header: Header,
    data_sum: u32,
    out: &mut Vec<u8>,
) -> Result<()> {
    header.set_internal("DATASUM", data_sum.to_string());
    header.set_internal("CHECKSUM", PLACEHOLDER_CHECKSUM);
    render_header(&header, out)?;
    let hdu_sum = checksum::combine(checksum::accumulate(out, 0), data_sum);
    patch_checksum(out, &checksum::encode(hdu_sum, true));
    Ok(())
}

fn write_prepared<W: Write>(
    sink: &mut W,
    state: &mut WriterState,
    header: &[u8],
    data: &[u8],
) -> Result<()> {
    if let Err(error) = sink.write_all(header).and_then(|()| sink.write_all(data)) {
        *state = WriterState::Failed;
        return Err(FitsError::Io(error));
    }
    *state = WriterState::Active;
    Ok(())
}

/// A dataless primary HDU (`NAXIS = 0`), written before extensions when the
/// caller's first HDU is itself an extension.
fn empty_primary_header() -> Header {
    let mut header = Header::new();
    header
        .set_internal("SIMPLE", true)
        .comment_internal("SIMPLE", "file conforms to FITS standard");
    header.set_internal("BITPIX", 8).set_internal("NAXIS", 0);
    header
        .set_internal("EXTEND", true)
        .comment_internal("EXTEND", "extensions follow");
    header
}

/// Image header: the primary array (§4.4.1) when `primary`, else an `IMAGE`
/// extension (§7.1). The two differ only in the prologue (`SIMPLE`+`EXTEND` vs
/// `XTENSION`+`PCOUNT`/`GCOUNT`); the axes and scaling keywords are identical.
fn image_header(image: &Image, primary: bool, template: Option<&Header>) -> Result<Header> {
    image_header_parts(
        &image.shape,
        image.samples.bitpix(),
        image.scaling,
        primary,
        template,
    )
}

fn image_header_parts(
    shape: &[usize],
    bitpix: Bitpix,
    scaling: Scaling,
    primary: bool,
    template: Option<&Header>,
) -> Result<Header> {
    let mut header = Header::new();
    if primary {
        header
            .set_internal("SIMPLE", true)
            .comment_internal("SIMPLE", "file conforms to FITS standard");
        add_image_axes(&mut header, shape, bitpix)?;
        header
            .set_internal("EXTEND", true)
            .comment_internal("EXTEND", "extensions may follow");
    } else {
        header
            .set_internal("XTENSION", "IMAGE")
            .comment_internal("XTENSION", "image extension");
        add_image_axes(&mut header, shape, bitpix)?;
        header.set_internal("PCOUNT", 0).set_internal("GCOUNT", 1);
    }
    scaling.add_to_header(&mut header, bitpix)?;
    merge_header_template(&mut header, template);
    Ok(header)
}

/// `BITPIX`, `NAXIS`, `NAXISn` — the mandatory array-shape keywords, in order.
fn add_image_axes(header: &mut Header, shape: &[usize], bitpix: Bitpix) -> Result<()> {
    if shape.len() > 999 {
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS" });
    }
    header
        .set_internal("BITPIX", bitpix.code())
        .comment_internal("BITPIX", "number of bits per data pixel");
    header
        .set_internal("NAXIS", fits_i64(shape.len())?)
        .comment_internal("NAXIS", "number of data axes");
    for (i, &n) in shape.iter().enumerate() {
        header.set_internal(key!("NAXIS{}", i + 1).as_str(), fits_i64(n)?);
    }
    Ok(())
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

fn validate_scaling(tscale: Option<f64>, tzero: Option<f64>, allowed: bool) -> Result<()> {
    if tscale.is_some_and(|value| !allowed || !value.is_finite()) {
        return Err(FitsError::KeywordOutOfRange { name: "TSCALn" });
    }
    if tzero.is_some_and(|value| !allowed || !value.is_finite()) {
        return Err(FitsError::KeywordOutOfRange { name: "TZEROn" });
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

/// Replace the 16 placeholder bytes of the rendered `CHECKSUM` card's value with
/// the solved value. The value occupies bytes 12–27 (0-based 11–26) of its card.
fn patch_checksum(header_bytes: &mut [u8], encoded: &[u8; 16]) {
    for card in header_bytes.chunks_exact_mut(CARD_SIZE) {
        if &card[..8] == b"CHECKSUM" {
            card[11..27].copy_from_slice(encoded);
            return;
        }
    }
}

fn validate_ascii_column(col: &AsciiWriteColumn) -> Result<()> {
    validate_ascii(&col.name, "ASCII column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "ASCII column unit")?;
    }
    let marker = col.tnull.as_deref().map(str::trim);
    if let Some(marker) = &col.tnull {
        validate_ascii(marker, "ASCII null marker")?;
        if marker.trim().is_empty() || marker.len() > col.width {
            return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
        }
    }
    validate_scaling(
        col.tscale,
        col.tzero,
        !matches!(&col.data, AsciiColumnData::Text(_)),
    )?;
    let has_null = match &col.data {
        AsciiColumnData::Text(values) => values.iter().any(Option::is_none),
        AsciiColumnData::Integer(values) => values.iter().any(Option::is_none),
        AsciiColumnData::Float(values) => values.iter().any(Option::is_none),
    };
    if has_null && marker.is_none() {
        return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
    }
    Ok(())
}

fn validate_ascii_field_width(
    col: &AsciiWriteColumn,
    row: usize,
    minimum_width: usize,
) -> Result<()> {
    if minimum_width > col.width {
        return Err(FitsError::AsciiFieldTooWide {
            column: col.name.clone(),
            row,
            width: col.width,
            minimum_width,
        });
    }
    Ok(())
}

#[derive(Debug)]
struct AsciiField<'a> {
    text: Cow<'a, str>,
    left_aligned: bool,
}

fn ascii_field<'a>(col: &'a AsciiWriteColumn, row: usize) -> Result<AsciiField<'a>> {
    let field = match &col.data {
        AsciiColumnData::Text(values) => AsciiField {
            text: match &values[row] {
                Some(value) => Cow::Borrowed(value),
                None => Cow::Borrowed(
                    col.tnull
                        .as_deref()
                        .expect("null ASCII cells require a validated TNULLn marker"),
                ),
            },
            left_aligned: true,
        },
        AsciiColumnData::Integer(values) => AsciiField {
            text: match values[row] {
                Some(value) => Cow::Owned(value.to_string()),
                None => Cow::Borrowed(
                    col.tnull
                        .as_deref()
                        .expect("null ASCII cells require a validated TNULLn marker"),
                ),
            },
            left_aligned: values[row].is_none(),
        },
        AsciiColumnData::Float(values) => AsciiField {
            text: match values[row] {
                Some(value) => {
                    if !value.is_finite() {
                        return Err(FitsError::InvalidValue {
                            card: "ASCII float cells must be finite; use None for null".to_string(),
                        });
                    }
                    let sign_width = usize::from(value.is_sign_negative());
                    let minimum_width = if col.decimals == 0 {
                        1 + sign_width
                    } else {
                        if col.decimals > col.width {
                            validate_ascii_field_width(col, row, col.decimals)?;
                        }
                        col.decimals
                            .checked_add(2 + sign_width)
                            .ok_or(FitsError::DataUnitOverflow)?
                    };
                    validate_ascii_field_width(col, row, minimum_width)?;
                    Cow::Owned(format!("{:.*}", col.decimals, value))
                }
                None => Cow::Borrowed(
                    col.tnull
                        .as_deref()
                        .expect("null ASCII cells require a validated TNULLn marker"),
                ),
            },
            left_aligned: values[row].is_none(),
        },
    };
    validate_ascii_field_width(col, row, field.text.len())?;
    Ok(field)
}

fn validate_ascii_null_collision(value: &str, marker: Option<&str>) -> Result<()> {
    if marker == Some(value) {
        Err(FitsError::InvalidValue {
            card: "ASCII value equals its TNULLn marker".to_string(),
        })
    } else {
        Ok(())
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

/// `TABLE` extension header (§7.2) for the given columns and computed `TBCOLn`s.
fn ascii_table_header(
    nrows: usize,
    row_len: usize,
    columns: &[AsciiWriteColumn],
    tbcols: &[usize],
    template: Option<&Header>,
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .comment_internal("XTENSION", "ASCII table extension");
    header.set_internal("BITPIX", 8).set_internal("NAXIS", 2);
    header
        .set_internal("NAXIS1", fits_i64(row_len)?)
        .comment_internal("NAXIS1", "width of table in characters");
    header
        .set_internal("NAXIS2", fits_i64(nrows)?)
        .comment_internal("NAXIS2", "number of rows");
    header.set_internal("PCOUNT", 0).set_internal("GCOUNT", 1);
    header
        .set_internal("TFIELDS", fits_i64(columns.len())?)
        .comment_internal("TFIELDS", "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set_internal(key!("TBCOL{n}").as_str(), fits_i64(tbcols[i])?);
        header.set_internal(key!("TFORM{n}").as_str(), ascii_tform(col));
        header.set_internal(key!("TTYPE{n}").as_str(), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set_internal(key!("TUNIT{n}").as_str(), unit.as_str());
        }
        if let Some(tscale) = col.tscale {
            header.set_internal(key!("TSCAL{n}").as_str(), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set_internal(key!("TZERO{n}").as_str(), tzero);
        }
        if let Some(tnull) = &col.tnull {
            header.set_internal(key!("TNULL{n}").as_str(), tnull.as_str());
        }
    }
    merge_header_template(&mut header, template);
    Ok(header)
}

fn merge_header_template(header: &mut Header, template: Option<&Header>) {
    let Some(template) = template else {
        return;
    };
    header.append_filtered_from(template, |keyword| !is_structural_keyword(keyword));
}

fn is_structural_keyword(keyword: &str) -> bool {
    if matches!(
        keyword,
        "SIMPLE"
            | "XTENSION"
            | "BITPIX"
            | "NAXIS"
            | "PCOUNT"
            | "GCOUNT"
            | "EXTEND"
            | "GROUPS"
            | "BLOCKED"
            | "BSCALE"
            | "BZERO"
            | "BLANK"
            | "CHECKSUM"
            | "DATASUM"
            | "THEAP"
            | "TFIELDS"
            | "ZIMAGE"
            | "ZTABLE"
            | "ZTILELEN"
            | "ZNAXIS"
            | "ZPCOUNT"
            | "ZGCOUNT"
            | "ZSIMPLE"
            | "ZTENSION"
            | "ZEXTEND"
            | "ZBLOCKED"
            | "ZTHEAP"
            | "ZHEAPPTR"
            | "ZHECKSUM"
            | "ZDATASUM"
            | "ZCMPTYPE"
            | "ZBITPIX"
            | "ZQUANTIZ"
            | "ZDITHER0"
            | "ZBLANK"
            | "ZMASKCMP"
    ) {
        return true;
    }
    [
        "NAXIS", "TFORM", "TTYPE", "TUNIT", "TDIM", "TSCAL", "TZERO", "TNULL", "TBCOL", "ZFORM",
        "ZCTYP", "ZNAXIS", "ZTILE", "ZNAME", "ZVAL",
    ]
    .iter()
    .any(|prefix| indexed_keyword(keyword, prefix))
}

fn indexed_keyword(keyword: &str, prefix: &str) -> bool {
    keyword.strip_prefix(prefix).is_some_and(|suffix| {
        keyword.len() <= 8
            && !suffix.is_empty()
            && !suffix.starts_with('0')
            && suffix.bytes().all(|byte| byte.is_ascii_digit())
    })
}

fn fits_i64(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}

fn ascii_tform(col: &AsciiWriteColumn) -> String {
    match col.data {
        AsciiColumnData::Text(_) => format!("A{}", col.width),
        AsciiColumnData::Integer(_) => format!("I{}", col.width),
        AsciiColumnData::Float(_) => format!("F{}.{}", col.width, col.decimals),
    }
}

fn append_ascii_field(out: &mut Vec<u8>, col: &AsciiWriteColumn, r: usize) -> Result<()> {
    let field = ascii_field(col, r)?;
    let marker = col.tnull.as_deref().map(str::trim);
    match &col.data {
        AsciiColumnData::Text(values) => {
            if let Some(value) = &values[r] {
                validate_ascii(value, "ASCII text cell")?;
                validate_ascii_null_collision(value.trim(), marker)?;
            }
            debug_assert!(field.left_aligned);
        }
        AsciiColumnData::Integer(values) => {
            if values[r].is_some() {
                validate_ascii_null_collision(&field.text, marker)?;
            }
        }
        AsciiColumnData::Float(values) => {
            if values[r].is_some() {
                validate_ascii_null_collision(&field.text, marker)?;
            }
        }
    }
    let bytes = field.text.as_bytes();
    let pad = col.width - bytes.len();
    if field.left_aligned {
        out.extend_from_slice(bytes);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(bytes);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
