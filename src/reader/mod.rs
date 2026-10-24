use std::io::Read;
use std::io::Seek;
use std::ops::Range;

use crate::alternate::Alternate;
use crate::ascii::Ascii;
use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
#[cfg(feature = "compression")]
use crate::bintable::selection::validate_row_range;
use crate::bintable::selection::{Selection, TableCell, TableRows, TableSelection};
use crate::bintable::table_view::TableView;
use crate::checksum::checksum_report::ChecksumReport;
use crate::column::ColumnSelector;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_index::TileIndex;
#[cfg(feature = "compression")]
use crate::compression::table::{ContainerTiles, DecodedRows};
use crate::fits_error::ExpectedHdu;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::DataSource;
use crate::reader::hdu::Hdu;
use crate::reader::hdu_selector::HduSelector;
use crate::reader::hdus::Hdus;
use crate::reader::image_reader::{ImageReader, PlainRead, StoredRuns};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular;

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod data_checksum;
mod data_source;
pub(crate) mod hdu;
pub(crate) mod hdu_selector;
pub(crate) mod hdus;
pub(crate) mod image_reader;
pub(crate) mod source;

use source::SliceSource;
use source::Source;
use source::StreamSource;

/// A FITS file opened over a seekable byte source. Opening scans HDU boundaries
/// from headers alone (no data is read); data units are fetched on demand.
#[derive(Debug)]
pub struct FitsReader<S> {
    /// The scanned HDU records; exposed read-only via [`FitsReader::hdus`].
    pub(crate) hdus: Hdus,
    data: DataSource<S>,
    scratch: Scratch,
}

/// The reader's reused buffers a section or selection read fills and the result
/// borrows: the selected section's shape, the tile rows it touches and the odometer
/// that walks them, a compacted table selection with the columns it selects, the words
/// a compressed image's physical read decodes into, and the last compressed image's
/// tile index, which every handle on that image reuses.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    pub(crate) section_shape: Vec<usize>,
    #[cfg(feature = "compression")]
    pub(crate) tile_rows: Vec<usize>,
    #[cfg(feature = "compression")]
    pub(crate) tile_coordinates: Vec<usize>,
    #[cfg(feature = "compression")]
    pub(crate) tile_index: TileIndex,
    pub(crate) selection: Vec<u8>,
    pub(crate) column_indices: Vec<usize>,
    pub(crate) column_mask: Vec<bool>,
    #[cfg(feature = "compression")]
    pub(crate) words: Vec<u64>,
}

/// A [`FitsReader`] over a seeking byte source (`Read + Seek`, e.g. a `File`) — the
/// type [`FitsReader::open`] returns. A friendlier name for `FitsReader<StreamSource<R>>`.
pub type StreamReader<R> = FitsReader<StreamSource<R>>;

/// A [`FitsReader`] over an in-memory byte slice — the type [`FitsReader::from_bytes`]
/// returns. The lifetime is that of the borrowed bytes.
pub type SliceReader<'a> = FitsReader<SliceSource<'a>>;

impl<R: Read + Seek> FitsReader<StreamSource<R>> {
    /// Open a seekable byte source (file, cursor). Data units are copied into the
    /// reader's scratch on demand; for an in-memory file prefer
    /// [`FitsReader::from_bytes`], which decodes straight from the bytes.
    pub fn open(source: R) -> Result<Self> {
        Self::from_source(StreamSource::new(source)?)
    }

    /// Consume the reader and recover the underlying seekable input.
    pub fn into_inner(self) -> R {
        self.data.into_source().into_inner()
    }
}

impl<'a> FitsReader<SliceSource<'a>> {
    /// Open an in-memory FITS file — the whole thing as a byte slice: bytes already in
    /// RAM, or a memory map the caller holds, which is how to read a mapped file. Data
    /// units decode straight from the borrowed bytes with no staging copy, and no
    /// scratch allocation.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<Self> {
        FitsReader::from_source(SliceSource::new(bytes))
    }

    /// Consume the reader and recover the original borrowed byte slice.
    pub fn into_bytes(self) -> &'a [u8] {
        self.data.into_source().into_bytes()
    }
}

impl<S: Source> FitsReader<S> {
    /// Scan the whole HDU sequence, parsing every header and recording the byte
    /// range of each data unit — without reading any data.
    fn from_source(mut source: S) -> Result<Self> {
        let mut scratch = Vec::new();
        let hdus = Hdus::scan(&mut source, &mut scratch)?;
        Ok(Self {
            hdus,
            data: DataSource::new(source, scratch),
            scratch: Scratch::default(),
        })
    }

