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
#[derive(Debug, Clone)]
pub struct WriteColumn {
    pub name: String,
    pub unit: Option<String>,
    pub data: ColumnData,
    pub repeat: usize,
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
    /// never be the primary HDU). Fixed-width columns only.
    pub fn write_table(&mut self, nrows: usize, columns: &[WriteColumn]) -> Result<()> {
        self.ensure_primary()?;
        let mut row_len = 0;
        for col in columns {
            row_len += check_column(col, nrows)?;
        }
        let header = bintable_header(nrows, row_len, columns);
        self.write_hdu(header, pack_rows(nrows, row_len, columns), ZERO_FILL)
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
    if let Some(blank) = image.scaling.blank {
        header.set("BLANK", blank);
    }
}

/// `BINTABLE` extension header (§7.3.1) for the given columns.
fn bintable_header(nrows: usize, row_len: usize, columns: &[WriteColumn]) -> Header {
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
    header.set("PCOUNT", 0).set("GCOUNT", 1);
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
    format!("{}{}", col.repeat, code)
}

/// Validate a column against `nrows` and return its per-row byte width.
fn check_column(col: &WriteColumn, nrows: usize) -> Result<usize> {
    let (_, elem) = column_code(&col.data);
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

/// Pack the table into `nrows × row_len` big-endian bytes, row-major.
fn pack_rows(nrows: usize, row_len: usize, columns: &[WriteColumn]) -> Vec<u8> {
    let mut out = Vec::with_capacity(nrows * row_len);
    for r in 0..nrows {
        for col in columns {
            pack_cell(&mut out, col, r);
        }
    }
    out
}

fn pack_cell(out: &mut Vec<u8>, col: &WriteColumn, r: usize) {
    let rep = col.repeat;
    let base = r * rep;
    match &col.data {
        ColumnData::Logical(v) => {
            for k in 0..rep {
                out.push(if v[base + k] { b'T' } else { b'F' });
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

fn extend_be<const N: usize, T: Copy>(out: &mut Vec<u8>, values: &[T], conv: fn(T) -> [u8; N]) {
    for &v in values {
        out.extend_from_slice(&conv(v));
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
mod tests {
    use super::*;
    use crate::block::ZERO_FILL;
    use crate::data::{ImageData, Scaling};
    use crate::hdu::HduKind;
    use crate::reader::FitsReader;
    use crate::table::ColumnData;
    use std::io::Cursor;

    fn write_to_vec(image: &Image) -> Vec<u8> {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_image(image).unwrap();
        w.into_inner().into_inner()
    }

    fn identity() -> Scaling {
        Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        }
    }

    #[test]
    fn writes_a_multi_hdu_image_file() {
        let primary = Image {
            shape: vec![2, 2],
            samples: ImageData::U8(vec![1, 2, 3, 4]),
            scaling: identity(),
        };
        let ext = Image {
            shape: vec![3],
            samples: ImageData::I16(vec![10, 20, 30]),
            scaling: identity(),
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_image(&primary).unwrap();
        w.write_image(&ext).unwrap(); // second image ⇒ IMAGE extension
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

        assert_eq!(r.hdus.len(), 2);
        assert_eq!(r.hdus[0].kind, HduKind::Primary);
        assert_eq!(r.hdus[1].kind, HduKind::Image);
        assert_eq!(
            r.read_image(0).unwrap().samples,
            ImageData::U8(vec![1, 2, 3, 4])
        );
        assert_eq!(
            r.read_image(1).unwrap().samples,
            ImageData::I16(vec![10, 20, 30])
        );
    }

    #[test]
    fn writes_and_reads_back_a_binary_table() {
        let columns = vec![
            WriteColumn {
                name: "NOSTA".into(),
                unit: None,
                data: ColumnData::I32(vec![1, 2, 3]),
                repeat: 1,
            },
            WriteColumn {
                name: "XYZ".into(),
                unit: Some("m".into()),
                data: ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
                repeat: 3, // 3 floats per row
            },
            WriteColumn {
                name: "NAME".into(),
                unit: None,
                data: ColumnData::Text(vec!["AB".into(), "CDE".into(), "F".into()]),
                repeat: 3, // 3-char field
            },
        ];
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_table(3, &columns).unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

        // A dataless primary is auto-written before the table extension.
        assert_eq!(r.hdus.len(), 2);
        assert_eq!(r.hdus[0].kind, HduKind::Primary);
        assert_eq!(r.hdus[0].header.naxis().unwrap(), 0);
        assert_eq!(r.hdus[1].kind, HduKind::BinTable);

        let t = r.read_table(1).unwrap();
        assert_eq!(t.nrows, 3);
        assert_eq!(t.columns.len(), 3);
        assert_eq!(t.columns[0].name.as_deref(), Some("NOSTA"));
        assert_eq!(t.columns[1].unit.as_deref(), Some("m"));
        assert_eq!(t.read_column(0).unwrap(), ColumnData::I32(vec![1, 2, 3]));
        assert_eq!(
            t.read_column(1).unwrap(),
            ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
        );
        assert_eq!(
            t.read_column(2).unwrap(),
            ColumnData::Text(vec!["AB".into(), "CDE".into(), "F".into()])
        );
    }

    fn header(lines: &[&str]) -> Header {
        let mut buf = Vec::new();
        for line in lines {
            let mut card = [b' '; CARD_SIZE];
            card[..line.len()].copy_from_slice(line.as_bytes());
            buf.extend_from_slice(&card);
        }
        let mut end = [b' '; CARD_SIZE];
        end[..3].copy_from_slice(b"END");
        buf.extend_from_slice(&end);
        Header::parse(&buf).unwrap()
    }

    #[test]
    fn pad_to_block_rounds_up_with_the_fill_byte() {
        let mut empty = Vec::new();
        pad_to_block(&mut empty, ZERO_FILL);
        assert_eq!(empty.len(), 0);

        let mut one = vec![1u8];
        pad_to_block(&mut one, ZERO_FILL);
        assert_eq!(one.len(), BLOCK_SIZE);
        assert_eq!(one[0], 1);
        assert!(one[1..].iter().all(|&b| b == ZERO_FILL));

        let mut exact = vec![7u8; BLOCK_SIZE];
        pad_to_block(&mut exact, ZERO_FILL);
        assert_eq!(exact.len(), BLOCK_SIZE);

        let mut over = vec![0u8; BLOCK_SIZE + 1];
        pad_to_block(&mut over, ZERO_FILL);
        assert_eq!(over.len(), 2 * BLOCK_SIZE);
    }

    #[test]
    fn rendered_header_is_block_aligned_and_ends_in_end_then_spaces() {
        let unit = render_header(&header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
        ]));
        assert_eq!(unit.len() % BLOCK_SIZE, 0);
        assert_eq!(unit.len(), BLOCK_SIZE); // 4 cards fit in one block

        // The 4th card (index 3) is END, followed by space padding.
        assert_eq!(&unit[3 * CARD_SIZE..3 * CARD_SIZE + 3], b"END");
        assert!(unit[4 * CARD_SIZE..].iter().all(|&b| b == SPACE_FILL));
    }

    #[test]
    fn header_round_trips_through_render_and_parse() {
        let original = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                  -32",
            "NAXIS   =                    2",
            "NAXIS1  =                  100",
            "NAXIS2  =                   50",
            "OBJECT  = 'O''Brien'",
            "COMMENT  a remark",
        ]);
        let reparsed = Header::parse(&render_header(&original)).unwrap();
        assert_eq!(reparsed.cards, original.cards);
    }

    #[test]
    fn image_round_trips_through_write_image_and_read_image() {
        let image = Image {
            shape: vec![2, 3],
            samples: ImageData::I16(vec![1, -2, 3, -4, 5, -6]),
            scaling: Scaling {
                bscale: 1.0,
                bzero: 0.0,
                blank: None,
            },
        };
        let bytes = write_to_vec(&image);
        assert_eq!(bytes.len(), 2 * BLOCK_SIZE); // one header block + one data block

        let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
        assert_eq!(r.hdus.len(), 1);
        assert_eq!(r.hdus[0].kind, HduKind::Primary);
        let back = r.read_image(0).unwrap();
        assert_eq!(back.shape, vec![2, 3]);
        assert_eq!(back.samples, ImageData::I16(vec![1, -2, 3, -4, 5, -6]));
    }

    #[test]
    fn write_image_emits_scaling_keywords_and_preserves_unsigned_values() {
        // u16 data stored as signed-16 with BZERO = 32768.
        let image = Image {
            shape: vec![3],
            samples: ImageData::I16(vec![-32768, 0, 32767]),
            scaling: Scaling {
                bscale: 1.0,
                bzero: 32768.0,
                blank: None,
            },
        };
        let mut r = FitsReader::open(Cursor::new(write_to_vec(&image))).unwrap();
        assert_eq!(r.hdus[0].header.get_real("BZERO"), Some(32768.0));
        assert_eq!(r.hdus[0].header.get_real("BSCALE"), Some(1.0));
        let back = r.read_image(0).unwrap();
        assert_eq!(back.samples, ImageData::I16(vec![-32768, 0, 32767]));
        assert_eq!(back.physical(), vec![0.0, 32768.0, 65535.0]);
    }

    #[test]
    fn checksums_round_trip_and_verify() {
        let image = Image {
            shape: vec![2, 2],
            samples: ImageData::I16(vec![1, 2, 3, 4]),
            scaling: identity(),
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
        w.write_image(&image).unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        let report = r.verify_checksum(0).unwrap();
        assert_eq!(report.datasum_ok, Some(true));
        assert_eq!(report.checksum_ok, Some(true)); // whole-HDU sum is −0
    }

    #[test]
    fn corrupted_data_fails_checksum() {
        let image = Image {
            shape: vec![2, 2],
            samples: ImageData::I16(vec![1, 2, 3, 4]),
            scaling: identity(),
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
        w.write_image(&image).unwrap();
        let mut bytes = w.into_inner().into_inner();
        bytes[BLOCK_SIZE] ^= 0xFF; // flip the first data byte (data starts at block 1)

        let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
        let report = r.verify_checksum(0).unwrap();
        assert_eq!(report.datasum_ok, Some(false));
        assert_eq!(report.checksum_ok, Some(false));
    }

    #[test]
    fn verify_is_none_when_checksum_keywords_are_absent() {
        let image = Image {
            shape: vec![2, 2],
            samples: ImageData::U8(vec![0, 0, 0, 0]),
            scaling: identity(),
        };
        let mut r = FitsReader::open(Cursor::new(write_to_vec(&image))).unwrap();
        let report = r.verify_checksum(0).unwrap();
        assert_eq!(report.datasum_ok, None);
        assert_eq!(report.checksum_ok, None);
    }

    #[test]
    fn written_file_reads_back_with_matching_boundaries() {
        let header = header(&[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    1",
            "NAXIS1  =                   10",
        ]);
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        writer.write_header(&header).unwrap();
        writer.write_data_unit(&[0u8; 10], ZERO_FILL).unwrap();
        let bytes = writer.into_inner().into_inner();

        // Header block + one padded data block.
        assert_eq!(bytes.len(), 2 * BLOCK_SIZE);

        let f = FitsReader::open(Cursor::new(bytes)).unwrap();
        assert_eq!(f.hdus.len(), 1);
        assert_eq!(f.hdus[0].data_offset, BLOCK_SIZE as u64);
        assert_eq!(f.hdus[0].data_len, BLOCK_SIZE as u64);
        assert_eq!(f.hdus[0].header.axes().unwrap(), vec![10]);
    }
}
