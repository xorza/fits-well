use std::io::Read;
use std::io::Seek;
use std::ops::Range;

use crate::allocation;
use crate::alternate::Alternate;
use crate::ascii::Ascii;
use crate::bintable::Table;
use crate::bintable::column::Column;
use crate::bintable::column_data::ColumnData;
use crate::bintable::column_reader::ColumnReader;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::padded_len;
use crate::checksum;
use crate::column::ColumnSelector;
use crate::column::Named;
use crate::column::index_of;
#[cfg(feature = "compression")]
use crate::compress::table::DecodedRows;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::hdu::HduPosition;
use crate::hdu::HduRole;
use crate::hdu::data_extent;
use crate::header_model::card::is_end_record;
use crate::header_model::{Header, keywords};
use crate::ragged::Ragged;
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::DataSource;
use crate::reader::data_source::TableRows;
#[cfg(feature = "compression")]
use crate::reader::data_source::validate_row_range;
use crate::reader::hdu::Hdu;
use crate::reader::hdu_selector::HduSelector;
use crate::reader::image_reader::ImageReader;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular;

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod data_checksum;
mod data_source;
pub(crate) mod hdu;
pub(crate) mod hdu_selector;
pub(crate) mod image_reader;
pub(crate) mod source;

use source::SliceSource;
use source::Source;
use source::StreamSource;

/// Raw data for one selected table column over a row range.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum SelectedData {
    Fixed(ColumnData),
    Variable(Ragged<ColumnData>),
}

/// One decoded column in a ranged table selection.
#[derive(Debug, Clone)]
pub struct SelectedColumn {
    pub descriptor: Column,
    pub data: SelectedData,
}

impl SelectedColumn {
    fn new(column: &ColumnReader<'_>) -> Result<Self> {
        let descriptor = column.descriptor().clone();
        let data = if descriptor.tform.kind.is_descriptor() {
            SelectedData::Variable(column.vla()?)
        } else {
            SelectedData::Fixed(column.raw()?)
        };
        Ok(Self { descriptor, data })
    }

    /// Decode `column` into this column's own buffers, which it reuses when they hold
    /// the same kind of data.
    fn refill(&mut self, column: &ColumnReader<'_>) -> Result<()> {
        self.descriptor.clone_from(column.descriptor());
        let variable = self.descriptor.tform.kind.is_descriptor();
        match &mut self.data {
            SelectedData::Variable(rows) if variable => column.vla_into(rows),
            SelectedData::Fixed(values) if !variable => column.raw_into(values),
            _ => {
                *self = Self::new(column)?;
                Ok(())
            }
        }
    }
}

impl Named for SelectedColumn {
    fn name(&self) -> Option<&str> {
        self.descriptor.name()
    }
}

/// Selected columns decoded over a zero-based half-open row range.
#[derive(Debug, Clone)]
pub struct Selection {
    pub rows: Range<usize>,
    pub columns: Vec<SelectedColumn>,
}

impl Selection {
    /// The selected column that `column` names: an index of the table's columns, or a
    /// `TTYPEn` name compared case-insensitively. `None` when the selection does not
    /// hold it.
    pub fn column<'c>(&self, column: impl Into<ColumnSelector<'c>>) -> Option<&SelectedColumn> {
        match column.into() {
            ColumnSelector::Index(index) => self
                .columns
                .iter()
                .find(|selected| selected.descriptor.number == index + 1),
            ColumnSelector::Name(name) => {
                index_of(&self.columns, name).map(|pos| &self.columns[pos])
            }
        }
    }
}

/// A data unit read from the source: the full block-padded bytes plus the range
/// within them holding the actual data. The bytes after `data_range` are FITS
/// block fill, not part of the data array.
#[derive(Debug, Clone)]
pub struct DataUnit {
    /// The on-disk data unit, padded to the 2880-byte block grid.
    bytes: Vec<u8>,
    /// The sub-range of `bytes` that is meaningful data (`0..Nbits/8`).
    data_range: Range<usize>,
}

