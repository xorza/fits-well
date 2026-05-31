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

    pub fn into_inner(self) -> W {
        self.sink
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::block::ZERO_FILL;
    use crate::reader::FitsReader;
    use std::io::Cursor;

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
