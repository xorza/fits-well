//! The reader's byte source, paired with the staging buffer a seeking source reads
//! through.

#[cfg(feature = "compression")]
use std::mem;
use std::ops::Range;

use crate::bintable::selection::Fetch;
#[cfg(feature = "compression")]
use crate::bintable::selection::{TableRows, TableSelection};
#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
use crate::error::FitsError;
use crate::error::Result;
#[cfg(feature = "compression")]
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_len;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;

/// A [`Source`] and its staging buffer. Kept apart from the HDU records so a read can
/// borrow a header or schema while it fetches bytes.
#[derive(Debug)]
pub(crate) struct DataSource<S> {
    source: S,
    /// Reused staging buffer for the seeking-source reads: a stream source copies
    /// each fetched range here (an in-memory source borrows instead, so this stays
    /// empty). Grows once to the largest range fetched, then holds.
    scratch: Vec<u8>,
    /// The container rows of a tiled-compressed HDU that a read of some of its tiles
    /// selected, compacted.
    #[cfg(feature = "compression")]
    rows: Vec<u8>,
}

impl<S: Source> DataSource<S> {
    pub(crate) const fn new(source: S, scratch: Vec<u8>) -> Self {
        Self {
            source,
            scratch,
            #[cfg(feature = "compression")]
            rows: Vec::new(),
        }
    }

    pub(crate) fn into_source(self) -> S {
        self.source
    }

    /// Fill `out` with the bytes at `offset` — see [`Source::read_into`].
    pub(crate) fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        self.source.read_into(offset, out)
    }

    /// The `len` bytes at `offset` of a resident source, borrowed; `None` for a streaming
    /// one — see [`Source::resident`]. Errors when the range runs past the source.
    pub(crate) fn resident(&self, offset: u64, len: usize) -> Option<Result<&[u8]>> {
        let bytes = self.source.resident()?;
        let range = usize::try_from(offset)
            .ok()
            .and_then(|start| Some(start..start.checked_add(len)?))
            .filter(|range| range.end <= bytes.len());
        Some(
            range
                .map(|range| &bytes[range])
                .ok_or(FitsError::UnexpectedEof),
        )
    }

    /// The bytes at `offset`, read a bounded chunk at a time into this source's staging
    /// buffer (or borrowed from a resident source) and given to `each` with their file
    /// offset.
    pub(crate) fn chunks(
        &mut self,
        offset: u64,
        len: usize,
        mut each: impl FnMut(u64, &[u8]) -> Result<()>,
    ) -> Result<()> {
        /// The bytes read at a time: a bounded scratch, whatever the run's size.
        const CHUNK: usize = 1 << 20;
        if let Some(bytes) = self.resident(offset, len) {
            return each(offset, bytes?);
        }
        let mut at = 0;
        while at < len {
            let take = CHUNK.min(len - at);
            let chunk_offset = offset_at(offset, at)?;
            let bytes = self.slice(chunk_offset, take)?;
            each(chunk_offset, bytes)?;
            at += take;
        }
        Ok(())
    }

    /// The `len` bytes at `offset`, borrowed — see [`Source::slice`].
    pub(crate) fn slice(&mut self, offset: u64, len: usize) -> Result<&[u8]> {
        self.source.slice(offset, len, &mut self.scratch)
    }

    /// A [`Fetch`] of `hdu`'s data unit.
    pub(crate) const fn unit(&mut self, hdu: &Hdu) -> UnitFetch<'_, S> {
        UnitFetch {
            data: self,
            data_offset: hdu.data_offset,
        }
    }

    /// The data bytes of `hdu`'s data unit, without its fill, in a fresh owned buffer.
    pub(crate) fn data_unit(&mut self, hdu: &Hdu) -> Result<Vec<u8>> {
        let mut bytes = Vec::new();
        self.source
            .read_append(hdu.data_offset, data_len(hdu.data_bytes)?, &mut bytes)?;
        Ok(bytes)
    }

    /// The binary table `hdu` holds, viewed in place over the source (or this
    /// source's staging buffer), its bytes fed to `sum`, when given, as they are read.
    #[cfg(feature = "compression")]
    pub(crate) fn table<'a>(
        &'a mut self,
        hdu: &'a Hdu,
        sum: Option<&mut DataChecksum>,
    ) -> Result<TableView<'a>> {
        let schema = hdu.container_schema()?;
        let len = usize::try_from(hdu.data_bytes).map_err(|_| FitsError::DataUnitTooLarge {
            bytes: hdu.data_bytes,
        })?;
        let bytes = self.slice(hdu.data_offset, len)?;
        if let Some(sum) = sum {
            sum.feed(hdu.data_offset, bytes);
        }
        TableView::new(schema, bytes)
    }

    /// The rows `rows` of the binary table `hdu` holds, compacted into this source's
    /// buffer of selected rows — see [`TableSelection::copy_into`].
    #[cfg(feature = "compression")]
    pub(crate) fn table_rows<'a>(
        &'a mut self,
        hdu: &'a Hdu,
        rows: TableRows<'_>,
    ) -> Result<TableView<'a>> {
        let schema = hdu.container_schema()?;
        let row_count = rows.len();
        let selection = TableSelection {
            schema,
            rows,
            columns: None,
        };
        let mut out = mem::take(&mut self.rows);
        let copied = selection.copy_into(&mut self.unit(hdu), &mut out);
        self.rows = out;
        copied?;
        Ok(TableView::compact(schema, row_count, &self.rows))
    }
}

/// One data unit of a [`DataSource`], fetched by ranges relative to its start.
#[derive(Debug)]
pub(crate) struct UnitFetch<'d, S> {
    data: &'d mut DataSource<S>,
    data_offset: u64,
}

impl<S: Source> Fetch for UnitFetch<'_, S> {
    fn append(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()> {
        let offset = offset_at(self.data_offset, range.start)?;
        self.data.source.read_append(offset, range.len(), out)
    }

    fn bytes(&mut self, range: Range<usize>) -> Result<&[u8]> {
        let offset = offset_at(self.data_offset, range.start)?;
        self.data.slice(offset, range.len())
    }
}

/// `base + delta`, rejecting a sum that cannot address a source byte.
pub(crate) fn offset_at(base: u64, delta: usize) -> Result<u64> {
    u64::try_from(delta)
        .ok()
        .and_then(|delta| base.checked_add(delta))
        .ok_or(FitsError::DataUnitOverflow)
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::reader::data_source::DataSource;

    /// The staging buffer a seeking source reads through.
    pub(crate) fn scratch<S>(data: &DataSource<S>) -> &Vec<u8> {
        &data.scratch
    }
}