/// Immutable view of a padded data unit and its meaningful byte range.
#[derive(Debug, Clone)]
pub struct DataUnitView<'a> {
    /// Complete on-disk unit, including trailing 2880-byte block fill.
    pub padded: &'a [u8],
    /// Meaningful data bytes within [`DataUnitView::padded`].
    pub data_range: Range<usize>,
}

impl DataUnit {
    /// Borrow the complete padded unit and the meaningful data range within it.
    pub fn view(&self) -> DataUnitView<'_> {
        DataUnitView {
            padded: &self.bytes,
            data_range: self.data_range.clone(),
        }
    }

    /// The meaningful data with the trailing block fill sliced off — what a
    /// decoder should consume.
    pub fn data(&self) -> &[u8] {
        &self.bytes[self.data_range.clone()]
    }

    /// Consume the unit and return only its meaningful data bytes, without block fill.
    pub fn into_data(mut self) -> Vec<u8> {
        debug_assert_eq!(self.data_range.start, 0);
        self.bytes.truncate(self.data_range.end);
        self.bytes
    }

    /// Consume the unit and return its complete block-padded bytes.
    pub fn into_padded(self) -> Vec<u8> {
        self.bytes
    }
}

/// A FITS file opened over a seekable byte source. Opening scans HDU boundaries
/// from headers alone (no data is read); data units are fetched on demand.
#[derive(Debug)]
pub struct FitsReader<S> {
    /// The scanned HDU records; exposed read-only via [`FitsReader::hdus`].
    pub(crate) hdus: Vec<Hdu>,
    data: DataSource<S>,
    scratch: Scratch,
}

/// The reader's reused buffers a section or selection read fills and the result
/// borrows: the selected section's shape, its stored samples, the tile rows it
/// touches, a compacted table selection with the columns it selects, and the words a compressed image's physical
/// read decodes into.
#[derive(Debug, Default)]
pub(crate) struct Scratch {
    pub(crate) section_shape: Vec<usize>,
    pub(crate) section_bytes: Vec<u8>,
    #[cfg(feature = "compression")]
    pub(crate) tile_rows: Vec<usize>,
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
        let mut hdus = Vec::new();
        let mut offset = 0u64;
        loop {
            let position = if hdus.is_empty() {
                HduPosition::Primary
            } else {
                HduPosition::Extension
            };
            match scan_header_unit(&mut source, &mut offset, &mut scratch, position)? {
                NextHeader::Found { bytes, sum } => {
                    let header = Header::parse(&bytes)?;
                    let role = HduRole::from_header(&header, position)?;
                    let kind = HduKind::classify(&header, role)?;
                    let data_offset = offset;
                    let extent = data_extent(&header, role)?;
                    let next = data_offset
                        .checked_add(extent.padded_bytes)
                        .ok_or(FitsError::DataUnitOverflow)?;
                    hdus.push(Hdu::new(
                        hdus.len(),
                        header,
                        kind,
                        sum,
                        data_offset,
                        extent.data_bytes,
                    ));
                    // Skip past the data unit to the next header. Clamp at the source
                    // end so a declared unit larger than the file just ends the scan
                    // (the HDU is still recorded; a later read bounds-checks it).
                    offset = next.min(source.size());
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
        Ok(Self {
            hdus,
            data: DataSource::new(source, scratch),
            scratch: Scratch::default(),
        })
    }

    /// The scanned HDU records, read-only and in file order — each carrying its
    /// parsed [`Header`] and [`HduKind`]. Every read names its HDU with an
    /// [`HduSelector`]: an index into this slice, `"SCI"`, or `("SCI", 2)`.
    pub fn hdus(&self) -> &[Hdu] {
        &self.hdus
    }

    /// The HDU `hdu` names: the first that [`Hdu::matches`] it. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last HDU, and with
    /// [`FitsError::HduNotFound`] for a name no HDU carries.
    pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu> {
        selected_hdu(&self.hdus, hdu.into())
    }

    /// The index of the HDU `hdu` names, as [`FitsReader::hdu`] finds it.
    pub fn hdu_index<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<usize> {
        Ok(self.hdu(hdu)?.index)
    }

    /// The indices of every HDU [`FitsReader::image`] can read as an image: image
    /// extensions, tiled-compressed images, and a non-empty primary array (an empty
    /// `NAXIS = 0` primary is a container, not an image, and is skipped). The first
    /// image is `images().next()`.
    pub fn images(&self) -> impl Iterator<Item = usize> + '_ {
        self.hdus
            .iter()
            .filter(|hdu| hdu.is_image())
            .map(|hdu| hdu.index)
    }

    /// Read the raw, still-encoded (big-endian, unscaled) data unit into a fresh,
    /// caller-owned buffer. The returned [`DataUnit`] carries the full block-padded
    /// bytes plus the range of actual data within them, so a decoder consumes
    /// [`DataUnit::data`] and the block fill is never mistaken for samples.
    ///
    /// This is the owned form, backing the table readers (which keep the bytes as
    /// the parsed table's storage). Image and random-groups reads instead stage
    /// through the reader's reused internal scratch — see [`FitsReader::image`].
    pub fn read_data_raw<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<DataUnit> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let bytes = self.data.read_owned(hdu.data_offset, lengths.padded)?;
        Ok(DataUnit {
            bytes,
            data_range: 0..lengths.data,
        })
    }

