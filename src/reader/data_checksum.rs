//! [`DataChecksum`]: one HDU's data-unit checksum, summed from the bytes a decode reads.

use crate::checksum;

/// The 32-bit ones'-complement sum of one HDU's block-padded data unit, accumulated as its bytes
/// are read rather than from a copy of the whole unit.
///
/// A decode that reads the unit front to back feeds every byte as it goes, and the finish
/// ([`FitsReader::finish_data_checksum`](crate::FitsReader::finish_data_checksum)) reads only the
/// block fill after it: one pass over the file, no unit-sized buffer. Bytes that do not continue
/// the sum — a section further on, a re-read, a tile of a compressed image — are not fed, and the
/// finish reads the unit from the first byte not summed, in bounded chunks.
#[derive(Debug, Clone)]
pub struct DataChecksum {
    pub(crate) index: usize,
    /// The file offset one past the padded unit.
    end: u64,
    /// The file offset of the first byte not yet summed.
    pub(crate) next: u64,
    sum: u32,
    /// The bytes of a word a read split, waiting for the rest of it.
    partial: [u8; 4],
    partial_len: usize,
}

impl DataChecksum {
    /// An empty sum of HDU `index`'s data unit, `padded` bytes from `start`.
    pub(crate) const fn new(index: usize, start: u64, padded: u64) -> Self {
        Self {
            index,
            end: start + padded,
            next: start,
            sum: 0,
            partial: [0; 4],
            partial_len: 0,
        }
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
    pub(crate) const fn covers(&self, start: u64, padded: u64) -> bool {
        self.end == start + padded && self.next >= start
    }

    /// The bytes still to be read, from [`Self::next`].
    pub(crate) const fn remaining(&self) -> u64 {
        self.end - self.next
    }

    /// The finished sum; every byte of the unit, a whole number of words, has been fed.
    pub(crate) fn sum(&self) -> u32 {
        debug_assert!(self.next == self.end && self.partial_len == 0);
        self.sum
    }
}
