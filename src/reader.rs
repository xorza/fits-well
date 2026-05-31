use std::io::Read;
use std::io::Seek;
use std::io::SeekFrom;
use std::ops::Range;

use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::data_extent;
use crate::header::Header;

/// One Header/Data Unit located by the reader.
///
/// The data unit itself is read lazily via [`FitsReader::read_data_raw`]; this
/// record only carries the parsed header, the inferred [`HduKind`], and the
/// data unit's byte range within the source.
#[derive(Debug)]
pub struct Hdu {
    pub header: Header,
    pub kind: HduKind,
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
        while let Some(header_bytes) = read_header_unit(&mut source)? {
            let header = Header::parse(&header_bytes)?;
            let kind = HduKind::classify(&header);
            let data_offset = source.stream_position()?;
            let extent = data_extent(&header)?;
            source.seek(SeekFrom::Current(extent.padded_bytes as i64))?;
            hdus.push(Hdu {
                header,
                kind,
                data_offset,
                data_bytes: extent.data_bytes,
                data_len: extent.padded_bytes,
            });
        }
        Ok(FitsReader { source, hdus })
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
        let bitpix = hdu.header.bitpix()?;
        let shape = hdu.header.axes()?;
        let scaling = Scaling::from_header(&hdu.header);
        let samples = ImageData::decode(unit.data(), bitpix);

        let expected = if shape.is_empty() {
            0
        } else {
            shape.iter().product::<usize>()
        };
        assert_eq!(
            samples.len(),
            expected,
            "decoded sample count must match the NAXISn product"
        );
        Ok(Image {
            shape,
            samples,
            scaling,
        })
    }
}

/// Read one header unit: consume 2880-byte blocks until one carries the `END`
/// record. Returns `None` at a clean end of stream (no HDU left to read).
fn read_header_unit<R: Read>(source: &mut R) -> Result<Option<Vec<u8>>> {
    let mut bytes = Vec::new();
    loop {
        let mut block = [0u8; BLOCK_SIZE];
        match fill_block(source, &mut block)? {
            BlockRead::Eof if bytes.is_empty() => return Ok(None),
            BlockRead::Eof => return Err(FitsError::UnexpectedEof),
            BlockRead::Full => {
                bytes.extend_from_slice(&block);
                if block_has_end(&block) {
                    return Ok(Some(bytes));
                }
            }
        }
    }
}

enum BlockRead {
    Full,
    Eof,
}

/// Read exactly one block, distinguishing a clean EOF (zero bytes) from a
/// truncated unit (a partial block before EOF).
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
        _ => Err(FitsError::UnexpectedEof),
    }
}

