//! Header and data-unit serialization.
//!
//! The high-level writers — [`FitsWriter::write_image`], `write_table`,
//! `write_ascii_table`, and the compressed forms — synthesize the mandatory header
//! and emit the data unit through `write_hdu` (which pads to the block grid and
//! embeds `CHECKSUM`/`DATASUM` when enabled), assembling each unit in the writer's
//! reused `scratch`. [`FitsWriter::write_header`] / [`FitsWriter::write_data_unit`]
//! are the low-level escape hatches for callers driving the layout themselves.

use std::borrow::Cow;
use std::io::Write;
use std::ops::Range;

use num_complex::Complex;

use crate::allocation;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::SPACE_FILL;
use crate::block::ZERO_FILL;
use crate::checksum;
#[cfg(feature = "compression")]
use crate::compress::{CompressOptions, compress_image, compress_table};
use crate::data::Image;
use crate::data::shape_product;
use crate::endian::extend_be;
use crate::endian::push_pq_descriptor;
use crate::endian::validate_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::header::value::Value;
use crate::keyword::key;
#[cfg(feature = "compression")]
use crate::table::BinTable;
use crate::table::ColumnData;

/// 16-zero `CHECKSUM` value written before the real checksum is solved and
/// patched in (Appendix J.1).
const PLACEHOLDER_CHECKSUM: &str = "0000000000000000";

