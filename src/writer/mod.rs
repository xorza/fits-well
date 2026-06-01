//! Header and data-unit serialization.
//!
//! Header units and pre-encoded data units round-trip through this layer today.
//! Typed *encoding* — building a conforming header from an [`crate::Image`] or
//! table and emitting the inverse `BSCALE`/`BZERO` scaling — is the next layer;
//! it will sit on top of [`FitsWriter::write_data_unit`].

use std::io::Write;

use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::SPACE_FILL;
use crate::block::ZERO_FILL;
use crate::checksum;
use crate::data::Image;
use crate::endian::extend_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::table::ColumnData;

/// 16-zero `CHECKSUM` value written before the real checksum is solved and
/// patched in (Appendix J.1).
const PLACEHOLDER_CHECKSUM: &str = "0000000000000000";

/// Serialize a header unit: every card rendered to 80 bytes, the `END` record,
/// then space padding to the next 2880-byte boundary.
pub(crate) fn render_header(header: &Header) -> Vec<u8> {
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
    buf
}

/// Round `buf` up to a whole number of 2880-byte blocks using `fill`.
pub(crate) fn pad_to_block(buf: &mut Vec<u8>, fill: u8) {
    let rem = buf.len() % BLOCK_SIZE;
    if rem != 0 {
        buf.resize(buf.len() + (BLOCK_SIZE - rem), fill);
    }
}

/// One column to write into a binary table: its name, optional unit, data, and
/// the number of elements per row (`repeat`). For [`ColumnData::Text`], `repeat`
/// is the fixed character width of the field.
///
/// When `vla` is `Some`, the column is written as a variable-length (`P`) array:
/// each entry is one row's array and `data`/`repeat` are ignored (the element
/// type comes from the first row, or from `data` if there are no rows).
#[derive(Debug, Clone)]
pub struct WriteColumn {
    pub name: String,
    pub unit: Option<String>,
    pub data: ColumnData,
    pub repeat: usize,
    pub vla: Option<Vec<ColumnData>>,
    /// `TDIMn` array shape (fastest axis first) for a multidimensional column.
    pub tdim: Option<Vec<usize>>,
    /// Use 64-bit `Q` descriptors instead of 32-bit `P` for a VLA column.
    pub wide: bool,
    /// Bit count for an `X` (bit-array) column; `data` is the packed bytes.
    pub bits: Option<usize>,
    /// `TSCALn`/`TZEROn` to emit: `data` holds the stored values, and a reader's
    /// `read_column_physical` recovers `TZEROn + TSCALn × stored`.
    pub tscale: Option<f64>,
    pub tzero: Option<f64>,
    /// `TNULLn`: the stored integer marking an undefined element.
    pub tnull: Option<i64>,
}

impl WriteColumn {
    /// A fixed-width column of `repeat` elements per row.
    pub fn fixed(name: impl Into<String>, data: ColumnData, repeat: usize) -> WriteColumn {
        WriteColumn {
            name: name.into(),
            unit: None,
            data,
            repeat,
            vla: None,
            tdim: None,
            wide: false,
            bits: None,
            tscale: None,
            tzero: None,
            tnull: None,
        }
    }

    /// A variable-length (`P`, or `Q` via [`WriteColumn::wide`]) column: `rows[r]`
    /// is row `r`'s array.
    pub fn vla(name: impl Into<String>, rows: Vec<ColumnData>) -> WriteColumn {
        // The element type tag for `data` is the first row's kind, or empty bytes.
        let tag = rows
            .first()
            .cloned()
            .unwrap_or(ColumnData::Bytes(Vec::new()));
        WriteColumn {
            data: tag,
            repeat: 0,
            vla: Some(rows),
            ..WriteColumn::fixed(name, ColumnData::Bytes(Vec::new()), 0)
        }
    }