    /// The scanned HDU records, read-only and in file order — each carrying its
    /// parsed [`Header`](crate::header::Header) and [`HduKind`]. Every read names its
    /// HDU with an [`HduSelector`]: an index into this slice, `"SCI"`, or `("SCI", 2)`.
    pub fn hdus(&self) -> &[Hdu] {
        self.hdus.as_slice()
    }

    /// The HDU `hdu` names: the first that [`Hdu::matches`] it. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last HDU, and with
    /// [`FitsError::HduNotFound`] for a name no HDU carries.
    pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu> {
        self.hdus.select(hdu.into())
    }

    /// Every HDU [`FitsReader::image`] can read as an image: image extensions,
    /// tiled-compressed images, and a non-empty primary array (an empty `NAXIS = 0`
    /// primary is a container, not an image, and is skipped). The first image is
    /// `images().next()`, and its [`Hdu::index`] selects it.
    pub fn images(&self) -> impl Iterator<Item = &Hdu> + '_ {
        self.hdus.images()
    }

    /// Read the raw, still-encoded (big-endian, unscaled) data unit into a fresh,
    /// caller-owned buffer: its [`Hdu::data_bytes`](Hdu#structfield.data_bytes), without
    /// the block fill after them.
    ///
    /// This is the owned form, backing the table readers (which keep the bytes as
    /// the parsed table's storage). Image and random-groups reads instead stage
    /// through the reader's reused internal scratch — see [`FitsReader::image`].
    pub fn read_data_unit<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Vec<u8>> {
        let hdu = self.hdus.select(hdu.into())?;
        self.data.data_unit(hdu)
    }

    /// The image HDU `index` holds, plain or tiled-compressed alike, ready to read a
    /// region at a time. Errors with [`FitsError::HduKindMismatch`] for tables, random
    /// groups and unmodelled extensions, a compressed image among them without the
    /// `compression` feature.
    pub fn image<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ImageReader<'_, S>> {
        let hdu = self.hdus.select(hdu.into())?;
        ImageReader::new(hdu, &mut self.data, &mut self.scratch)
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`Table::column`]. Errors as
    /// [`Hdu::table_schema`] does for an HDU that is not a plain binary table.
    pub fn read_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Table> {
        let hdu = self.hdus.select(hdu.into())?;
        let schema = hdu.table_schema()?;
        #[cfg(feature = "compression")]
        if let Some(decoded) =
            decoded_rows(&mut self.data, hdu, &(0..schema.extent.row_count), None)?
        {
            return Table::new(decoded.schema, decoded.data);
        }
        Table::new(schema.clone(), self.data.data_unit(hdu)?)
    }

    /// Materialize only the requested contiguous row range, including only the VLA
    /// heap cells referenced by those rows.
    pub fn read_table_rows<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: Range<usize>,
    ) -> Result<Table> {
        let hdu = self.hdus.select(hdu.into())?;
        let schema = hdu.table_schema()?;
        let row_count = rows.len();
        let mut data = Vec::new();
        select_rows(&mut self.data, hdu, rows, None, &mut data)?;
        Table::new(schema.compacted(row_count, data.len()), data)
    }

    /// Decode selected columns over a row range without reading unrelated rows.
    pub fn read_table_columns<'h, 'c>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: Range<usize>,
        columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>,
    ) -> Result<Selection> {
        let mut selection = Selection {
            rows: 0..0,
            columns: Vec::new(),
        };
        self.read_table_columns_into(hdu, rows, columns, &mut selection)?;
        Ok(selection)
    }

    /// [`Self::read_table_columns`] into `out`, whose buffers it refills: a read of the
    /// same columns over as many rows as the last allocates nothing for a plain table.
    /// On an error, `out` holds no column.
    pub fn read_table_columns_into<'h, 'c>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: Range<usize>,
        columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>,
        out: &mut Selection,
    ) -> Result<()> {
        let filled = self.hdus.select(hdu.into()).and_then(|hdu| {
            let schema = hdu.table_schema()?;
            self.scratch.column_indices.clear();
            for selector in columns {
                let index = selector.into().resolve(&schema.columns)?;
                self.scratch.column_indices.push(index);
            }
            let Scratch {
                selection,
                column_indices,
                column_mask,
                ..
            } = &mut self.scratch;
            column_mask.clear();
            column_mask.resize(schema.columns.len(), false);
            for &index in column_indices.iter() {
                column_mask[index] = true;
            }
            select_rows(
                &mut self.data,
                hdu,
                rows.clone(),
                Some(column_mask),
                selection,
            )?;
            let table = TableView::compact(schema, rows.len(), selection);
            out.refill(table, rows, column_indices)
        });
        if filled.is_err() {
            out.rows = 0..0;
            out.columns.clear();
        }
        filled
    }

    /// Decode one fixed or variable-length table cell without materializing other rows.
    pub fn read_table_cell<'h, 'c>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        row: usize,
        column: impl Into<ColumnSelector<'c>>,
    ) -> Result<ColumnData> {
        let hdu = self.hdus.select(hdu.into())?;
        let schema = hdu.table_schema()?;
        let column = column.into().resolve(&schema.columns)?;
        #[cfg(feature = "compression")]
        {
            let mut selected = vec![false; schema.columns.len()];
            selected[column] = true;
            let rows = row..row.saturating_add(1);
            if let Some(decoded) = decoded_rows(&mut self.data, hdu, &rows, Some(&selected))? {
                let cell = TableCell {
                    schema: &decoded.schema,
                    row: row - decoded.first_row,
                    column,
                };
                return cell.read(&mut decoded.data.as_slice());
            }
        }
        let cell = TableCell {
            schema,
            row,
            column,
        };
        cell.read(&mut self.data.unit(hdu))
    }

    /// Parse an image HDU's WCS and resolve any standard `-TAB` coordinate arrays from
    /// their referenced `BINTABLE` extensions, plain or tiled-compressed. Header-only WCS
    /// descriptions use the same path as [`Wcs::from_header`]; this method is required
    /// for `-TAB` because its coordinate values and optional index vectors live outside
    /// the source header. It resolves `-TAB` for an image header only: a binary table's
    /// own coordinate columns are not read here. A `-TAB` extension the file does not
    /// hold is [`FitsError::HduNotFound`].
    pub fn read_wcs<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Wcs> {
        let alt = Alternate::of(alt)?;
        let header = &self.hdus.select(hdu.into())?.header;
        let descriptors = tabular::descriptors(header, Wcs::image_axis_count(header, alt)?, alt)?;
        if descriptors.is_empty() {
            return Wcs::from_header(header, alt);
        }
        let transform_count = descriptors.len();
        let mut groups = Vec::<Vec<tabular::TabularDescriptor>>::new();
        for descriptor in descriptors {
            match groups.iter_mut().find(|group| {
                group[0]
                    .reference
                    .identifies_same_extension(&descriptor.reference)
            }) {
                Some(group) => group.push(descriptor),
                None => groups.push(vec![descriptor]),
            }
        }
        let mut transforms = Vec::with_capacity(transform_count);
        for group in groups {
            let reference = &group[0].reference;
            let table_hdu = self.hdus.select(HduSelector::Extension {
                name: &reference.extension_name,
                version: Some(reference.extension_version),
                level: Some(reference.extension_level),
            })?;
            let schema = table_hdu.table_schema()?;
            let mut selected = vec![false; schema.columns.len()];
            for descriptor in &group {
                for name in descriptor.referenced_columns() {
                    selected[schema.column_index_checked(name)?] = true;
                }
            }
            let rows = 0..usize::from(schema.extent.row_count != 0);
            let row_count = rows.len();
            select_rows(
                &mut self.data,
                table_hdu,
                rows,
                Some(&selected),
                &mut self.scratch.selection,
            )?;
            let table = TableView::compact(schema, row_count, &self.scratch.selection);
            for descriptor in group {
                transforms.push(tabular::TabularTransform::from_table(&descriptor, table)?);
            }
        }
        Wcs::from_header_with_tabular(header, alt, transforms)
    }

    /// Read an `TABLE` (ASCII table) extension and parse its column structure.
    /// Errors with [`FitsError::HduKindMismatch`] for any other HDU.
    pub fn read_ascii_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Ascii> {
        let hdu = self.hdus.select(hdu.into())?;
        let schema = hdu.ascii_schema()?.clone();
        Ascii::new(schema, self.data.data_unit(hdu)?)
    }

    /// Read and decode a random-groups primary array (§6). Errors with
    /// [`FitsError::HduKindMismatch`] for any other HDU.
    pub fn read_groups<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<RandomGroups> {
        let hdu = self.hdus.select(hdu.into())?;
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::HduKindMismatch {
                expected: ExpectedHdu::RandomGroups,
                got: hdu.kind,
            });
        }
        let runs = StoredRuns::All {
            len: data_len(hdu.data_bytes)?,
            shape: &[],
        };
        let mut read = PlainRead {
            data: &mut self.data,
            data_offset: hdu.data_offset,
            sum: None,
        };
        let samples = read.data(runs, hdu.header.bitpix()?)?;
        RandomGroups::from_samples(&hdu.header, samples)
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J). An empty or
    /// all-blank value is [`ChecksumStatus::Unknown`], as the standard reserves it for an
    /// undefined or unknown checksum.
    ///
    /// [`ChecksumStatus::Unknown`]: crate::io::ChecksumStatus::Unknown
    pub fn verify_checksum<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
    ) -> Result<ChecksumReport> {
        let hdu = self.hdus.select(hdu.into())?;
        DataChecksum::of(hdu).finish(hdu, &mut self.data)
    }
}

