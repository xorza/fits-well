//! [`Hdus`]: the HDU list a scan of a file's headers finds.

use std::ops::Index;
use std::slice;

use crate::allocation;
use crate::block::{BLOCK_SIZE, CARD_SIZE};
use crate::checksum;
use crate::error::{FitsError, Indexed, Result};
use crate::hdu::HduPosition;
use crate::header_model::card::is_end_record;
use crate::header_model::keywords;
use crate::reader::hdu::{Hdu, HeaderUnit};
use crate::reader::hdu_selector::HduSelector;
use crate::reader::source::Source;

/// The HDUs of a file in file order, as a scan of its headers finds them: the one list
/// the reader and the editor select from.
#[derive(Debug)]
pub(crate) struct Hdus {
    hdus: Vec<Hdu>,
}

impl Hdus {
    /// Scan the whole HDU sequence of `source`, parsing every header and recording the
    /// byte range of each data unit — without reading any data. `scratch` stages a
    /// seeking source's blocks.
    pub(crate) fn scan<S: Source>(source: &mut S, scratch: &mut Vec<u8>) -> Result<Self> {
        let mut hdus = Vec::new();
        let mut scan = Scan {
            source,
            scratch,
            offset: 0,
        };
        loop {
            let position = HduPosition::of_index(hdus.len());
            match scan.next_header(position)? {
                NextHeader::Found { bytes, sum } => {
                    let unit = HeaderUnit {
                        bytes: &bytes,
                        sum,
                        offset: scan.offset - bytes.len() as u64,
                    };
                    let hdu = Hdu::parse(hdus.len(), unit)?;
                    let next = hdu
                        .data_offset
                        .checked_add(hdu.padded_data_bytes())
                        .ok_or(FitsError::DataUnitOverflow)?;
                    hdus.push(hdu);
                    // Skip past the data unit to the next header. Clamp at the source
                    // end so a declared unit larger than the file just ends the scan
                    // (the HDU is still recorded; a later read bounds-checks it).
                    scan.offset = next.min(scan.source.size());
                }
                // §3.5/§3.6: special records and a trailing partial or fill block may
                // follow the last HDU, and a reader disregards them. Before any HDU,
                // either shape means there is no conforming primary.
                NextHeader::End | NextHeader::Trailing if hdus.is_empty() => {
                    return Err(FitsError::UnexpectedEof);
                }
                NextHeader::End | NextHeader::Trailing => break,
            }
        }
        Ok(Self { hdus })
    }

    pub(crate) fn as_slice(&self) -> &[Hdu] {
        &self.hdus
    }

    pub(crate) fn iter(&self) -> slice::Iter<'_, Hdu> {
        self.hdus.iter()
    }

    /// The last HDU: a scan finds a primary at least.
    pub(crate) fn last(&self) -> &Hdu {
        self.hdus.last().expect("a scan finds a primary HDU")
    }

    /// The HDU `selector` names: the first that [`Hdu::matches`] it. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last HDU, and with
    /// [`FitsError::HduNotFound`] for a name no HDU carries.
    pub(crate) fn select(&self, selector: HduSelector<'_>) -> Result<&Hdu> {
        match selector {
            HduSelector::Index(index) => self.hdus.get(index).ok_or(FitsError::IndexOutOfBounds {
                indexed: Indexed::Hdu,
                index,
                len: self.hdus.len(),
            }),
            HduSelector::Extension {
                name,
                version,
                level,
            } => {
                for hdu in &self.hdus {
                    if hdu.matches(selector)? {
                        return Ok(hdu);
                    }
                }
                Err(FitsError::HduNotFound {
                    name: name.to_string(),
                    version,
                    level,
                })
            }
        }
    }

    /// Every HDU that holds an image array, as [`Hdu::is_image`] judges it.
    pub(crate) fn images(&self) -> impl Iterator<Item = &Hdu> + '_ {
        self.hdus.iter().filter(|hdu| hdu.is_image())
    }

    /// Put `hdu` in the place of the HDU of its index, as an edit of its header leaves
    /// it.
    pub(crate) fn replace(&mut self, hdu: Hdu) {
        let index = hdu.index;
        self.hdus[index] = hdu;
    }
}

impl Index<usize> for Hdus {
    type Output = Hdu;

    fn index(&self, index: usize) -> &Hdu {
        &self.hdus[index]
    }
}

impl<'a> IntoIterator for &'a Hdus {
    type Item = &'a Hdu;
    type IntoIter = slice::Iter<'a, Hdu>;