    /// An `X` (bit-array) column of `nbits` bits per row, `data` the packed bytes
    /// (`ceil(nbits/8)` per row). `repeat` is the byte width so the bytes pack
    /// directly; `TFORMn` is rendered as `<nbits>X`.
    pub fn bits(name: impl Into<String>, data: ColumnData, nbits: usize) -> WriteColumn {
        WriteColumn {
            bits: Some(nbits),
            ..WriteColumn::fixed(name, data, nbits.div_ceil(8))
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
        self.wide = true;
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
}

impl<W: Write> FitsWriter<W> {
    pub fn new(sink: W) -> Self {
        FitsWriter {
            sink,
            has_primary: false,
            checksum: false,
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
        self.sink.write_all(&render_header(header))?;
        Ok(())
    }

    /// Write a pre-encoded data unit, padding to a block with `fill` — NUL for
    /// most data, ASCII space for ASCII-table data (§3.1).
    pub fn write_data_unit(&mut self, raw: &[u8], fill: u8) -> Result<()> {
        self.sink.write_all(raw)?;
        let rem = raw.len() % BLOCK_SIZE;
        if rem != 0 {
            self.sink.write_all(&vec![fill; BLOCK_SIZE - rem])?;
        }
        Ok(())
    }

    /// Write `image` as the primary HDU (first call) or an `IMAGE` extension
    /// (later calls). The mandatory header is synthesized (`SIMPLE`/`XTENSION`,
    /// `BITPIX`, `NAXISn`, plus `BSCALE`/`BZERO`/`BLANK` when scaling is
    /// non-trivial), followed by the big-endian data unit.
    pub fn write_image(&mut self, image: &Image) -> Result<()> {
        let expected = if image.shape.is_empty() {
            0
        } else {
            image.shape.iter().product::<usize>()
        };
        assert_eq!(
            image.samples.len(),
            expected,
            "image sample count must match the shape product"
        );
        let header = if self.has_primary {
            image_extension_header(image)
        } else {
            primary_image_header(image)
        };
        self.has_primary = true;
        self.write_hdu(header, image.samples.encode(), ZERO_FILL)
    }

    /// Write a binary table as a `BINTABLE` extension. A dataless primary HDU is
    /// written automatically first if nothing has been written yet (a table can
    /// never be the primary HDU). Fixed-width and variable-length (`P`) columns
    /// are both supported — VLA columns write a heap after the main table.
    pub fn write_table(&mut self, nrows: usize, columns: &[WriteColumn]) -> Result<()> {
        self.ensure_primary()?;
        let mut row_len = 0;
        for col in columns {
            row_len += check_column(col, nrows)?;
        }
        // Build the heap (row-major) and per-VLA-column descriptors first, so the
        // main table can carry the `P` (count, offset) pairs.
        let mut heap: Vec<u8> = Vec::new();
        let mut descs: Vec<Vec<(u64, u64)>> = vec![Vec::new(); columns.len()];
        for r in 0..nrows {
            for (ci, col) in columns.iter().enumerate() {
                if let Some(rows) = &col.vla {
                    let cell = &rows[r];
                    descs[ci].push((count_of(cell) as u64, heap.len() as u64));
                    append_be(&mut heap, cell);
                }
            }
        }
        // Main table: fixed cells inline, VLA columns as `P` descriptors (consumed
        // per column in the same row order they were built).
        let mut data = Vec::with_capacity(nrows * row_len + heap.len());
        let mut cursor = vec![0usize; columns.len()];
        for r in 0..nrows {
            for (ci, col) in columns.iter().enumerate() {
                if col.vla.is_some() {
                    let (n, o) = descs[ci][cursor[ci]];
                    cursor[ci] += 1;
                    push_vla_descriptor(&mut data, col.wide, n, o);
                } else {
                    pack_cell(&mut data, col, r);
                }
            }
        }
        data.extend_from_slice(&heap);
        let header = bintable_header(nrows, row_len, columns, heap.len());
        self.write_hdu(header, data, ZERO_FILL)
    }

    /// Write an ASCII table as a `TABLE` extension (a dataless primary is written
    /// first if needed). Columns are packed left-to-right with no gaps; data is
    /// space-padded per §7.2.3.
    pub fn write_ascii_table(&mut self, nrows: usize, columns: &[AsciiWriteColumn]) -> Result<()> {
        self.ensure_primary()?;
        let mut tbcols = Vec::with_capacity(columns.len());
        let mut row_len = 0;
        for col in columns {
            if ascii_count(&col.data)? != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: ascii_count(&col.data)?,
                    declared: nrows,
                });
            }
            tbcols.push(row_len + 1); // 1-based start column
            row_len += col.width;
        }
        let header = ascii_table_header(nrows, row_len, columns, &tbcols);
        let mut data = Vec::with_capacity(nrows * row_len);
        for r in 0..nrows {
            for col in columns {
                format_ascii_field(&mut data, col, r);
            }
        }
        self.write_hdu(header, data, SPACE_FILL)
    }

