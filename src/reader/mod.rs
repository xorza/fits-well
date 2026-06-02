use std::io::Read;
use std::io::Seek;
use std::ops::Range;

use crate::ascii::AsciiTable;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::padded_len;
use crate::checksum;
#[cfg(feature = "compression")]
use crate::data::Image;
use crate::data::RawImage;
use crate::data::Scaling;
use crate::data::shape_product;
use crate::error::FitsError;
use crate::error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::hdu::data_extent;
use crate::header::Header;
use crate::table::BinTable;

pub(crate) mod source;

use source::SliceSource;
use source::Source;
use source::StreamSource;

#[cfg(feature = "compression")]
use crate::compress::{decompress_image, uncompress_table};

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
    /// the padded unit. The on-disk length is `padded_len(data_bytes)`.
    pub(crate) data_bytes: u64,
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
pub struct FitsReader<S> {
    source: S,
    pub hdus: Vec<Hdu>,
    /// Reused staging buffer for the seeking-source reads: a [`StreamSource`] copies
    /// each data unit here before decoding (an in-memory source borrows instead, so
    /// this stays empty). Grows once to the largest unit touched, then holds.
    scratch: Vec<u8>,
}

impl<R: Read + Seek> FitsReader<StreamSource<R>> {
    /// Open a seekable byte source (file, cursor). Data units are copied into the
    /// reader's scratch on demand; for an in-memory file prefer
    /// [`FitsReader::from_bytes`], which decodes straight from the bytes.
    pub fn open(source: R) -> Result<FitsReader<StreamSource<R>>> {
        FitsReader::from_source(StreamSource::new(source)?)
    }
}

impl<'a> FitsReader<SliceSource<'a>> {
    /// Open an in-memory FITS file — the whole thing as a byte slice (e.g. an mmap,
    /// or bytes already in RAM). Data units decode straight from the borrowed bytes
    /// with no staging copy, and no scratch allocation.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<FitsReader<SliceSource<'a>>> {
        FitsReader::from_source(SliceSource::new(bytes))
    }
}

#[cfg(feature = "mmap")]
impl FitsReader<source::MmapSource> {
    /// Memory-map a FITS file and read it zero-copy: data units decode straight from
    /// the mapped pages (no staging copy, no read syscalls). Best for large files
    /// and random HDU access. Requires the `mmap` feature.
    pub fn open_mmap(path: impl AsRef<std::path::Path>) -> Result<FitsReader<source::MmapSource>> {
        FitsReader::from_source(source::MmapSource::open(path.as_ref())?)
    }
}

impl<S: Source> FitsReader<S> {
    /// Scan the whole HDU sequence, parsing every header and recording the byte
    /// range of each data unit — without reading any data.
    fn from_source(mut source: S) -> Result<FitsReader<S>> {
        let mut scratch = Vec::new();
        let mut hdus = Vec::new();
        let mut offset = 0u64;
        loop {
            match scan_header_unit(&mut source, &mut offset, &mut scratch)? {
                NextHeader::Found(header_bytes) => {
                    let header = Header::parse(&header_bytes)?;
                    let kind = HduKind::classify(&header);
                    let data_offset = offset;
                    let extent = data_extent(&header)?;
                    let next = data_offset
                        .checked_add(extent.padded_bytes)
                        .ok_or(FitsError::DataUnitOverflow)?;
                    hdus.push(Hdu {
                        header,
                        kind,
                        header_bytes,
                        data_offset,
                        data_bytes: extent.data_bytes,
                    });
                    // Skip past the data unit to the next header. Clamp at the source
                    // end so a declared unit larger than the file just ends the scan
                    // (the HDU is still recorded; a later read bounds-checks it).
                    offset = next.min(source.size());
                }
                NextHeader::End => break,
                // §3.5/§3.6: special records and a trailing partial / fill block may
                // follow the last HDU; a reader disregards them. But the same shape
                // *before* any valid HDU means there is no conforming primary.
                NextHeader::Trailing if hdus.is_empty() => return Err(FitsError::UnexpectedEof),
                NextHeader::Trailing => break,
            }
        }
        Ok(FitsReader {
            source,
            hdus,
            scratch,
        })
    }

    /// The HDU at `index` (panics if out of range — check `self.hdus.len()` first).
    pub fn hdu(&self, index: usize) -> &Hdu {
        &self.hdus[index]
    }

    /// The HDU at `index`, or [`FitsError::HduIndexOutOfBounds`] — the checked form
    /// the `read_*` methods bound-check through.
    fn checked_hdu(&self, index: usize) -> Result<&Hdu> {
        self.hdus.get(index).ok_or(FitsError::HduIndexOutOfBounds {
            index,
            len: self.hdus.len(),
        })
    }

    /// Read the raw, still-encoded (big-endian, unscaled) data unit into a fresh,
    /// caller-owned buffer. The returned [`DataUnit`] carries the full block-padded
    /// bytes plus the range of actual data within them, so a decoder consumes
    /// [`DataUnit::data`] and the block fill is never mistaken for samples.
    ///
    /// This is the owned form, backing the table readers (which keep the bytes as
    /// the parsed table's storage). Image and random-groups reads instead stage
    /// through the reader's reused internal scratch — see [`FitsReader::read_image`].
    pub fn read_data_raw(&mut self, index: usize) -> Result<DataUnit> {
        let hdu = self.checked_hdu(index)?;
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let bytes = self
            .source
            .read_owned(data_offset, padded_len(data_bytes) as usize)?;
        Ok(DataUnit {
            bytes,
            data_range: 0..data_bytes as usize,
        })
    }

