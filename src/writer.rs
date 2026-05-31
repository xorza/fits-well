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
use crate::data::Image;
use crate::error::Result;
use crate::header::Header;

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

/// Writes FITS HDUs to a byte sink, one unit at a time.
#[derive(Debug)]
pub struct FitsWriter<W> {
    sink: W,
}

impl<W: Write> FitsWriter<W> {
    pub fn new(sink: W) -> Self {
        FitsWriter { sink }
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

    /// Write `image` as a primary HDU: a synthesized mandatory header
    /// (`SIMPLE`/`BITPIX`/`NAXISn`, plus `BSCALE`/`BZERO`/`BLANK` when the scaling
    /// is non-trivial) followed by the big-endian data unit.
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
        self.write_header(&primary_image_header(image))?;
        self.write_data_unit(&image.samples.encode(), ZERO_FILL)
    }

    pub fn into_inner(self) -> W {
        self.sink
    }
}

/// Build the mandatory primary-array header for an image (§4.4.1).
fn primary_image_header(image: &Image) -> Header {
    let mut header = Header::new();
    header
        .set("SIMPLE", true)
        .comment("SIMPLE", "file conforms to FITS standard");
    header
        .set("BITPIX", image.samples.bitpix().code())
        .comment("BITPIX", "number of bits per data pixel");
    header
        .set("NAXIS", image.shape.len() as i64)
        .comment("NAXIS", "number of data axes");
    for (i, &n) in image.shape.iter().enumerate() {
        header.set(&format!("NAXIS{}", i + 1), n as i64);
    }
    // Emit scaling only when it carries information beyond the identity map.
    if !image.scaling.is_identity() {
        header.set("BZERO", image.scaling.bzero);
        header.set("BSCALE", image.scaling.bscale);
    }
    if let Some(blank) = image.scaling.blank {
        header.set("BLANK", blank);
    }
    header
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::ZERO_FILL;
    use crate::data::{ImageData, Scaling};
    use crate::hdu::HduKind;
    use crate::reader::FitsReader;
    use std::io::Cursor;

    fn write_to_vec(image: &Image) -> Vec<u8> {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_image(image).unwrap();
        w.into_inner().into_inner()
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