/// Serialize a header unit: every card rendered to 80 bytes, the `END` record,
/// then space padding to the next 2880-byte boundary.
pub(crate) fn render_header(header: &Header) -> Result<Vec<u8>> {
    for entry in header.iter() {
        if let Some(Value::Text(text)) = entry.value {
            validate_ascii(text, "header text value")?;
        }
        if let Some(comment) = entry.comment {
            validate_ascii(comment, "header comment")?;
        }
    }
    let mut buf = Vec::with_capacity((header.cards.len() + 1) * CARD_SIZE);
    for card in &header.cards {
        for record in card.render_records() {
            buf.extend_from_slice(&record);
        }
    }
    let mut end = [SPACE_FILL; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    pad_to_block(&mut buf, SPACE_FILL);
    Ok(buf)
}

/// Round `buf` up to a whole number of 2880-byte blocks using `fill`.
fn pad_to_block(buf: &mut Vec<u8>, fill: u8) {
    let rem = buf.len() % BLOCK_SIZE;
    if rem != 0 {
        buf.resize(buf.len() + (BLOCK_SIZE - rem), fill);
    }
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
    Text,
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
    /// A fixed-width column of `repeat` elements per row.
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

    /// A variable-length `P` column. `kind` states the heap element type even when
    /// `rows` is empty; [`WriteColumn::wide`] changes the descriptors to `Q`.
    pub fn vla(name: impl Into<String>, kind: ColumnType, rows: Vec<ColumnData>) -> WriteColumn {
        assert!(
            rows.iter().all(|row| kind.matches(row)),
            "VLA column cells must match the declared ColumnType"
        );
        WriteColumn {
            name: name.into(),
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
        }
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
    pub fn wide(mut self) -> WriteColumn {
        let WriteColumnData::Vla { wide, .. } = &mut self.values else {
            panic!("WriteColumn::wide requires a VLA column");
        };
        *wide = true;
        self
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
}

/// One column to write into an ASCII table: data (`Text`/`I64`/`F64` only), the
/// fixed field width in characters, and the decimal count for floats.
#[derive(Debug, Clone)]
pub struct AsciiWriteColumn {
    pub name: String,
    pub unit: Option<String>,
    pub data: ColumnData,
    pub width: usize,
    pub decimals: usize,
    /// Emit `TSCALn`/`TZEROn` (§7.2.2): `data` holds the stored field values and a
    /// reader recovers `TZEROn + TSCALn × field` physically.
    pub tscale: Option<f64>,
    pub tzero: Option<f64>,
    /// Emit `TNULLn`, the field text marking an undefined value (§7.2.4). A
    /// non-finite `F64` cell is written as this marker (or a blank field — which
    /// reads back as 0 per §7.2.5 — when no marker is set).
    pub tnull: Option<String>,
}

/// Writes FITS HDUs to a byte sink. The first HDU written becomes the primary
/// array; subsequent images/tables are written as extensions.
#[derive(Debug)]
pub struct FitsWriter<W> {
    sink: W,
    has_primary: bool,
    checksum: bool,
    /// Reused buffer the data unit is assembled into before padding + writing, so
    /// writing many HDUs allocates no per-call staging. Each high-level write
    /// `clear`s it, builds the unit, and hands it to [`FitsWriter::write_hdu`].
    scratch: Vec<u8>,
}

impl<W: Write> FitsWriter<W> {
    pub fn new(sink: W) -> Self {
        FitsWriter {
            sink,
            has_primary: false,
            checksum: false,
            scratch: Vec::new(),
        }
    }

    /// Enable `DATASUM`/`CHECKSUM` integrity keywords on every HDU written through
    /// the high-level [`FitsWriter::write_image`] / `write_table` / `write_ascii_table`
    /// methods (§J).
    pub fn with_checksums(mut self) -> Self {
        self.checksum = true;
        self
    }

    /// Write a header unit (cards + `END` + block padding).
    pub fn write_header(&mut self, header: &Header) -> Result<()> {
        self.sink.write_all(&render_header(header)?)?;
        Ok(())
    }

    /// Write a pre-encoded data unit, padding to a block with `fill` — NUL for
    /// most data, ASCII space for ASCII-table data (§3.1).
    pub fn write_data_unit(&mut self, raw: &[u8], fill: u8) -> Result<()> {
        self.sink.write_all(raw)?;
        let rem = raw.len() % BLOCK_SIZE;
        if rem != 0 {
            let padding = [fill; BLOCK_SIZE];
            self.sink.write_all(&padding[..BLOCK_SIZE - rem])?;
        }
        Ok(())
    }

    /// Write `image` as the primary HDU (first call) or an `IMAGE` extension
    /// (later calls). The mandatory header is synthesized (`SIMPLE`/`XTENSION`,
    /// `BITPIX`, `NAXISn`, plus `BSCALE`/`BZERO`/`BLANK` when scaling is
    /// non-trivial), followed by the big-endian data unit.
    pub fn write_image(&mut self, image: &Image) -> Result<()> {
        let expected = shape_product(&image.shape)?;
        assert_eq!(
            image.samples.len(),
            expected,
            "image sample count must match the shape product"
        );
        let encoded_len = expected
            .checked_mul(image.samples.bitpix().elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        let header = image_header(image, !self.has_primary)?;
        self.has_primary = true;
        self.scratch.clear();
        allocation::try_reserve_exact(&mut self.scratch, encoded_len)?;
        image.samples.encode_into(&mut self.scratch);
        self.write_hdu(header, ZERO_FILL)
    }

    /// Write a binary table as a `BINTABLE` extension. A dataless primary HDU is
    /// written automatically first if nothing has been written yet (a table can
    /// never be the primary HDU). Fixed-width and variable-length (`P`) columns
    /// are both supported — VLA columns write a heap after the main table.
    pub fn write_table(&mut self, nrows: usize, columns: &[WriteColumn]) -> Result<()> {
        let mut layouts = Vec::new();
        allocation::try_reserve_exact(&mut layouts, columns.len())?;
        let mut row_len = 0usize;
        for col in columns {
            let layout = validate_column(col, nrows)?;
            row_len = row_len
                .checked_add(layout.row_width)
                .ok_or(FitsError::DataUnitOverflow)?;
            layouts.push(layout);
        }
        // Build the heap (row-major) and the VLA descriptors first, so the main table
        // can carry the `P`/`Q` (count, offset) pairs. Descriptors are recorded in the
        // same row-major, column order the main-table pass emits them, so a single flat
        // queue (drained below) stays aligned without per-column bookkeeping.
        let descriptor_count = nrows
            .checked_mul(
                columns
                    .iter()
                    .filter(|col| matches!(&col.values, WriteColumnData::Vla { .. }))
                    .count(),
            )
            .ok_or(FitsError::DataUnitOverflow)?;
        let heap_len = columns
            .iter()
            .filter_map(|col| match &col.values {
                WriteColumnData::Vla { rows, .. } => Some(rows.as_slice()),
                _ => None,
            })
            .flatten()
            .try_fold(0usize, |len, cell| {
                len.checked_add(cell_byte_len(cell)?)
                    .ok_or(FitsError::DataUnitOverflow)
            })?;
        let mut heap = Vec::new();
        allocation::try_reserve_exact(&mut heap, heap_len)?;
        let mut descs = Vec::new();
        allocation::try_reserve_exact(&mut descs, descriptor_count)?;
        for r in 0..nrows {
            for col in columns {
                if let WriteColumnData::Vla { kind, rows, wide } = &col.values {
                    let cell = &rows[r];
                    let descriptor = Descriptor {
                        count: encoded_element_count(*kind, cell)? as u64,
                        offset: heap.len() as u64,
                        wide: *wide,
                    };
                    validate_pq_descriptor(descriptor.wide, descriptor.count, descriptor.offset)?;
                    descs.push(descriptor);
                    append_be(&mut heap, cell);
                }
            }
        }
        self.ensure_primary()?;
        // Main table: fixed cells inline, VLA columns as `P`/`Q` descriptors drained
        // in the same row-major order they were built. Built into the reused scratch,
        // with the heap appended after.
        self.scratch.clear();
        let main_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let total_len = main_len
            .checked_add(heap.len())
            .ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_reserve_exact(&mut self.scratch, total_len)?;
        let mut descs = descs.into_iter();
        for r in 0..nrows {
            for col in columns {
                match &col.values {
                    WriteColumnData::Vla { .. } => {
                        let descriptor = descs.next().expect("one descriptor per VLA cell");
                        push_pq_descriptor(
                            &mut self.scratch,
                            descriptor.wide,
                            descriptor.count,
                            descriptor.offset,
                        )?;
                    }
                    _ => pack_cell(&mut self.scratch, col, r),
                }
            }
        }
        self.scratch.extend_from_slice(&heap);
        let header = bintable_header(nrows, row_len, columns, &layouts, heap.len())?;
        self.write_hdu(header, ZERO_FILL)
    }

    /// Write an ASCII table as a `TABLE` extension (a dataless primary is written
    /// first if needed). Columns are packed left-to-right with no gaps; data is
    /// space-padded per §7.2.3.
    pub fn write_ascii_table(&mut self, nrows: usize, columns: &[AsciiWriteColumn]) -> Result<()> {
        let mut tbcols = Vec::new();
        allocation::try_reserve_exact(&mut tbcols, columns.len())?;
        let mut row_len = 0usize;
        for col in columns {
            validate_ascii_column(col)?;
            let count = ascii_count(&col.data)?;
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
        let header = ascii_table_header(nrows, row_len, columns, &tbcols)?;
        self.scratch.clear();
        let total_len = nrows
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        self.ensure_primary()?;
        allocation::try_reserve_exact(&mut self.scratch, total_len)?;
        for r in 0..nrows {
            for col in columns {
                format_ascii_field(&mut self.scratch, col, r);
            }
        }
        self.write_hdu(header, SPACE_FILL)
    }

    /// Write `image` as a tiled-compressed `BINTABLE` extension (§10.1), using the
    /// `ZCMPTYPE` codec and the given [`CompressOptions`] (tile shape, gzip level,
    /// HCOMPRESS scale, float quantization level — each used only by the codecs it
    /// applies to). Requires the `compression` feature. Integer images support
    /// `GZIP_1`/`GZIP_2`/`RICE_1`/`PLIO_1`/`HCOMPRESS_1`; float images are quantized
    /// (`SUBTRACTIVE_DITHER_1`) and compressed with `GZIP_1`/`GZIP_2`/`RICE_1`.
    /// `HCOMPRESS_1` needs a 2-D tile shape, and `PLIO_1` a non-negative (mask) image.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image(
        &mut self,
        image: &Image,
        cmptype: &str,
        options: &CompressOptions,
    ) -> Result<()> {
        self.ensure_primary()?;
        // The codec assembles the compressed data unit directly into the reused
        // scratch and hands back just the header.
        let header = compress_image(image, cmptype, options, &mut self.scratch)?;
        self.write_hdu(header, ZERO_FILL)
    }

    /// Write a fixed-width `BINTABLE` as a tiled-compressed table (§10.3). `header`
    /// is the original table's header (column metadata is copied from it), `table`
    /// its parsed data, `rows_per_tile` the tile height, and `algo` the per-column
    /// codec (`GZIP_1`/`GZIP_2`/`RICE_1`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn write_compressed_table(
        &mut self,
        header: &Header,
        table: &BinTable,
        rows_per_tile: usize,
        algo: &str,
    ) -> Result<()> {
        self.ensure_primary()?;
        let zheader = compress_table(header, table, rows_per_tile, algo, &mut self.scratch)?;
        self.write_hdu(zheader, ZERO_FILL)
    }

    /// Write a dataless primary HDU if none has been written yet, so subsequent
    /// extensions are well-formed.
    fn ensure_primary(&mut self) -> Result<()> {
        if !self.has_primary {
            self.scratch.clear();
            self.write_hdu(empty_primary_header(), ZERO_FILL)?;
            self.has_primary = true;
        }
        Ok(())
    }

    /// Render and write one HDU: the unpadded data unit the caller has assembled in
    /// `self.scratch`, padded to a block and framed by the header (with
    /// `DATASUM`/`CHECKSUM` embedded when checksums are enabled).
    ///
    /// Takes the data via the reused `scratch` field rather than an owned argument,
    /// so the high-level writers build into one buffer that survives across HDUs.
    fn write_hdu(&mut self, mut header: Header, fill: u8) -> Result<()> {
        let rem = self.scratch.len() % BLOCK_SIZE;
        if rem != 0 {
            let padded = self
                .scratch
                .len()
                .checked_add(BLOCK_SIZE - rem)
                .ok_or(FitsError::DataUnitOverflow)?;
            allocation::try_resize(&mut self.scratch, padded, fill)?;
        }
        let data_sum = if self.checksum {
            let sum = checksum::accumulate(&self.scratch, 0);
            header.set("DATASUM", sum.to_string());
            header.set("CHECKSUM", PLACEHOLDER_CHECKSUM);
            Some(sum)
        } else {
            None
        };
        let mut header_bytes = render_header(&header)?;
        if let Some(data_sum) = data_sum {
            // Re-sum with the zero placeholder, then encode the value that forces
            // the whole-HDU checksum to negative zero, and patch it in place.
            let hdu_sum = checksum::combine(checksum::accumulate(&header_bytes, 0), data_sum);
            patch_checksum(&mut header_bytes, &checksum::encode(hdu_sum, true));
        }
        self.sink.write_all(&header_bytes)?;
        self.sink.write_all(&self.scratch)?;
        Ok(())
    }

    /// Consume the writer and return the underlying sink. HDUs are written eagerly,
    /// so an unbuffered sink (e.g. a `File`) holds the complete file. This does **not**
    /// flush: if the sink is a `BufWriter`, flush it (or rely on its `Drop`) before
    /// trusting the bytes, and check the flush result if you need write errors surfaced.
    pub fn into_inner(self) -> W {
        self.sink
    }
}

/// A dataless primary HDU (`NAXIS = 0`), written before extensions when the
/// caller's first HDU is itself an extension.
fn empty_primary_header() -> Header {
    let mut header = Header::new();
    header
        .set("SIMPLE", true)
        .comment("SIMPLE", "file conforms to FITS standard");
    header.set("BITPIX", 8).set("NAXIS", 0);
    header
        .set("EXTEND", true)
        .comment("EXTEND", "extensions follow");
    header
}

/// Image header: the primary array (§4.4.1) when `primary`, else an `IMAGE`
/// extension (§7.1). The two differ only in the prologue (`SIMPLE`+`EXTEND` vs
/// `XTENSION`+`PCOUNT`/`GCOUNT`); the axes and scaling keywords are identical.
fn image_header(image: &Image, primary: bool) -> Result<Header> {
    let mut header = Header::new();
    if primary {
        header
            .set("SIMPLE", true)
            .comment("SIMPLE", "file conforms to FITS standard");
        add_image_axes(&mut header, image)?;
        header
            .set("EXTEND", true)
            .comment("EXTEND", "extensions may follow");
    } else {
        header
            .set("XTENSION", "IMAGE")
            .comment("XTENSION", "image extension");
        add_image_axes(&mut header, image)?;
        header.set("PCOUNT", 0).set("GCOUNT", 1);
    }
    add_scaling(&mut header, image);
    Ok(header)
}

/// `BITPIX`, `NAXIS`, `NAXISn` — the mandatory array-shape keywords, in order.
fn add_image_axes(header: &mut Header, image: &Image) -> Result<()> {
    if image.shape.len() > 999 {
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS" });
    }
    header
        .set("BITPIX", image.samples.bitpix().code())
        .comment("BITPIX", "number of bits per data pixel");
    header
        .set("NAXIS", fits_i64(image.shape.len())?)
        .comment("NAXIS", "number of data axes");
    for (i, &n) in image.shape.iter().enumerate() {
        header.set(key!("NAXIS{}", i + 1).as_str(), fits_i64(n)?);
    }
    Ok(())
}

/// Emit `BZERO`/`BSCALE`/`BLANK` only when scaling carries information beyond the
/// identity map.
fn add_scaling(header: &mut Header, image: &Image) {
    if !image.scaling.is_identity() {
        header.set("BZERO", image.scaling.bzero);
        header.set("BSCALE", image.scaling.bscale);
    }
    // §4.4.2.5: BLANK applies only to integer images (positive BITPIX).
    if let Some(blank) = image.scaling.blank
        && image.samples.bitpix().is_integer()
    {
        header.set("BLANK", blank);
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
        .set("XTENSION", "BINTABLE")
        .comment("XTENSION", "binary table extension");
    header.set("BITPIX", 8).set("NAXIS", 2);
    header
        .set("NAXIS1", fits_i64(row_len)?)
        .comment("NAXIS1", "width of table in bytes");
    header
        .set("NAXIS2", fits_i64(nrows)?)
        .comment("NAXIS2", "number of rows");
    header.set("PCOUNT", fits_i64(heap_len)?).set("GCOUNT", 1);
    header
        .set("TFIELDS", fits_i64(columns.len())?)
        .comment("TFIELDS", "number of columns");
    for (i, (col, layout)) in columns.iter().zip(layouts).enumerate() {
        let n = i + 1;
        header.set(key!("TFORM{n}").as_str(), layout.tform.as_str());
        header.set(key!("TTYPE{n}").as_str(), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set(key!("TUNIT{n}").as_str(), unit.as_str());
        }
        if let Some(shape) = &col.tdim {
            let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            header.set(key!("TDIM{n}").as_str(), format!("({})", dims.join(",")));
        }
        if let Some(tscale) = col.tscale {
            header.set(key!("TSCAL{n}").as_str(), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set(key!("TZERO{n}").as_str(), tzero);
        }
        if let Some(tnull) = col.tnull {
            header.set(key!("TNULL{n}").as_str(), tnull);
        }
    }
    Ok(header)
}

#[derive(Debug)]
struct ColumnLayout {
    row_width: usize,
    tform: String,
}

#[derive(Debug, Clone, Copy)]
struct Descriptor {
    count: u64,
    offset: u64,
    wide: bool,
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
            ColumnData::Text(_) => ColumnType::Text,
        }
    }

    fn matches(self, data: &ColumnData) -> bool {
        self == ColumnType::from_data(data)
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
            ColumnType::Text => 'A',
        }
    }

    fn elem_size(self) -> usize {
        match self {
            ColumnType::Logical | ColumnType::Byte | ColumnType::Text => 1,
            ColumnType::I16 => 2,
            ColumnType::I32 | ColumnType::F32 => 4,
            ColumnType::I64 | ColumnType::F64 | ColumnType::ComplexF32 => 8,
            ColumnType::ComplexF64 => 16,
        }
    }
}

fn validate_column(col: &WriteColumn, nrows: usize) -> Result<ColumnLayout> {
    validate_ascii(&col.name, "binary column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "binary column unit")?;
    }
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let kind = ColumnType::from_data(data);
            let expected = nrows
                .checked_mul(*repeat)
                .ok_or(FitsError::DataUnitOverflow)?;
            match data {
                ColumnData::Text(values) => {
                    if values.len() != nrows {
                        return Err(FitsError::RowWidthMismatch {
                            computed: values.len(),
                            declared: nrows,
                        });
                    }
                    for value in values {
                        validate_ascii(value, "binary text cell")?;
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
            if rows.len() != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: rows.len(),
                    declared: nrows,
                });
            }
            let mut max_elements = 0usize;
            for cell in rows {
                assert!(
                    kind.matches(cell),
                    "validated VLA kind must match every cell"
                );
                if let ColumnData::Text(values) = cell {
                    for value in values {
                        validate_ascii(value, "binary VLA text cell")?;
                    }
                }
                let count = encoded_element_count(*kind, cell)?;
                max_elements = max_elements.max(count);
                validate_tdim(col.tdim.as_deref(), count)?;
            }
            let descriptor = if *wide { 'Q' } else { 'P' };
            Ok(ColumnLayout {
                row_width: if *wide { 16 } else { 8 },
                tform: format!("1{descriptor}{}({max_elements})", kind.letter()),
            })
        }
        WriteColumnData::Bits { bytes, bit_count } => {
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

fn cell_byte_len(cell: &ColumnData) -> Result<usize> {
    let kind = ColumnType::from_data(cell);
    encoded_element_count(kind, cell)?
        .checked_mul(kind.elem_size())
        .ok_or(FitsError::DataUnitOverflow)
}

fn encoded_element_count(kind: ColumnType, cell: &ColumnData) -> Result<usize> {
    assert!(kind.matches(cell), "column kind must match its data");
    if let ColumnData::Text(values) = cell {
        values.iter().try_fold(0usize, |len, value| {
            len.checked_add(value.len())
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
    if shape.is_empty() || shape.contains(&0) {
        return Err(FitsError::KeywordOutOfRange { name: "TDIMn" });
    }
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
/// logical / byte / complex kind. `Text` is handled by the two callers (a heap cell
/// concatenates the strings, a main-table cell space-pads one row to its field
/// width), so it is a no-op here.
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
        ColumnData::Text(_) => {} // strings are caller-specific (see the doc)
    }
}

/// Append a whole column cell (a VLA row's array) to the heap, big-endian.
fn append_be(out: &mut Vec<u8>, cell: &ColumnData) {
    match cell {
        // Character VLAs (`PA`) concatenate the strings' bytes.
        ColumnData::Text(v) => {
            for s in v {
                out.extend_from_slice(s.as_bytes());
            }
        }
        _ => append_cells(out, cell, 0..cell.element_count()),
    }
}

fn pack_cell(out: &mut Vec<u8>, col: &WriteColumn, r: usize) {
    match &col.values {
        WriteColumnData::Fixed { data, repeat } => {
            let base = r * *repeat;
            match data {
                ColumnData::Text(values) => {
                    let bytes = values[r].as_bytes();
                    let count = bytes.len().min(*repeat);
                    out.extend_from_slice(&bytes[..count]);
                    out.extend(std::iter::repeat_n(b' ', *repeat - count));
                }
                data => append_cells(out, data, base..base + *repeat),
            }
        }
        WriteColumnData::Bits { bytes, bit_count } => {
            let width = bit_count.div_ceil(8);
            let start = r * width;
            out.extend_from_slice(&bytes[start..start + width]);
        }
        WriteColumnData::Vla { .. } => unreachable!("VLA cells are descriptors"),
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

/// Number of rows implied by an ASCII column (`Text`/`I64`/`F64` only).
fn ascii_count(data: &ColumnData) -> Result<usize> {
    match data {
        ColumnData::Text(v) => Ok(v.len()),
        ColumnData::I64(v) => Ok(v.len()),
        ColumnData::F64(v) => Ok(v.len()),
        _ => Err(FitsError::InvalidValue {
            card: "ASCII table column must be Text, I64, or F64".to_string(),
        }),
    }
}

fn validate_ascii_column(col: &AsciiWriteColumn) -> Result<()> {
    validate_ascii(&col.name, "ASCII column name")?;
    if let Some(unit) = &col.unit {
        validate_ascii(unit, "ASCII column unit")?;
    }
    if let Some(marker) = &col.tnull {
        validate_ascii(marker, "ASCII null marker")?;
        if marker.is_empty() || marker.len() > col.width {
            return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
        }
    }
    match &col.data {
        ColumnData::Text(values) => {
            for value in values {
                validate_ascii(value, "ASCII text cell")?;
            }
        }
        ColumnData::F64(values)
            if values.iter().any(|value| !value.is_finite()) && col.tnull.is_none() =>
        {
            return Err(FitsError::KeywordOutOfRange { name: "TNULLn" });
        }
        _ => {}
    }
    Ok(())
}

fn validate_ascii(text: &str, context: &'static str) -> Result<()> {
    if text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
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
) -> Result<Header> {
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .comment("XTENSION", "ASCII table extension");
    header.set("BITPIX", 8).set("NAXIS", 2);
    header
        .set("NAXIS1", fits_i64(row_len)?)
        .comment("NAXIS1", "width of table in characters");
    header
        .set("NAXIS2", fits_i64(nrows)?)
        .comment("NAXIS2", "number of rows");
    header.set("PCOUNT", 0).set("GCOUNT", 1);
    header
        .set("TFIELDS", fits_i64(columns.len())?)
        .comment("TFIELDS", "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set(key!("TBCOL{n}").as_str(), fits_i64(tbcols[i])?);
        header.set(key!("TFORM{n}").as_str(), ascii_tform(col));
        header.set(key!("TTYPE{n}").as_str(), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set(key!("TUNIT{n}").as_str(), unit.as_str());
        }
        if let Some(tscale) = col.tscale {
            header.set(key!("TSCAL{n}").as_str(), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set(key!("TZERO{n}").as_str(), tzero);
        }
        if let Some(tnull) = &col.tnull {
            header.set(key!("TNULL{n}").as_str(), tnull.as_str());
        }
    }
    Ok(header)
}

fn fits_i64(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}

fn ascii_tform(col: &AsciiWriteColumn) -> String {
    match col.data {
        ColumnData::Text(_) => format!("A{}", col.width),
        ColumnData::I64(_) => format!("I{}", col.width),
        ColumnData::F64(_) => format!("F{}.{}", col.width, col.decimals),
        _ => format!("A{}", col.width), // unreachable: validated in ascii_count
    }
}

/// Format row `r` of an ASCII column into exactly `width` bytes (space-padded;
/// overflow becomes `*` fill per §7.2.5).
fn format_ascii_field(out: &mut Vec<u8>, col: &AsciiWriteColumn, r: usize) {
    let (text, left) = match &col.data {
        ColumnData::Text(values) => (Cow::Borrowed(values[r].as_str()), true),
        ColumnData::I64(values) => (Cow::Owned(values[r].to_string()), false),
        ColumnData::F64(values) if !values[r].is_finite() => (
            Cow::Borrowed(
                col.tnull
                    .as_deref()
                    .expect("non-finite ASCII cells require a validated null marker"),
            ),
            false,
        ),
        ColumnData::F64(values) => (Cow::Owned(format!("{:.*}", col.decimals, values[r])), false),
        _ => unreachable!("ASCII column type was validated"),
    };
    let bytes = text.as_bytes();
    if bytes.len() > col.width {
        out.extend(std::iter::repeat_n(b'*', col.width));
        return;
    }
    let pad = col.width - bytes.len();
    if left {
        out.extend_from_slice(bytes);
        out.extend(std::iter::repeat_n(b' ', pad));
    } else {
        out.extend(std::iter::repeat_n(b' ', pad));
        out.extend_from_slice(bytes);
    }
}

#[cfg(test)]
mod tests;