/// Select `rows` and `columns` (every column when `None`) of the binary table `hdu` holds
/// into `out`, as [`TableSelection::copy_into`] does: from the source for a plain table, from
/// the decoded tiles for a tiled-compressed one.
fn select_rows<S: Source>(
    data: &mut DataSource<S>,
    hdu: &Hdu,
    rows: Range<usize>,
    columns: Option<&[bool]>,
    out: &mut Vec<u8>,
) -> Result<()> {
    #[cfg(feature = "compression")]
    if let Some(decoded) = decoded_rows(data, hdu, &rows, columns)? {
        let rows = rows.start - decoded.first_row..rows.end - decoded.first_row;
        let selection = TableSelection {
            schema: &decoded.schema,
            rows: TableRows::Range(rows),
            columns,
        };
        return selection.copy_into(&mut decoded.data.as_slice(), out);
    }
    let selection = TableSelection {
        schema: hdu.table_schema()?,
        rows: TableRows::Range(rows),
        columns,
    };
    selection.copy_into(&mut data.unit(hdu), out)
}

/// The tiles of the table a tiled-compressed `hdu` holds that `rows` touch, decoded with
/// the table's heap, of the `columns` selected; `None` for any other HDU.
#[cfg(feature = "compression")]
fn decoded_rows<S: Source>(
    data: &mut DataSource<S>,
    hdu: &Hdu,
    rows: &Range<usize>,
    columns: Option<&[bool]>,
) -> Result<Option<DecodedRows>> {
    let Some(tiled) = hdu.tiled_table()? else {
        return Ok(None);
    };
    validate_row_range(rows, tiled.schema.extent.row_count)?;
    let tiles = tiled.tiles(rows);
    let all = 0..hdu.container_schema()?.extent.row_count;
    // A tile's arrays have descriptors that address the whole container heap, and a read
    // of every tile reads the whole container: then the container is read whole, else
    // only the touched tiles.
    if tiled.has_arrays() || tiles == all {
        let container = data.table(hdu, None)?;
        let tiles = ContainerTiles {
            view: container,
            first_tile: 0,
        };
        return tiled.decode(tiles, rows, columns).map(Some);
    }
    let first_tile = tiles.start;
    let container = data.table_rows(hdu, TableRows::Range(tiles))?;
    let tiles = ContainerTiles {
        view: container,
        first_tile,
    };
    tiled.decode(tiles, rows, columns).map(Some)
}

