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
#[cfg(feature = "compression")]
use crate::compression::table::{ContainerTiles, DecodedRows, TiledTable};
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::hdu::Hdu;
#[cfg(feature = "compression")]
use crate::reader::data_checksum::DataChecksum;
use crate::reader::source::{Source, check_range};

/// The bytes a chunked read takes at a time, [`DataSource::chunks`]: a bounded staging
/// buffer, whatever the unit's size.
pub(crate) const CHUNK: usize = 1 << 20;

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
    /// The rows a read of a tiled-compressed table last decoded, kept for a later read
    /// of the same tiles and columns, as a loop over the cells of one tile reads them.
    #[cfg(feature = "compression")]
    decoded: Option<DecodedTiles>,
}

/// A read of some rows and columns of the table a tiled-compressed HDU holds: the HDU, its
/// tile layout, the rows, and the columns, every column when `None`.
#[cfg(feature = "compression")]
#[derive(Debug, Clone, Copy)]
pub(crate) struct TiledRead<'a> {
    pub(crate) hdu: &'a Hdu,
    pub(crate) tiled: &'a TiledTable,
    pub(crate) rows: &'a Range<usize>,
    pub(crate) columns: Option<&'a [bool]>,
}

/// The rows a read decoded: the HDU, its tiles and the columns it decoded, every column
/// when `None`.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct DecodedTiles {
    hdu: usize,
    tiles: Range<usize>,
    columns: Option<Vec<bool>>,
    rows: DecodedRows,
}

impl<S: Source> DataSource<S> {
    pub(crate) const fn new(source: S) -> Self {
        Self {
            source,
            scratch: Vec::new(),
            #[cfg(feature = "compression")]
            rows: Vec::new(),
            #[cfg(feature = "compression")]
            decoded: None,
        }
    }

    pub(crate) fn into_source(self) -> S {
        self.source
    }

    /// Fill `out` with the bytes at `offset` — see [`Source::read_into`].
    pub(crate) fn read_into(&mut self, offset: u64, out: &mut [u8]) -> Result<()> {
        self.source.read_into(offset, out)
    }

    /// Whether the source is resident in memory — see [`Source::resident`]: a read borrows
    /// its bytes, where a stream copies them through the staging buffer first.
    pub(crate) fn is_resident(&self) -> bool {
        self.source.resident().is_some()
    }

    /// The `len` bytes at `offset` of a resident source, borrowed. Errors when the range
    /// runs past the source.
    pub(crate) fn resident_range(&self, offset: u64, len: usize) -> Result<&[u8]> {
        let bytes = self.source.resident().expect("a resident source");
        check_range(offset, len, bytes.len() as u64)?;
        let start = usize::try_from(offset).expect("an offset inside a slice");
        Ok(&bytes[start..start + len])
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
        if self.is_resident() {
            return each(offset, self.resident_range(offset, len)?);
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

    /// The length of `hdu`'s data unit, once it is known to lie inside the source: a size
    /// the header claims past the end is `UnexpectedEof` here, before any read sizes a
    /// buffer from it.
    pub(crate) fn unit_len(&self, hdu: &Hdu) -> Result<usize> {
        let len = usize::try_from(hdu.data_bytes).map_err(|_| FitsError::DataUnitTooLarge {
            bytes: hdu.data_bytes,
        })?;
        check_range(hdu.data_offset, len, self.source.size())?;
        Ok(len)
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
        let len = self.unit_len(hdu)?;
        self.source.read_append(hdu.data_offset, len, &mut bytes)?;
        Ok(bytes)
    }

    /// The binary table `hdu` holds, viewed in place over the source (or this
    /// source's staging buffer).
    #[cfg(feature = "compression")]
    pub(crate) fn table<'a>(&'a mut self, hdu: &'a Hdu) -> Result<TableView<'a>> {
        let schema = hdu.container_schema()?;
        let len = self.unit_len(hdu)?;
        TableView::new(schema, self.slice(hdu.data_offset, len)?)
    }

    /// The tiles of the table `read` names that its rows touch, decoded into `buffer` with
    /// the heap arrays they reference, of the columns it selects. A tile's arrays have
    /// descriptors that address the whole container heap, and a read of every tile reads
    /// the whole container: then the container is read whole, else only the touched tiles.
    #[cfg(feature = "compression")]
    pub(crate) fn decode(&mut self, read: TiledRead<'_>, buffer: Vec<u8>) -> Result<DecodedRows> {
        let TiledRead {
            hdu,
            tiled,
            rows,
            columns,
        } = read;
        let tiles = tiled.tiles(rows);
        let all = 0..hdu.container_schema()?.extent.row_count;
        let container = if tiled.has_arrays(columns) || tiles == all {
            ContainerTiles {
                view: self.table(hdu)?,
                first_tile: 0,
            }
        } else {
            ContainerTiles {
                first_tile: tiles.start,
                view: self.table_rows(hdu, TableRows::Range(tiles))?,
            }
        };
        tiled.decode(container, rows, columns, buffer)
    }

    /// [`Self::decode`] of `read`, the rows the last decode gave when it read the same
    /// tiles and columns, else decoded now into their buffer and kept.
    #[cfg(feature = "compression")]
    pub(crate) fn decoded(&mut self, read: TiledRead<'_>) -> Result<&DecodedRows> {
        let tiles = read.tiled.tiles(read.rows);
        let hit = self.decoded.as_ref().is_some_and(|held| {
            held.hdu == read.hdu.index
                && held.tiles == tiles
                && held.columns.as_deref() == read.columns
        });
        if !hit {
            let buffer = self
                .decoded
                .take()
                .map(|held| held.rows.data)
                .unwrap_or_default();
            let rows = self.decode(read, buffer)?;
            self.decoded = Some(DecodedTiles {
                hdu: read.hdu.index,
                tiles,
                columns: read.columns.map(<[bool]>::to_vec),
                rows,
            });
        }
        let Some(held) = &self.decoded else {
            unreachable!("held or decoded above")
        };
        Ok(&held.rows)
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
            let sum = if self.is_resident() { None } else { sum };
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
        let len = self.unit_len(hdu)?;
        let main = index.rows();
        let heap_len = len
            .checked_sub(main.len())
            .ok_or(FitsError::UnexpectedEof)?;
        let heap_offset = offset_at(hdu.data_offset, main.len())?;
        let resident = self.is_resident();
        if !resident {
            self.scratch.clear();
            allocation::try_grow_to(&mut self.scratch, len)?;
            self.scratch.extend_from_slice(main);
            self.source
                .read_append(heap_offset, heap_len, &mut self.scratch)?;
        }
        let bytes = if resident {
            self.resident_range(hdu.data_offset, len)?
        } else {
            &self.scratch
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