    /// The image HDU `index` holds, plain or tiled-compressed alike, ready to read a
    /// region at a time. Errors with [`FitsError::NotAnImage`] for tables, random
    /// groups and unmodelled extensions, a compressed image among them without the
    /// `compression` feature.
    pub fn image<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<ImageReader<'_, S>> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        ImageReader::new(hdu, &mut self.data, &mut self.scratch)
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`Table::column`]. Errors as
    /// [`Hdu::table_schema`] does for an HDU that is not a plain binary table.
    pub fn read_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Table> {
        let index = self.hdu_index(hdu)?;
        #[cfg(feature = "compression")]
        {
            let hdu = &self.hdus[index];
            let schema = hdu.table_schema()?;
            if let Some(decoded) = decoded_rows(&mut self.data, hdu, &(0..schema.nrows), None)? {
                return Table::new(decoded.schema, decoded.data);
            }
        }
        self.read_table_as(index, Hdu::table_schema)
    }

    /// [`Self::read_table`] under the schema `schema` gives the HDU.
    fn read_table_as(
        &mut self,
        index: usize,
        schema: fn(&Hdu) -> Result<&Schema>,
    ) -> Result<Table> {
        let schema = schema(&self.hdus[index])?.clone();
        let unit = self.read_data_raw(index)?;
        Table::new(schema, unit.bytes)
    }

    /// A binary table's schema, parsed from its header when the file was opened. Errors
    /// as [`Hdu::table_schema`] does.
    pub fn table_schema<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Schema> {
        selected_hdu(&self.hdus, hdu.into())?.table_schema()
    }

    /// Materialize only the requested contiguous row range, including only the VLA
    /// heap cells referenced by those rows.
    pub fn read_table_rows<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: Range<usize>,
    ) -> Result<Table> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        let schema = hdu.table_schema()?;
        let row_count = rows.len();
        let mut data = Vec::new();
        select_rows(&mut self.data, hdu, schema, rows, None, &mut data)?;
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
        let filled = self
            .resolve_columns(hdu.into(), columns)
            .and_then(|index| self.fill_selection(index, rows, out));
        if filled.is_err() {
            out.rows = 0..0;
            out.columns.clear();
        }
        filled
    }

    /// Resolve `columns` of the table `hdu` selects into the scratch, and give the index
    /// of that HDU.
    fn resolve_columns<'c>(
        &mut self,
        hdu: HduSelector<'_>,
        columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>,
    ) -> Result<usize> {
        let hdu = selected_hdu(&self.hdus, hdu)?;
        let schema = hdu.table_schema()?;
        let column_indices = &mut self.scratch.column_indices;
        column_indices.clear();
        for selector in columns {
            column_indices.push(selector.into().resolve(&schema.columns)?);
        }
        Ok(hdu.index)
    }

    /// Decode the columns [`Self::resolve_columns`] resolved of HDU `index` over `rows`
    /// into `out`.
    fn fill_selection(
        &mut self,
        index: usize,
        rows: Range<usize>,
        out: &mut Selection,
    ) -> Result<()> {
        let hdu = &self.hdus[index];
        let schema = hdu.table_schema()?;
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
            schema,
            rows.clone(),
            Some(column_mask),
            selection,
        )?;
        let table = TableView::compact(&schema.columns, rows.len(), schema.row_len, selection);
        out.rows = rows;
        out.columns.truncate(column_indices.len());
        for (pos, &index) in column_indices.iter().enumerate() {
            let column = table.column(index)?;
            if let Some(selected) = out.columns.get_mut(pos) {
                selected.refill(&column)?;
            } else {
                out.columns.push(SelectedColumn::new(&column)?);
            }
        }
        Ok(())
    }

    /// Decode one fixed or variable-length table cell without materializing other rows.
    pub fn read_table_cell<'h, 'c>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        row: usize,
        column: impl Into<ColumnSelector<'c>>,
    ) -> Result<ColumnData> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        let schema = hdu.table_schema()?;
        let column = column.into().resolve(&schema.columns)?;
        #[cfg(feature = "compression")]
        {
            let mut selected = vec![false; schema.columns.len()];
            selected[column] = true;
            let rows = row..row.saturating_add(1);
            if let Some(decoded) = decoded_rows(&mut self.data, hdu, &rows, Some(&selected))? {
                return DataSource::new(SliceSource::new(&decoded.data), Vec::new())
                    .read_table_cell(0, &decoded.schema, row - decoded.first_row, column);
            }
        }
        self.data
            .read_table_cell(hdu.data_offset, schema, row, column)
    }

    /// Parse an HDU's WCS and resolve any standard `-TAB` coordinate arrays from
    /// their referenced `BINTABLE` extensions. Header-only WCS descriptions use the
    /// same path as [`Wcs::from_header`]; this method is required for `-TAB` because its
    /// coordinate values and optional index vectors live outside the source header.
    pub fn read_wcs<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        alt: impl TryInto<Alternate, Error: Into<FitsError>>,
    ) -> Result<Wcs> {
        let alt = Alternate::of(alt)?;
        self.resolve_wcs(hdu.into(), alt)
    }

    /// [`Self::read_wcs`] of the HDU `hdu` selects, the description `alt` selects.
    fn resolve_wcs(&mut self, hdu: HduSelector<'_>, alt: Alternate) -> Result<Wcs> {
        let header = &selected_hdu(&self.hdus, hdu)?.header;
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
            let selector = HduSelector::Extension {
                name: &reference.extension_name,
                version: Some(reference.extension_version),
                level: Some(reference.extension_level),
            };
            let table_hdu =
                find_extension(&self.hdus, selector, HduKind::BinTable)?.ok_or_else(|| {
                    FitsError::InvalidWcs {
                        detail: format!(
                            "TAB BINTABLE {:?}, EXTVER {}, EXTLEVEL {} was not found",
                            reference.extension_name,
                            reference.extension_version,
                            reference.extension_level
                        ),
                    }
                })?;
            let schema = table_hdu.table_schema()?;
            let mut selected = vec![false; schema.columns.len()];
            for descriptor in &group {
                for name in descriptor.referenced_columns() {
                    selected[schema.column_index_checked(name)?] = true;
                }
            }
            let rows = 0..usize::from(schema.nrows != 0);
            let row_count = rows.len();
            self.data.select_table(
                table_hdu.data_offset,
                schema,
                TableRows::Range(rows),
                Some(&selected),
                &mut self.scratch.selection,
            )?;
            let table = TableView::compact(
                &schema.columns,
                row_count,
                schema.row_len,
                &self.scratch.selection,
            );
            for descriptor in group {
                transforms.push(tabular::TabularTransform::from_table(&descriptor, table)?);
            }
        }
        Wcs::from_header_with_tabular(header, alt, transforms)
    }

    /// Read an `TABLE` (ASCII table) extension and parse its column structure.
    /// Errors with [`FitsError::NotAnAsciiTable`] for any other HDU.
    pub fn read_ascii_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Ascii> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        if hdu.kind != HduKind::AsciiTable {
            return Err(FitsError::NotAnAsciiTable);
        }
        let index = hdu.index;
        let unit = self.read_data_raw(index)?;
        Ascii::from_data(&self.hdus[index].header, unit.bytes)
    }

    /// Read and decode a random-groups primary array (§6). Errors with
    /// [`FitsError::NotRandomGroups`] for any other HDU.
    pub fn read_groups<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<RandomGroups> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::NotRandomGroups);
        }
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let unit = self.data.slice(hdu.data_offset, lengths.padded)?;
        RandomGroups::from_data(&hdu.header, &unit[..lengths.data])
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J).
    /// Strings containing one or more blanks are reported as
    /// [`ChecksumStatus::Unknown`] because the standard reserves them for
    /// undefined or unknown checksum values.
    pub fn verify_checksum<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
    ) -> Result<ChecksumReport> {
        let hdu = selected_hdu(&self.hdus, hdu.into())?;
        DataChecksum::of(hdu).finish(hdu, &mut self.data)
    }
}