/// The `data_bytes` of a data unit as a length in memory.
pub(crate) fn data_len(data_bytes: u64) -> Result<usize> {
    usize::try_from(data_bytes).map_err(|_| FitsError::DataUnitTooLarge { bytes: data_bytes })
}

#[cfg(test)]
pub(crate) mod internals {
    /// The `BINTABLE` a tiled-compressed HDU is stored as, its cells still compressed:
    /// what a test of the container's own columns reads.
    #[cfg(feature = "compression")]
    mod container {
        use std::ops::Range;

        use crate::bintable::Table;
        use crate::bintable::selection::{TableRows, TableSelection};
        use crate::fits_error::Result;
        use crate::reader::FitsReader;
        use crate::reader::source::Source;

        impl<S: Source> FitsReader<S> {
            pub(crate) fn read_container_table(&mut self, index: usize) -> Result<Table> {
                let hdu = &self.hdus[index];
                Table::new(hdu.container_schema()?.clone(), self.data.data_unit(hdu)?)
            }

            pub(crate) fn read_container_rows(
                &mut self,
                index: usize,
                rows: Range<usize>,
            ) -> Result<Table> {
                let hdu = &self.hdus[index];
                let schema = hdu.container_schema()?;
                let row_count = rows.len();
                let mut data = Vec::new();
                let selection = TableSelection {
                    schema,
                    rows: TableRows::Range(rows),
                    columns: None,
                };
                selection.copy_into(&mut self.data.unit(hdu), &mut data)?;
                Table::new(schema.compacted(row_count, data.len()), data)
            }
        }
    }
}

#[cfg(test)]
mod tests;
