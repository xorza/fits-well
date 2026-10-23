//! [`DataChecksum`]: one HDU's data-unit checksum, summed from the bytes a decode reads.

use crate::checksum;
use crate::error::Result;
use crate::header_model::keywords;
use crate::reader::data_source::DataSource;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;
use crate::reader::{ChecksumReport, ChecksumStatus};

/// The 32-bit ones'-complement sum of one HDU's block-padded data unit, accumulated as its bytes
/// are read rather than from a copy of the whole unit.
///
/// A decode that reads the unit front to back feeds every byte as it goes, and
/// [`DataChecksum::finish`] reads only the block fill after it: one pass over the file, no
/// unit-sized buffer. Bytes that do not continue
/// the sum — a section further on, a re-read, a tile of a compressed image — are not fed, and the
/// finish reads the unit from the first byte not summed, in bounded chunks.
#[derive(Debug, Clone)]
pub(crate) struct DataChecksum {
    /// The file offset one past the padded unit.
    end: u64,
    /// The file offset of the first byte not yet summed.
    next: u64,
    sum: u32,
    /// The bytes of a word a read split, waiting for the rest of it.
    partial: [u8; 4],
    partial_len: usize,
}

impl DataChecksum {
    /// An empty sum of `hdu`'s data unit.
    pub(crate) const fn of(hdu: &Hdu) -> Self {
        Self {
            end: hdu.data_offset + hdu.padded_data_bytes(),
            next: hdu.data_offset,
            sum: 0,
            partial: [0; 4],
            partial_len: 0,
        }
    }

    /// Read whatever of `hdu`'s data unit this has not summed, in bounded chunks, and judge
    /// the HDU's `DATASUM` and `CHECKSUM` against it. Strings of one or more blanks are
    /// reported as [`ChecksumStatus::Unknown`], as the standard reserves them for undefined
    /// or unknown values.
    ///
    /// # Panics
    ///
    /// If this is not [`DataChecksum::of`] `hdu`.
    pub(crate) fn finish<S: Source>(
        mut self,
        hdu: &Hdu,
        data: &mut DataSource<S>,
    ) -> Result<ChecksumReport> {
        /// The bytes read at a time: a bounded scratch, whatever the unit's size.
        const CHUNK: u64 = 1 << 20;
        assert!(
            self.covers(hdu.data_offset, hdu.padded_data_bytes()),
            "a data checksum of another HDU"
        );
        let stored_datasum = hdu.header.get_text(keywords::DATASUM)?;
        let stored_checksum = hdu.header.get_text(keywords::CHECKSUM)?;
        let expected_datasum = stored_datasum
            .filter(|value| !value.is_empty() && !is_unknown(value))
            .and_then(|value| value.trim().parse::<u32>().ok());
        let verify_whole_hdu =
            stored_checksum.is_some_and(|value| !value.is_empty() && !is_unknown(value));
        let mut report = ChecksumReport {
            datasum: initial_status(stored_datasum),
            checksum: initial_status(stored_checksum),
        };
        if expected_datasum.is_none() && !verify_whole_hdu {
            return Ok(report);
        }
        while self.remaining() > 0 {
            let offset = self.next;
            let len = usize::try_from(self.remaining().min(CHUNK)).expect("a chunk fits usize");
            let bytes = data.slice(offset, len)?;
            self.feed(offset, bytes);
        }
        let data_sum = self.sum();
        if let Some(expected) = expected_datasum {
            report.datasum = if expected == data_sum {
                ChecksumStatus::Valid
            } else {
                ChecksumStatus::Invalid
            };
        }
        if verify_whole_hdu {
            report.checksum = if checksum::combine(hdu.header_sum, data_sum) == 0xFFFF_FFFF {
                ChecksumStatus::Valid
            } else {
                ChecksumStatus::Invalid
            };
        }
        Ok(report)
    }

    /// Sum `bytes`, read from file offset `offset`, when they continue what is summed; anything
    /// else waits for the finish.
    pub(crate) fn feed(&mut self, offset: u64, bytes: &[u8]) {
        if offset != self.next || self.next >= self.end {
            return;
        }
        let take = bytes
            .len()
            .min(usize::try_from(self.end - self.next).unwrap_or(usize::MAX));
        let mut bytes = &bytes[..take];
        self.next += take as u64;
        if self.partial_len > 0 {
            let fill = (4 - self.partial_len).min(bytes.len());
            self.partial[self.partial_len..self.partial_len + fill].copy_from_slice(&bytes[..fill]);
            self.partial_len += fill;
            bytes = &bytes[fill..];
            if self.partial_len < 4 {
                return;
            }
            self.sum = checksum::combine(self.sum, u32::from_be_bytes(self.partial));
            self.partial_len = 0;
        }
        let whole = bytes.len() - bytes.len() % 4;
        self.sum = checksum::accumulate(&bytes[..whole], self.sum);
        let tail = &bytes[whole..];
        self.partial[..tail.len()].copy_from_slice(tail);
        self.partial_len = tail.len();
    }

    /// Whether this sums the unit of `padded` bytes from `start`.
    const fn covers(&self, start: u64, padded: u64) -> bool {
        self.end == start + padded && self.next >= start
    }

    /// The bytes still to be read, from [`Self::next`].
    const fn remaining(&self) -> u64 {
        self.end - self.next
    }

    /// The finished sum; every byte of the unit, a whole number of words, has been fed.
    fn sum(&self) -> u32 {
        debug_assert!(self.next == self.end && self.partial_len == 0);
        self.sum
    }
}

/// Where a stored integrity keyword starts before anything is recomputed: absent,
/// the standard all-blank "unknown", or provisionally invalid until it verifies.
fn initial_status(stored: Option<&str>) -> ChecksumStatus {
    match stored {
        None => ChecksumStatus::Absent,
        Some(value) if is_unknown(value) => ChecksumStatus::Unknown,
        Some(_) => ChecksumStatus::Invalid,
    }
}

fn is_unknown(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte == b' ')
}