/// Select `rows` and `columns` (every column when `None`) of the binary table `hdu` holds
/// into `out`, as [`DataSource::select_table`] does: from the source for a plain table, from
/// the decoded tiles for a tiled-compressed one. A free function over the reader's fields,
/// so the HDU stays borrowed while the source is read.
fn select_rows<S: Source>(
    data: &mut DataSource<S>,
    hdu: &Hdu,
    schema: &Schema,
    rows: Range<usize>,
    columns: Option<&[bool]>,
    out: &mut Vec<u8>,
) -> Result<()> {
    #[cfg(feature = "compression")]
    if let Some(decoded) = decoded_rows(data, hdu, &rows, columns)? {
        let rows = rows.start - decoded.first_row..rows.end - decoded.first_row;
        return DataSource::new(SliceSource::new(&decoded.data), Vec::new()).select_table(
            0,
            &decoded.schema,
            TableRows::Range(rows),
            columns,
            out,
        );
    }
    data.select_table(
        hdu.data_offset,
        schema,
        TableRows::Range(rows),
        columns,
        out,
    )
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
    validate_row_range(rows, tiled.schema.nrows)?;
    let container = data.table(hdu, None)?;
    tiled
        .decode(&hdu.header, container, rows, columns)
        .map(Some)
}

/// The HDU `selector` names. A free function over the records, so a read can hold an
/// HDU while it fetches bytes from the source.
fn selected_hdu<'h>(hdus: &'h [Hdu], selector: HduSelector<'_>) -> Result<&'h Hdu> {
    match selector {
        HduSelector::Index(index) => hdus.get(index).ok_or(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index,
            len: hdus.len(),
        }),
        HduSelector::Extension {
            name,
            version,
            level,
        } => {
            for hdu in hdus {
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

/// The first HDU of `kind` that `selector` names.
fn find_extension<'h>(
    hdus: &'h [Hdu],
    selector: HduSelector<'_>,
    kind: HduKind,
) -> Result<Option<&'h Hdu>> {
    for hdu in hdus {
        if hdu.kind == kind && hdu.matches(selector)? {
            return Ok(Some(hdu));
        }
    }
    Ok(None)
}

/// The state of one FITS checksum keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumStatus {
    /// The keyword is not present, so the HDU asserts no checksum knowledge.
    Absent,
    /// The keyword contains one or more blanks and nothing else.
    Unknown,
    /// The asserted checksum matches the recomputed value.
    Valid,
    /// The value is malformed or does not match the recomputed checksum.
    Invalid,
}