    /// Borrow an HDU's data unit as a [`RawImage`] — the stored big-endian bytes
    /// viewed in place (over the in-memory source, or the reader's reused scratch for
    /// a seeking source), plus the shape, `BITPIX`, and [`Scaling`] needed to
    /// interpret them. **Zero-copy**: nothing is decoded until you ask. Errors with
    /// [`FitsError::NotAnImage`] for tables, random groups, and unmodelled extensions.
    ///
    /// The result borrows the reader, so it can't outlive it and only one is live at
    /// a time — own what you need first: [`RawImage::u8`] for the zero-copy `BITPIX =
    /// 8` plane, [`RawImage::decode`] for host-endian samples, [`RawImage::physical`]
    /// for the scaled plane.
    pub fn read_image(&mut self, index: usize) -> Result<RawImage<'_>> {
        let hdu = self.checked_hdu(index)?;
        if !matches!(hdu.kind, HduKind::Primary | HduKind::Image) {
            return Err(FitsError::NotAnImage);
        }
        // §4.3: a plain image array has no group structure. A non-conforming
        // `PCOUNT`/`GCOUNT` would make `data_extent` size extra bytes, so reject it
        // up front (on untrusted input) rather than expose mismatched samples.
        if hdu.header.get_integer("PCOUNT").unwrap_or(0) != 0
            || hdu.header.get_integer("GCOUNT").unwrap_or(1) != 1
        {
            return Err(FitsError::ImageHasGroups);
        }
        let bitpix = hdu.header.bitpix()?;
        let shape = hdu.header.axes()?;
        let scaling = Scaling::from_header(&hdu.header);
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let unit = self.source.slice(
            data_offset,
            padded_len(data_bytes) as usize,
            &mut self.scratch,
        )?;
        let bytes = &unit[..data_bytes as usize];

        // With PCOUNT=0/GCOUNT=1 (checked above), `data_extent` sized the unit as
        // `elem · Π(axes)`, so the borrowed data is exactly `shape_product` elements
        // wide. This is an invariant between `data_extent` and the shape, not a
        // runtime failure mode — assert it rather than return an error that can't occur.
        debug_assert_eq!(
            bytes.len(),
            shape_product(&shape) * bitpix.elem_size(),
            "image data length must match the axis product"
        );
        Ok(RawImage {
            shape,
            bitpix,
            scaling,
            bytes,
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
        let hdu = self.checked_hdu(index)?;
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::NotRandomGroups);
        }
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let unit = self.source.slice(
            data_offset,
            padded_len(data_bytes) as usize,
            &mut self.scratch,
        )?;
        RandomGroups::from_data(&self.hdus[index].header, &unit[..data_bytes as usize])
    }

    /// Read a tiled-compressed image (§10.1) — a `BINTABLE` with `ZIMAGE = T` —
    /// and decompress it into the full [`Image`]. Supports `GZIP_1` and `RICE_1`.
    /// Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_image(&mut self, index: usize) -> Result<Image> {
        let table = self.read_table(index)?;
        let header = &self.hdus[index].header;
        decompress_image(header, &table)
    }

    /// Read a tiled-compressed table (§10.3) — a `BINTABLE` with `ZTABLE = T` —
    /// and uncompress it into the original [`BinTable`]. Fixed-width columns only
    /// (`GZIP_1`/`GZIP_2`/`RICE_1`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_table(&mut self, index: usize) -> Result<BinTable> {
        let table = self.read_table(index)?;
        let header = self.hdus[index].header.clone();
        let parts = uncompress_table(&header, &table)?;
        BinTable::from_data(&parts.header, parts.data)
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J). Each
    /// field of the report is `None` if that keyword is absent, else `Some(true)`
    /// when it matches the recomputed checksum.
    pub fn verify_checksum(&mut self, index: usize) -> Result<ChecksumReport> {
        let hdu = self.checked_hdu(index)?;
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        // The block-padded data unit (length = the padded size — the checksum covers
        // the block fill too).
        let unit = self.source.slice(
            data_offset,
            padded_len(data_bytes) as usize,
            &mut self.scratch,
        )?;
        let data_sum = checksum::accumulate(unit, 0);
        let hdu = &self.hdus[index];
        // Whole HDU = header (incl. the stored CHECKSUM card) then data.
        let hdu_sum = checksum::accumulate(unit, checksum::accumulate(&hdu.header_bytes, 0));
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

/// Read one header unit at `*offset`, advancing `offset` past each consumed block,
/// until a block carries the `END` record. Blocks come through [`Source::slice`], so
/// the same scan drives both seeking and in-memory sources.
fn scan_header_unit<S: Source>(
    source: &mut S,
    offset: &mut u64,
    scratch: &mut Vec<u8>,
) -> Result<NextHeader> {
    let size = source.size();
    // Most headers are a single block; reserve it so the common case parses with one
    // allocation and only multi-block headers grow.
    let mut bytes = Vec::with_capacity(BLOCK_SIZE);
    loop {
        match size - *offset {
            // Clean end at a block boundary, or trailing blocks with no `END`.
            0 if bytes.is_empty() => return Ok(NextHeader::End),
            0 => return Ok(NextHeader::Trailing),
            // A sub-block remnant before EOF: trailing content (§3.6).
            avail if avail < BLOCK_SIZE as u64 => return Ok(NextHeader::Trailing),
            _ => {}
        }
        let block = source.slice(*offset, BLOCK_SIZE, scratch)?;
        *offset += BLOCK_SIZE as u64;
        bytes.extend_from_slice(block);
        if block_has_end(block) {
            return Ok(NextHeader::Found(bytes));
        }
    }
}

fn block_has_end(block: &[u8]) -> bool {
    block
        .chunks_exact(CARD_SIZE)
        .any(|card| &card[..3] == b"END" && card[3..].iter().all(|&b| b == b' '))
}

#[cfg(test)]
mod tests;
