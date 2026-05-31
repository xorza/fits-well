use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::Range;

use crate::ascii::AsciiTable;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::checksum;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::hdu::data_extent;
use crate::header::Header;
use crate::table::BinTable;

/// One Header/Data Unit located by the reader.
///
/// The data unit itself is read lazily via [`FitsReader::read_data_raw`]; this
/// record only carries the parsed header, the inferred [`HduKind`], and the
/// data unit's byte range within the source.
#[derive(Debug)]
pub struct Hdu {
    pub header: Header,
    pub kind: HduKind,
    /// The raw, block-padded header-unit bytes as read — retained for checksum
    /// verification (the exact bytes matter).
    pub(crate) header_bytes: Vec<u8>,
    /// Byte offset of the data unit from the start of the source.
    pub(crate) data_offset: u64,
    /// Unpadded data length (`Nbits / 8`) — where the meaningful data ends within
    /// the padded unit.
    pub(crate) data_bytes: u64,
    /// On-disk (block-padded) length of the data unit in bytes.
    pub(crate) data_len: u64,
}

/// A data unit read from the source: the full block-padded bytes plus the range
/// within them holding the actual data. The bytes after `data_range` are FITS
/// block fill, not part of the data array.
#[derive(Debug, Clone)]
pub struct DataUnit {
    /// The on-disk data unit, padded to the 2880-byte block grid.
    pub bytes: Vec<u8>,
    /// The sub-range of `bytes` that is meaningful data (`0..Nbits/8`).
    pub data_range: Range<usize>,
}

impl DataUnit {
    /// The meaningful data with the trailing block fill sliced off — what a
    /// decoder should consume.
    pub fn data(&self) -> &[u8] {
        &self.bytes[self.data_range.clone()]
    }
}

/// A FITS file opened over a seekable byte source. Opening scans HDU boundaries
/// from headers alone (no data is read); data units are fetched on demand.
#[derive(Debug)]
pub struct FitsReader<R> {
    source: R,
    pub hdus: Vec<Hdu>,
}

impl<R: Read + Seek> FitsReader<R> {
    /// Scan the whole HDU sequence, parsing every header and recording the byte
    /// range of each data unit.
    pub fn open(mut source: R) -> Result<Self> {
        let mut hdus = Vec::new();
        loop {
            match read_header_unit(&mut source)? {
                NextHeader::Found(header_bytes) => {
                    let header = Header::parse(&header_bytes)?;
                    let kind = HduKind::classify(&header);
                    let data_offset = source.stream_position()?;
                    let extent = data_extent(&header)?;
                    source.seek(SeekFrom::Current(extent.padded_bytes as i64))?;
                    hdus.push(Hdu {
                        header,
                        kind,
                        header_bytes,
                        data_offset,
                        data_bytes: extent.data_bytes,
                        data_len: extent.padded_bytes,
                    });
                }
                NextHeader::End => break,
                // §3.5/§3.6: special records and a trailing partial / fill block may
                // follow the last HDU; a reader disregards them. But the same shape
                // *before* any valid HDU means there is no conforming primary.
                NextHeader::Trailing if hdus.is_empty() => return Err(FitsError::UnexpectedEof),
                NextHeader::Trailing => break,
            }
        }
        Ok(FitsReader { source, hdus })
    }

    /// The HDUs discovered by the lazy scan, in file order (HDU 0 is the primary).
    pub fn hdus(&self) -> &[Hdu] {
        &self.hdus
    }

    /// The HDU at `index` (panics if out of range — use [`FitsReader::hdus`] to
    /// check the count first).
    pub fn hdu(&self, index: usize) -> &Hdu {
        &self.hdus[index]
    }