    /// Write `image` as a tiled-compressed `BINTABLE` extension (§10.1), using the
    /// `ZCMPTYPE` codec and the given tile shape (empty ⇒ row tiling). Requires the
    /// `compression` feature. Integer images support `GZIP_1`/`GZIP_2`/`RICE_1`/
    /// `PLIO_1`/`HCOMPRESS_1`; float images are quantized (`SUBTRACTIVE_DITHER_1`)
    /// and compressed with `GZIP_1`/`GZIP_2`/`RICE_1`. `HCOMPRESS_1` needs a 2-D
    /// tile shape, and `PLIO_1` a non-negative (mask) image.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image(
        &mut self,
        image: &Image,
        cmptype: &str,
        tile_shape: &[usize],
    ) -> Result<()> {
        self.write_compressed_image_lossy(image, cmptype, tile_shape, 0)
    }

    /// Like [`FitsWriter::write_compressed_image`] but with an `HCOMPRESS_1`
    /// quantization `scale` (`0` = lossless; larger = more lossy compression). The
    /// scale is ignored by the other codecs.
    #[cfg(feature = "compression")]
    pub fn write_compressed_image_lossy(
        &mut self,
        image: &Image,
        cmptype: &str,
        tile_shape: &[usize],
        scale: i32,
    ) -> Result<()> {
        self.ensure_primary()?;
        let (header, data) = crate::compress::encode_image(image, cmptype, tile_shape, scale)?;
        self.write_hdu(header, data, ZERO_FILL)
    }

    /// Write a fixed-width `BINTABLE` as a tiled-compressed table (§10.3). `header`
    /// is the original table's header (column metadata is copied from it), `table`
    /// its parsed data, `rows_per_tile` the tile height, and `algo` the per-column
    /// codec (`GZIP_1`/`GZIP_2`/`RICE_1`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn write_compressed_table(
        &mut self,
        header: &Header,
        table: &crate::table::BinTable,
        rows_per_tile: usize,
        algo: &str,
    ) -> Result<()> {
        self.ensure_primary()?;
        let (h, data) = crate::compress::compress_table(header, table, rows_per_tile, algo)?;
        self.write_hdu(h, data, ZERO_FILL)
    }

    /// Write a dataless primary HDU if none has been written yet, so subsequent
    /// extensions are well-formed.
    fn ensure_primary(&mut self) -> Result<()> {
        if !self.has_primary {
            self.write_hdu(empty_primary_header(), Vec::new(), ZERO_FILL)?;
            self.has_primary = true;
        }
        Ok(())
    }

    /// Render and write one HDU (header + block-padded data), embedding
    /// `DATASUM`/`CHECKSUM` when checksums are enabled.
    fn write_hdu(&mut self, mut header: Header, mut data: Vec<u8>, fill: u8) -> Result<()> {
        pad_to_block(&mut data, fill);
        if self.checksum {
            header.set("DATASUM", checksum::accumulate(&data, 0).to_string());
            header.set("CHECKSUM", PLACEHOLDER_CHECKSUM);
        }
        let mut header_bytes = render_header(&header);
        if self.checksum {
            // Re-sum with the zero placeholder, then encode the value that forces
            // the whole-HDU checksum to negative zero, and patch it in place.
            let hdu_sum = checksum::accumulate(&data, checksum::accumulate(&header_bytes, 0));
            patch_checksum(&mut header_bytes, &checksum::encode(hdu_sum, true));
        }
        self.sink.write_all(&header_bytes)?;
        self.sink.write_all(&data)?;
        Ok(())
    }

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

