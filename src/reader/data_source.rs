//! The reader's byte source, paired with the staging buffer a seeking source reads
//! through.

#[cfg(feature = "compression")]
use std::mem;
use std::ops::Range;

#[cfg(feature = "compression")]
use crate::allocation;
use crate::bintable::selection::Fetch;
#[cfg(feature = "compression")]
use crate::bintable::selection::{TableRows, TableSelection};
#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_index::TileIndex;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
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

    /// The tiles `rows` of the tiled image `hdu` holds, whose container's main table is
    /// `main`, compacted into this source's buffer of selected rows with the heap arrays
    /// they address — see [`TableSelection::copy_into`]. Only the arrays are fetched, and
    /// the buffer grows to just what they and the rows take. A stream feeds each to `sum`
    /// as it is read, so the finish need not read it again; a resident source, which the
    /// finish sums in place with no read, does not, as summing between the decodes of
    /// sections leaves the workers waiting.
    #[cfg(feature = "compression")]
    pub(crate) fn tile_rows<'a>(
        &'a mut self,
        hdu: &'a Hdu,
        main: &[u8],
        rows: &[usize],
        sum: Option<&mut DataChecksum>,
    ) -> Result<TableView<'a>> {
        let schema = hdu.container_schema()?;
        let len = tile_rows_len(hdu, main, rows)?;
        let mut out = mem::take(&mut self.rows);
        out.clear();
        let copied = allocation::try_grow_to(&mut out, len).and_then(|()| {
            let sum = if self.source.resident().is_some() {
                None
            } else {
                sum
            };
            let mut fetch = IndexedUnit {
                main,
                unit: self.unit(hdu),
                sum,
            };
            TableSelection {
                schema,
                rows: TableRows::Picked(rows),
                columns: None,
            }
            .copy_into(&mut fetch, &mut out)
        });
        self.rows = out;
        copied?;
        Ok(TableView::compact(schema, rows.len(), &self.rows))
    }

    /// The container of the tiled image `hdu`, whose main table `index` holds: viewed in
    /// place over a resident source, or for a stream the main table copied from the index
    /// and the heap read after it into this source's staging buffer. The heap is fed to
    /// `sum`, which summed the index when it began.
    #[cfg(feature = "compression")]
    pub(crate) fn tiled_table<'a>(
        &'a mut self,
        hdu: &'a Hdu,
        index: &TileIndex,
        sum: Option<&mut DataChecksum>,
    ) -> Result<TableView<'a>> {
        let schema = hdu.container_schema()?;
        let len = data_len(hdu.data_bytes)?;
        let main = index.rows();
        let heap_len = len
            .checked_sub(main.len())
            .ok_or(FitsError::UnexpectedEof)?;
        let heap_offset = offset_at(hdu.data_offset, main.len())?;
        if self.source.resident().is_none() {
            self.scratch.clear();
            allocation::try_grow_to(&mut self.scratch, len)?;
            self.scratch.extend_from_slice(main);
            self.source
                .read_append(heap_offset, heap_len, &mut self.scratch)?;
        }
        let bytes = match self.resident(hdu.data_offset, len) {
            Some(bytes) => bytes?,
            None => &self.scratch,
        };
        if let Some(sum) = sum {
            sum.feed(heap_offset, &bytes[main.len()..]);
        }
        TableView::new(schema, bytes)
    }

    /// The bytes this source's buffer of selected rows can hold before it grows.
    #[cfg(feature = "compression")]
    pub(crate) const fn rows_capacity(&self) -> usize {
        self.rows.capacity()
    }

    /// The bytes this source's staging buffer can hold before it grows.
    pub(crate) const fn staging_capacity(&self) -> usize {
        self.scratch.capacity()
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

/// A tiled container's data unit, its main table served from the rows the reader holds
/// and its heap fetched from the source.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct IndexedUnit<'m, 'd, 's, S> {
    main: &'m [u8],
    unit: UnitFetch<'d, S>,
    /// The data unit's sum, fed each heap range that continues what it summed.
    sum: Option<&'s mut DataChecksum>,
}

#[cfg(feature = "compression")]
impl<S: Source> Fetch for IndexedUnit<'_, '_, '_, S> {
    fn append(&mut self, range: Range<usize>, out: &mut Vec<u8>) -> Result<()> {
        let split = range.end.min(self.main.len()).max(range.start);
        if range.start < split {
            out.extend_from_slice(&self.main[range.start..split]);
        }
        if split < range.end {
            let start = out.len();
            self.unit.append(split..range.end, out)?;
            if let Some(sum) = self.sum.as_deref_mut() {
                sum.feed(offset_at(self.unit.data_offset, split)?, &out[start..]);
            }
        }
        Ok(())
    }

    fn bytes(&mut self, range: Range<usize>) -> Result<&[u8]> {
        if range.end <= self.main.len() {
            return Ok(&self.main[range]);
        }
        assert!(
            range.start >= self.main.len(),
            "a cell lies in the main table or in the heap"
        );
        self.unit.bytes(range)
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

/// The bytes [`DataSource::tile_rows`] holds for the tiles `rows` of the tiled image
/// `hdu`, whose container's main table is `main`.
#[cfg(feature = "compression")]
pub(crate) fn tile_rows_len(hdu: &Hdu, main: &[u8], rows: &[usize]) -> Result<usize> {
    TableSelection {
        schema: hdu.container_schema()?,
        rows: TableRows::Picked(rows),
        columns: None,
    }
    .copied_len(main)
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