fn block_has_end(block: &[u8; BLOCK_SIZE]) -> bool {
    block
        .chunks_exact(CARD_SIZE)
        .any(|card| &card[..3] == b"END" && card[3..].iter().all(|&b| b == b' '))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::bitpix::Bitpix;
    use std::fs::File;

    fn open(name: &str) -> FitsReader<File> {
        let path = format!("tests/data/fits/{name}");
        FitsReader::open(File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}")))
            .unwrap_or_else(|e| panic!("parse {name}: {e}"))
    }

    #[test]
    fn reads_a_single_hdu_image_with_exact_boundaries() {
        let f = open("UITfuv2582gc.fits");
        assert_eq!(f.hdus.len(), 1);
        let p = &f.hdus[0];
        assert_eq!(p.kind, HduKind::Primary);
        assert_eq!(p.header.bitpix().unwrap(), Bitpix::I16);
        assert_eq!(p.header.axes().unwrap(), vec![512, 512]);
        assert_eq!(p.data_offset, 11_520);
        assert_eq!(p.data_len, 527_040);
    }

    #[test]
    fn reads_random_groups_primary_plus_bintable_extension() {
        let f = open("DDTSUVDATA.fits");
        assert_eq!(f.hdus.len(), 2);

        let g = &f.hdus[0];
        assert_eq!(g.kind, HduKind::RandomGroups);
        assert_eq!(g.header.bitpix().unwrap(), Bitpix::F32);
        assert_eq!(g.header.axes().unwrap(), vec![0, 3, 4, 1, 1, 1]);
        assert_eq!(g.data_offset, 14_400);
        assert_eq!(g.data_len, 573_120);

        let t = &f.hdus[1];
        assert_eq!(t.kind, HduKind::BinTable);
        assert_eq!(t.data_offset, 593_280);
        assert_eq!(t.data_len, 2_880);
    }

    #[test]
    fn reads_dataless_primary_then_bintable() {
        let f = open("IUElwp25637mxlo.fits");
        assert_eq!(f.hdus.len(), 2);

        let p = &f.hdus[0];
        assert_eq!(p.kind, HduKind::Primary);
        assert_eq!(p.header.naxis().unwrap(), 0);
        assert_eq!(p.data_offset, 28_800);
        assert_eq!(p.data_len, 0);

        let t = &f.hdus[1];
        assert_eq!(t.kind, HduKind::BinTable);
        assert_eq!(t.data_offset, 34_560);
        assert_eq!(t.data_len, 14_400);
    }

    #[test]
    fn last_data_unit_ends_exactly_at_end_of_file() {
        for name in [
            "UITfuv2582gc.fits",
            "DDTSUVDATA.fits",
            "IUElwp25637mxlo.fits",
        ] {
            let f = open(name);
            let last = f.hdus.last().unwrap();
            let file_len = std::fs::metadata(format!("tests/data/fits/{name}"))
                .unwrap()
                .len();
            assert_eq!(last.data_offset + last.data_len, file_len, "{name}");
        }
    }

    #[test]
    fn read_data_raw_returns_padded_bytes_and_the_data_range() {
        let mut f = open("UITfuv2582gc.fits");
        let unit = f.read_data_raw(0).unwrap();
        // 512×512 i16: 524_288 bytes of data, padded up to 527_040 on disk.
        assert_eq!(unit.bytes.len(), 527_040);
        assert_eq!(unit.data_range, 0..524_288);
        assert_eq!(unit.data().len(), 524_288);
        // The padding past the data range is block fill, not samples.
        assert!(unit.bytes[524_288..].iter().all(|&b| b == 0));
    }

    #[test]
    fn read_data_raw_rejects_out_of_bounds_index() {
        let mut f = open("UITfuv2582gc.fits"); // a single-HDU file
        assert!(matches!(
            f.read_data_raw(5),
            Err(FitsError::HduIndexOutOfBounds { index: 5, len: 1 })
        ));
    }

    #[test]
    fn read_image_decodes_the_primary_array_shape_and_type() {
        let mut f = open("UITfuv2582gc.fits");
        let img = f.read_image(0).unwrap();
        assert_eq!(img.shape, vec![512, 512]);
        assert_eq!(img.samples.bitpix(), Bitpix::I16);
        assert_eq!(img.samples.len(), 512 * 512);
        assert_eq!(img.physical().len(), 512 * 512);
    }

    #[test]
    fn read_image_raw_samples_match_a_manual_big_endian_decode() {
        let mut f = open("UITfuv2582gc.fits");
        // Independently decode the first few pixels straight from the data bytes.
        let unit = f.read_data_raw(0).unwrap();
        let manual: Vec<i16> = unit.data()[..8]
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]))
            .collect();
        let img = f.read_image(0).unwrap();
        match img.samples {
            ImageData::I16(v) => assert_eq!(&v[..4], manual.as_slice()),
            other => panic!("expected I16 samples, got {other:?}"),
        }
    }

    #[test]
    fn read_image_rejects_non_image_hdus() {
        // hdu[0] is random groups, hdu[1] is a binary table — neither is an image.
        let mut f = open("DDTSUVDATA.fits");
        assert!(matches!(f.read_image(0), Err(FitsError::NotAnImage)));
        assert!(matches!(f.read_image(1), Err(FitsError::NotAnImage)));
    }
}