/// Primary-array header for an image (§4.4.1).
fn primary_image_header(image: &Image) -> Header {
    let mut header = Header::new();
    header
        .set("SIMPLE", true)
        .comment("SIMPLE", "file conforms to FITS standard");
    add_image_axes(&mut header, image);
    header
        .set("EXTEND", true)
        .comment("EXTEND", "extensions may follow");
    add_scaling(&mut header, image);
    header
}

/// `IMAGE` extension header (§7.1).
fn image_extension_header(image: &Image) -> Header {
    let mut header = Header::new();
    header
        .set("XTENSION", "IMAGE")
        .comment("XTENSION", "image extension");
    add_image_axes(&mut header, image);
    header.set("PCOUNT", 0).set("GCOUNT", 1);
    add_scaling(&mut header, image);
    header
}

/// `BITPIX`, `NAXIS`, `NAXISn` — the mandatory array-shape keywords, in order.
fn add_image_axes(header: &mut Header, image: &Image) {
    header
        .set("BITPIX", image.samples.bitpix().code())
        .comment("BITPIX", "number of bits per data pixel");
    header
        .set("NAXIS", image.shape.len() as i64)
        .comment("NAXIS", "number of data axes");
    for (i, &n) in image.shape.iter().enumerate() {
        header.set(&format!("NAXIS{}", i + 1), n as i64);
    }
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
    heap_len: usize,
) -> Header {
    let mut header = Header::new();
    header
        .set("XTENSION", "BINTABLE")
        .comment("XTENSION", "binary table extension");
    header.set("BITPIX", 8).set("NAXIS", 2);
    header
        .set("NAXIS1", row_len as i64)
        .comment("NAXIS1", "width of table in bytes");
    header
        .set("NAXIS2", nrows as i64)
        .comment("NAXIS2", "number of rows");
    header.set("PCOUNT", heap_len as i64).set("GCOUNT", 1);
    header
        .set("TFIELDS", columns.len() as i64)
        .comment("TFIELDS", "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set(&format!("TFORM{n}"), tform_of(col));
        header.set(&format!("TTYPE{n}"), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set(&format!("TUNIT{n}"), unit.as_str());
        }
        if let Some(shape) = &col.tdim {
            let dims: Vec<String> = shape.iter().map(|d| d.to_string()).collect();
            header.set(&format!("TDIM{n}"), format!("({})", dims.join(",")));
        }
        if let Some(tscale) = col.tscale {
            header.set(&format!("TSCAL{n}"), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set(&format!("TZERO{n}"), tzero);
        }
        if let Some(tnull) = col.tnull {
            header.set(&format!("TNULL{n}"), tnull);
        }
    }
    header
}

/// The `(code letter, element byte size)` for a column's data kind.
fn column_code(data: &ColumnData) -> (char, usize) {
    match data {
        ColumnData::Logical(_) => ('L', 1),
        ColumnData::Bytes(_) => ('B', 1),
        ColumnData::I16(_) => ('I', 2),
        ColumnData::I32(_) => ('J', 4),
        ColumnData::I64(_) => ('K', 8),
        ColumnData::F32(_) => ('E', 4),
        ColumnData::F64(_) => ('D', 8),
        ColumnData::ComplexF32(_) => ('C', 8),
        ColumnData::ComplexF64(_) => ('M', 16),
        ColumnData::Text(_) => ('A', 1),
    }
}

fn tform_of(col: &WriteColumn) -> String {
    let (code, _) = column_code(&col.data);
    if let Some(nbits) = col.bits {
        return format!("{nbits}X");
    }
    match &col.vla {
        // `1P<code>(maxnelem)`, or `1Q…` for 64-bit descriptors.
        Some(rows) => {
            let max = rows.iter().map(count_of).max().unwrap_or(0);
            let p = if col.wide { 'Q' } else { 'P' };
            format!("1{p}{code}({max})")
        }
        None => format!("{}{}", col.repeat, code),
    }
}

/// Validate a column against `nrows` and return its per-row byte width.
fn check_column(col: &WriteColumn, nrows: usize) -> Result<usize> {
    let (_, elem) = column_code(&col.data);
    if let Some(rows) = &col.vla {
        if rows.len() != nrows {
            return Err(FitsError::RowWidthMismatch {
                computed: rows.len(),
                declared: nrows,
            });
        }
        // `P` descriptor = two 32-bit ints; `Q` = two 64-bit.
        return Ok(if col.wide { 16 } else { 8 });
    }
    let mismatch = || FitsError::RowWidthMismatch {
        computed: count_of(&col.data),
        declared: nrows * col.repeat,
    };
    match &col.data {
        ColumnData::Text(v) => {
            if v.len() != nrows {
                return Err(FitsError::RowWidthMismatch {
                    computed: v.len(),
                    declared: nrows,
                });
            }
            Ok(col.repeat) // field width in bytes
        }
        _ => {
            if count_of(&col.data) != nrows * col.repeat {
                return Err(mismatch());
            }
            Ok(col.repeat * elem)
        }
    }
}

/// Number of elements (or strings) in a column's data.
/// Encode a VLA descriptor — element count and heap byte offset — as a big-endian
/// `Q` (64-bit, `wide`) or `P` (32-bit) pair. The values are carried as `u64` up
/// to this point so a `Q` column can address a heap or count beyond the 4 GiB the
/// 32-bit `P` form allows (truncating earlier would defeat the point of `Q`).
fn push_vla_descriptor(data: &mut Vec<u8>, wide: bool, count: u64, offset: u64) {
    if wide {
        data.extend_from_slice(&(count as i64).to_be_bytes());
        data.extend_from_slice(&(offset as i64).to_be_bytes());
    } else {
        data.extend_from_slice(&(count as i32).to_be_bytes());
        data.extend_from_slice(&(offset as i32).to_be_bytes());
    }
}

fn count_of(data: &ColumnData) -> usize {
    match data {
        ColumnData::Logical(v) => v.len(),
        ColumnData::Bytes(v) => v.len(),
        ColumnData::I16(v) => v.len(),
        ColumnData::I32(v) => v.len(),
        ColumnData::I64(v) => v.len(),
        ColumnData::F32(v) => v.len(),
        ColumnData::F64(v) => v.len(),
        ColumnData::ComplexF32(v) => v.len(),
        ColumnData::ComplexF64(v) => v.len(),
        ColumnData::Text(v) => v.len(),
    }
}

/// Append a whole column cell (a VLA row's array) to the heap, big-endian.
fn append_be(out: &mut Vec<u8>, cell: &ColumnData) {
    match cell {
        ColumnData::Logical(v) => out.extend(v.iter().map(|&b| match b {
            Some(true) => b'T',
            Some(false) => b'F',
            None => 0, // §7.3.3 null
        })),
        ColumnData::Bytes(v) => out.extend_from_slice(v),
        ColumnData::I16(v) => extend_be(out, v, i16::to_be_bytes),
        ColumnData::I32(v) => extend_be(out, v, i32::to_be_bytes),
        ColumnData::I64(v) => extend_be(out, v, i64::to_be_bytes),
        ColumnData::F32(v) => extend_be(out, v, f32::to_be_bytes),
        ColumnData::F64(v) => extend_be(out, v, f64::to_be_bytes),
        ColumnData::ComplexF32(v) => {
            for &(re, im) in v {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
        ColumnData::ComplexF64(v) => {
            for &(re, im) in v {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
        // Character VLAs (`PA`) concatenate the strings' bytes.
        ColumnData::Text(v) => {
            for s in v {
                out.extend_from_slice(s.as_bytes());
            }
        }
    }
}

fn pack_cell(out: &mut Vec<u8>, col: &WriteColumn, r: usize) {
    let rep = col.repeat;
    let base = r * rep;
    match &col.data {
        ColumnData::Logical(v) => {
            for k in 0..rep {
                out.push(match v[base + k] {
                    Some(true) => b'T',
                    Some(false) => b'F',
                    None => 0, // §7.3.3 null
                });
            }
        }
        ColumnData::Bytes(v) => out.extend_from_slice(&v[base..base + rep]),
        ColumnData::I16(v) => extend_be(out, &v[base..base + rep], i16::to_be_bytes),
        ColumnData::I32(v) => extend_be(out, &v[base..base + rep], i32::to_be_bytes),
        ColumnData::I64(v) => extend_be(out, &v[base..base + rep], i64::to_be_bytes),
        ColumnData::F32(v) => extend_be(out, &v[base..base + rep], f32::to_be_bytes),
        ColumnData::F64(v) => extend_be(out, &v[base..base + rep], f64::to_be_bytes),
        ColumnData::ComplexF32(v) => {
            for &(re, im) in &v[base..base + rep] {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
        ColumnData::ComplexF64(v) => {
            for &(re, im) in &v[base..base + rep] {
                out.extend_from_slice(&re.to_be_bytes());
                out.extend_from_slice(&im.to_be_bytes());
            }
        }
        // `A`: the row's string, space-padded or truncated to the field width.
        ColumnData::Text(v) => {
            let bytes = v[r].as_bytes();
            let n = bytes.len().min(rep);
            out.extend_from_slice(&bytes[..n]);
            out.extend(std::iter::repeat_n(b' ', rep - n));
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

/// `TABLE` extension header (§7.2) for the given columns and computed `TBCOLn`s.
fn ascii_table_header(
    nrows: usize,
    row_len: usize,
    columns: &[AsciiWriteColumn],
    tbcols: &[usize],
) -> Header {
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .comment("XTENSION", "ASCII table extension");
    header.set("BITPIX", 8).set("NAXIS", 2);
    header
        .set("NAXIS1", row_len as i64)
        .comment("NAXIS1", "width of table in characters");
    header
        .set("NAXIS2", nrows as i64)
        .comment("NAXIS2", "number of rows");
    header.set("PCOUNT", 0).set("GCOUNT", 1);
    header
        .set("TFIELDS", columns.len() as i64)
        .comment("TFIELDS", "number of columns");
    for (i, col) in columns.iter().enumerate() {
        let n = i + 1;
        header.set(&format!("TBCOL{n}"), tbcols[i] as i64);
        header.set(&format!("TFORM{n}"), ascii_tform(col));
        header.set(&format!("TTYPE{n}"), col.name.as_str());
        if let Some(unit) = &col.unit {
            header.set(&format!("TUNIT{n}"), unit.as_str());
        }
        if let Some(tscale) = col.tscale {
            header.set(&format!("TSCAL{n}"), tscale);
        }
        if let Some(tzero) = col.tzero {
            header.set(&format!("TZERO{n}"), tzero);
        }
        if let Some(tnull) = &col.tnull {
            header.set(&format!("TNULL{n}"), tnull.as_str());
        }
    }
    header
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
        ColumnData::Text(v) => (v[r].clone(), true),
        ColumnData::I64(v) => (v[r].to_string(), false),
        // A non-finite cell has no §7.2.5 real representation: write the TNULLn
        // marker if set, else a blank field (which a reader takes as 0).
        ColumnData::F64(v) if !v[r].is_finite() => (col.tnull.clone().unwrap_or_default(), false),
        ColumnData::F64(v) => (format!("{:.*}", col.decimals, v[r]), false),
        _ => (String::new(), true),
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