    fn into_iter(self) -> slice::Iter<'a, Hdu> {
        self.iter()
    }
}

/// Outcome of scanning for the next header unit.
#[derive(Debug)]
enum NextHeader {
    /// A complete header unit terminated by an `END` card.
    Found { bytes: Vec<u8>, sum: u32 },
    /// Clean end of stream at a block boundary — no more HDUs.
    End,
    /// Trailing bytes carrying no `END`: special records (§3.5) or a trailing
    /// partial / fill block (§3.6). Disregarded after the last HDU.
    Trailing,
}

/// A header scan's cursor: the source, its staging buffer for a seeking source, and the
/// offset the next header unit starts at.
#[derive(Debug)]
struct Scan<'a, S> {
    source: &'a mut S,
    scratch: &'a mut Vec<u8>,
    offset: u64,
}

impl<S: Source> Scan<'_, S> {
    /// Read one header unit at the offset, advancing it past each consumed block,
    /// until a block carries the `END` record. Blocks come through [`Source::slice`], so
    /// the same scan drives both seeking and in-memory sources.
    fn next_header(&mut self, position: HduPosition) -> Result<NextHeader> {
        let size = self.source.size();
        // Most headers are a single block; reserve it so the common case parses with one
        // allocation and only multi-block headers grow.
        let mut bytes = Vec::with_capacity(BLOCK_SIZE);
        let mut sum = 0;
        loop {
            match size - self.offset {
                // Clean end at a block boundary.
                0 if bytes.is_empty() => return Ok(NextHeader::End),
                // Once a valid SIMPLE/XTENSION prefix was seen, EOF before END is a
                // malformed header rather than ignorable trailing content.
                0 => return Err(FitsError::UnexpectedEof),
                // A sub-block remnant before any header is trailing content (§3.6).
                avail if avail < BLOCK_SIZE as u64 && bytes.is_empty() => {
                    return Ok(NextHeader::Trailing);
                }
                avail if avail < BLOCK_SIZE as u64 => return Err(FitsError::UnexpectedEof),
                _ => {}
            }
            let block = self.source.slice(self.offset, BLOCK_SIZE, self.scratch)?;
            if bytes.is_empty() {
                let keyword = &block[..8];
                match position {
                    HduPosition::Primary if keyword != b"SIMPLE  " => {
                        return Err(FitsError::MissingKeyword {
                            keyword: keywords::SIMPLE,
                        });
                    }
                    // §3.5: a post-HDU block whose first card is not XTENSION begins
                    // special records, regardless of any END-shaped bytes later in it.
                    HduPosition::Extension if keyword != b"XTENSION" => {
                        return Ok(NextHeader::Trailing);
                    }
                    _ => {}
                }
            }
            self.offset += BLOCK_SIZE as u64;
            sum = checksum::accumulate(block, sum);
            allocation::try_reserve(&mut bytes, block.len())?;
            bytes.extend_from_slice(block);
            if block_has_end(block) {
                return Ok(NextHeader::Found { bytes, sum });
            }
        }
    }
}

fn block_has_end(block: &[u8]) -> bool {
    block
        .as_chunks::<CARD_SIZE>()
        .0
        .iter()
        .any(|card| is_end_record(card))
}

#[cfg(test)]
mod tests {
    use crate::block::{BLOCK_SIZE, CARD_SIZE};
    use crate::hdu::HduPosition;
    use crate::internals::cards::fits_file;
    use crate::reader::hdus::{NextHeader, Scan};
    use crate::reader::source::SliceSource;

    #[test]
    fn multi_block_header_scan_keeps_geometric_spare_capacity() {
        let mut cards = vec!["COMMENT"; 2 * BLOCK_SIZE / CARD_SIZE];
        cards[0] = "SIMPLE  = T";
        let bytes = fits_file(&cards, &[]);
        let mut source = SliceSource::new(&bytes);
        let mut scratch = Vec::new();
        let mut scan = Scan {
            source: &mut source,
            scratch: &mut scratch,
            offset: 0,
        };
        let NextHeader::Found { bytes, .. } = scan.next_header(HduPosition::Primary).unwrap()
        else {
            panic!("expected a complete header");
        };
        assert_eq!(bytes.len(), 3 * BLOCK_SIZE);
        assert!(
            bytes.capacity() > bytes.len(),
            "multi-block scanning should grow geometrically"
        );
    }
}