    /// Read the raw, still-encoded (big-endian, unscaled) data unit. The returned
    /// [`DataUnit`] carries the full block-padded bytes plus the range of actual
    /// data within them, so a decoder consumes [`DataUnit::data`] and the block
    /// fill is never mistaken for samples. Typed decoding is the data layer's job.
    pub fn read_data_raw(&mut self, index: usize) -> Result<DataUnit> {
        let hdu = self.hdus.get(index).ok_or(FitsError::HduIndexOutOfBounds {
            index,
            len: self.hdus.len(),
        })?;
        let data_range = 0..hdu.data_bytes as usize;
        self.source.seek(SeekFrom::Start(hdu.data_offset))?;
        let mut bytes = vec![0u8; hdu.data_len as usize];
        self.source.read_exact(&mut bytes)?;
        Ok(DataUnit { bytes, data_range })
    }

    /// Read an HDU's data unit and decode it into a typed [`Image`]: host-endian
    /// raw samples (`samples`) plus the [`Scaling`] map for the physical plane.
    /// Errors with [`FitsError::NotAnImage`] for tables, random groups, and
    /// unmodelled extensions.
    pub fn read_image(&mut self, index: usize) -> Result<Image> {
        let unit = self.read_data_raw(index)?; // also bounds-checks the index
        let hdu = &self.hdus[index];
        if !matches!(hdu.kind, HduKind::Primary | HduKind::Image) {
            return Err(FitsError::NotAnImage);
        }
        // §4.3: a plain image array has no group structure. A non-conforming
        // `PCOUNT`/`GCOUNT` would make `data_extent` size extra bytes, so reject it
        // up front (on untrusted input) rather than decode mismatched samples.
        if hdu.header.get_integer("PCOUNT").unwrap_or(0) != 0 {
            return Err(FitsError::WrongValueType { name: "PCOUNT" });
        }
        if hdu.header.get_integer("GCOUNT").unwrap_or(1) != 1 {
            return Err(FitsError::WrongValueType { name: "GCOUNT" });
        }
        let bitpix = hdu.header.bitpix()?;
        let shape = hdu.header.axes()?;
        let scaling = Scaling::from_header(&hdu.header);
        let samples = ImageData::decode(unit.data(), bitpix);

        let expected = if shape.is_empty() {
            0
        } else {
            shape.iter().product::<usize>()
        };
        if samples.len() != expected {
            return Err(FitsError::DataSizeMismatch {
                expected,
                got: samples.len(),
            });
        }
        Ok(Image {
            shape,
            samples,
            scaling,
        })
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`BinTable::read_column`]. Errors with
    /// [`FitsError::NotABinTable`] for any other HDU kind.
    pub fn read_table(&mut self, index: usize) -> Result<BinTable> {
        let unit = self.read_data_raw(index)?; // also bounds-checks the index
        let hdu = &self.hdus[index];
        if hdu.kind != HduKind::BinTable {
            return Err(FitsError::NotABinTable);
        }
        BinTable::from_data(&hdu.header, unit.bytes)
    }

    /// Read an `TABLE` (ASCII table) extension and parse its column structure.
    /// Errors with [`FitsError::NotAnAsciiTable`] for any other HDU.
    pub fn read_ascii_table(&mut self, index: usize) -> Result<AsciiTable> {
        let unit = self.read_data_raw(index)?;
        let hdu = &self.hdus[index];
        if hdu.kind != HduKind::AsciiTable {
            return Err(FitsError::NotAnAsciiTable);
        }
        AsciiTable::from_data(&hdu.header, unit.bytes)
    }