/// Result of [`FitsReader::verify_checksum`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecksumReport {
    pub datasum: ChecksumStatus,
    pub checksum: ChecksumStatus,
}

impl ChecksumReport {
    /// Whether both `DATASUM` and `CHECKSUM` are present and verify.
    pub const fn is_valid(&self) -> bool {
        matches!(
            (self.datasum, self.checksum),
            (ChecksumStatus::Valid, ChecksumStatus::Valid)
        )
    }

    /// Whether `DATASUM` or `CHECKSUM` is malformed or does not match its recomputed
    /// value.
    pub const fn has_invalid(&self) -> bool {
        matches!(self.datasum, ChecksumStatus::Invalid)
            || matches!(self.checksum, ChecksumStatus::Invalid)
    }
}

#[derive(Debug, Clone, Copy)]
struct DataLengths {
    data: usize,
    padded: usize,
}

impl DataLengths {
    fn new(data_bytes: u64) -> Result<Self> {
        let data = usize::try_from(data_bytes)
            .map_err(|_| FitsError::DataUnitTooLarge { bytes: data_bytes })?;
        let padded_bytes = padded_len(data_bytes).expect("data_extent bounds the padded data unit");
        let padded = usize::try_from(padded_bytes).map_err(|_| FitsError::DataUnitTooLarge {
            bytes: padded_bytes,
        })?;
        Ok(Self { data, padded })
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

/// Read one header unit at `*offset`, advancing `offset` past each consumed block,
/// until a block carries the `END` record. Blocks come through [`Source::slice`], so
/// the same scan drives both seeking and in-memory sources.
fn scan_header_unit<S: Source>(
    source: &mut S,
    offset: &mut u64,
    scratch: &mut Vec<u8>,
    position: HduPosition,
) -> Result<NextHeader> {
    let size = source.size();
    // Most headers are a single block; reserve it so the common case parses with one
    // allocation and only multi-block headers grow.
    let mut bytes = Vec::with_capacity(BLOCK_SIZE);
    let mut sum = 0;
    loop {
        match size - *offset {
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
        let block = source.slice(*offset, BLOCK_SIZE, scratch)?;
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
        *offset += BLOCK_SIZE as u64;
        sum = checksum::accumulate(block, sum);
        allocation::try_reserve(&mut bytes, block.len())?;
        bytes.extend_from_slice(block);
        if block_has_end(block) {
            return Ok(NextHeader::Found { bytes, sum });
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
pub(crate) mod internals {
    use crate::image_model::Image;
    use crate::image_model::region::Region;
    use crate::reader::FitsReader;
    use crate::reader::StreamReader;
    use crate::reader::source::Source;
    use std::fs;
    use std::fs::File;

    /// The `BINTABLE` a tiled-compressed HDU is stored as, its cells still compressed:
    /// what a test of the container's own columns reads.
    #[cfg(feature = "compression")]
    mod container {
        use std::ops::Range;

        use crate::bintable::Table;
        use crate::error::Result;
        use crate::reader::FitsReader;
        use crate::reader::data_source::TableRows;
        use crate::reader::hdu::Hdu;
        use crate::reader::source::Source;

        impl<S: Source> FitsReader<S> {
            pub(crate) fn read_container_table(&mut self, index: usize) -> Result<Table> {
                self.read_table_as(index, Hdu::container_schema)
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
                self.data.select_table(
                    hdu.data_offset,
                    schema,
                    TableRows::Range(rows),
                    None,
                    &mut data,
                )?;
                Table::new(schema.compacted(row_count, data.len()), data)
            }
        }
    }

    /// Where the fixture `name` lies, relative to the crate root tests run in.
    pub(crate) fn fixture_path(name: &str) -> String {
        format!("test_resources/{name}")
    }

    pub(crate) fn fixture_bytes(name: &str) -> Vec<u8> {
        let path = fixture_path(name);
        fs::read(&path).unwrap_or_else(|e| panic!("read {path}: {e}"))
    }

    /// HDU `index` read whole into an owned image.
    pub(crate) fn read_whole<S: Source>(reader: &mut FitsReader<S>, index: usize) -> Image {
        reader.image(index).unwrap().read(Region::All).unwrap()
    }

    /// Open a fixture as a streaming reader, reporting which file failed rather
    /// than a bare unwrap — every read-path test starts here.
    pub(crate) fn open_fixture(name: &str) -> StreamReader<File> {
        let path = fixture_path(name);
        FitsReader::open(File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}")))
            .unwrap_or_else(|e| panic!("parse {name}: {e}"))
    }
}

#[cfg(test)]
mod tests;
