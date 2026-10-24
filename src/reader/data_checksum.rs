//! [`DataChecksum`]: one HDU's data-unit checksum, summed from the bytes a decode reads.

use crate::checksum::checksum_report::ChecksumReport;
use crate::checksum::declared::Declared;
use crate::checksum::running::Running;
use crate::error::Result;
use crate::reader::data_source::DataSource;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;

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
    sum: Running,
}

impl DataChecksum {
    /// An empty sum of `hdu`'s data unit.
    pub(crate) const fn of(hdu: &Hdu) -> Self {
        Self {
            end: hdu.data_offset + hdu.padded_data_bytes(),
            next: hdu.data_offset,
            sum: Running::new(),
        }
    }

    /// Read whatever of `hdu`'s data unit this has not summed, in bounded chunks, and judge
    /// the HDU's `DATASUM` and `CHECKSUM` against it, by [`Declared`]'s rule. The unit is
    /// read only when a keyword needs its sum.
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
        debug_assert!(
            self.covers(hdu.data_offset, hdu.padded_data_bytes()),
            "a data checksum of another HDU"
        );
        Declared::read(&hdu.header).report(hdu.header_sum, || {
            while self.remaining() > 0 {
                let offset = self.next;
                let len = usize::try_from(self.remaining().min(CHUNK)).expect("a chunk fits usize");
                let bytes = data.slice(offset, len)?;
                self.feed(offset, bytes);
            }
            debug_assert_eq!(self.next, self.end);
            Ok(self.sum.finish())
        })
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
        self.next += take as u64;
        self.sum.update(&bytes[..take]);
    }

    /// Whether this sums the unit of `padded` bytes from `start`.
    const fn covers(&self, start: u64, padded: u64) -> bool {
        self.end == start + padded && self.next >= start
    }

    /// The bytes still to be read, from [`Self::next`].
    const fn remaining(&self) -> u64 {
        self.end - self.next
    }
}