    /// Read and decode a random-groups primary array (§6). Errors with
    /// [`FitsError::NotRandomGroups`] for any other HDU.
    pub fn read_groups(&mut self, index: usize) -> Result<RandomGroups> {
        let unit = self.read_data_raw(index)?;
        let hdu = &self.hdus[index];
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::NotRandomGroups);
        }
        RandomGroups::from_data(&hdu.header, unit.data())
    }

    /// Read a tiled-compressed image (§10.1) — a `BINTABLE` with `ZIMAGE = T` —
    /// and decompress it into the full [`Image`]. Supports `GZIP_1` and `RICE_1`.
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_image(&mut self, index: usize) -> Result<Image> {
        let table = self.read_table(index)?;
        let header = &self.hdus[index].header;
        crate::compress::decompress_image(header, &table)
    }

    /// Read a tiled-compressed table (§10.3) — a `BINTABLE` with `ZTABLE = T` —
    /// and uncompress it into the original [`BinTable`]. Fixed-width columns only
    /// (`GZIP_1`/`GZIP_2`/`RICE_1`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_table(&mut self, index: usize) -> Result<BinTable> {
        let table = self.read_table(index)?;
        let header = self.hdus[index].header.clone();
        let (out_header, data) = crate::compress::uncompress_table(&header, &table)?;
        BinTable::from_data(&out_header, data)
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J). Each
    /// field of the report is `None` if that keyword is absent, else `Some(true)`
    /// when it matches the recomputed checksum.
    pub fn verify_checksum(&mut self, index: usize) -> Result<ChecksumReport> {
        let data = self.read_data_raw(index)?.bytes; // block-padded data unit
        let hdu = &self.hdus[index];
        let data_sum = checksum::accumulate(&data, 0);
        // Whole HDU = header (incl. the stored CHECKSUM card) then data.
        let hdu_sum = checksum::accumulate(&data, checksum::accumulate(&hdu.header_bytes, 0));
        Ok(ChecksumReport {
            datasum_ok: hdu
                .header
                .get_text("DATASUM")
                .map(|s| s.trim().parse::<u32>().ok() == Some(data_sum)),
            checksum_ok: hdu
                .header
                .get_text("CHECKSUM")
                .map(|_| hdu_sum == 0xFFFF_FFFF),
        })
    }
}

/// Result of [`FitsReader::verify_checksum`]. A field is `None` when its keyword
/// is absent.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecksumReport {
    pub datasum_ok: Option<bool>,
    pub checksum_ok: Option<bool>,
}

/// Outcome of scanning for the next header unit.
enum NextHeader {
    /// A complete header unit terminated by an `END` card.
    Found(Vec<u8>),
    /// Clean end of stream at a block boundary — no more HDUs.
    End,
    /// Trailing bytes carrying no `END`: special records (§3.5) or a trailing
    /// partial / fill block (§3.6). Disregarded after the last HDU.
    Trailing,
}

/// Read one header unit: consume 2880-byte blocks until one carries the `END`
/// record.
fn read_header_unit<R: Read>(source: &mut R) -> Result<NextHeader> {
    let mut bytes = Vec::new();
    loop {
        let mut block = [0u8; BLOCK_SIZE];
        match fill_block(source, &mut block)? {
            BlockRead::Eof if bytes.is_empty() => return Ok(NextHeader::End),
            // EOF or a sub-block remnant with no `END` seen: trailing content.
            BlockRead::Eof | BlockRead::Partial => return Ok(NextHeader::Trailing),
            BlockRead::Full => {
                bytes.extend_from_slice(&block);
                if block_has_end(&block) {
                    return Ok(NextHeader::Found(bytes));
                }
            }
        }
    }
}

enum BlockRead {
    Full,
    Partial,
    Eof,
}

/// Read exactly one block, distinguishing a clean EOF (zero bytes) from a
/// trailing partial block (a sub-block remnant before EOF).
fn fill_block<R: Read>(source: &mut R, block: &mut [u8; BLOCK_SIZE]) -> Result<BlockRead> {
    let mut filled = 0;
    while filled < BLOCK_SIZE {
        let n = source.read(&mut block[filled..])?;
        if n == 0 {
            break;
        }
        filled += n;
    }
    match filled {
        0 => Ok(BlockRead::Eof),
        BLOCK_SIZE => Ok(BlockRead::Full),
        _ => Ok(BlockRead::Partial),
    }
}

fn block_has_end(block: &[u8; BLOCK_SIZE]) -> bool {
    block
        .chunks_exact(CARD_SIZE)
        .any(|card| &card[..3] == b"END" && card[3..].iter().all(|&b| b == b' '))
}

#[cfg(test)]
mod tests;
