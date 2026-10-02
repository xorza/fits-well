use std::io::Read;
use std::io::Seek;
use std::ops::Range;

use crate::allocation;
use crate::ascii::AsciiTable;
use crate::bintable::BinTable;
use crate::bintable::column::Column;
use crate::bintable::column_data::ColumnData;
use crate::bintable::table_schema::TableSchema;
use crate::bintable::table_view::TableView;
use crate::bitpix::Bitpix;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::padded_len;
use crate::checksum;
use crate::column;
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::data::image_view::BorrowedImage;
use crate::data::image_view::ImageView;
use crate::data::read_image::ReadImage;
use crate::data::swap_into_words;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::hdu::HduPosition;
use crate::hdu::HduRole;
use crate::hdu::data_extent;
use crate::header_model::Header;
use crate::header_model::card::is_end_record;
use crate::reader::data_source::DataSource;
use crate::reader::data_source::TableRows;
use crate::reader::hdu::Hdu;
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular;

mod data_source;
pub(crate) mod hdu;
pub(crate) mod source;

use source::SliceSource;
use source::Source;
use source::StreamSource;

#[cfg(feature = "compression")]
use crate::compress::decode::tiled_image::{TileSection, TiledImage};
#[cfg(feature = "compression")]
use crate::compress::table;

/// Zero-based or case-insensitive named table-column selection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ColumnSelector {
    Index(usize),
    Name(String),
}

impl From<usize> for ColumnSelector {
    fn from(index: usize) -> ColumnSelector {
        ColumnSelector::Index(index)
    }
}

impl From<&str> for ColumnSelector {
    fn from(name: &str) -> ColumnSelector {
        ColumnSelector::Name(name.to_string())
    }
}

impl From<String> for ColumnSelector {
    fn from(name: String) -> ColumnSelector {
        ColumnSelector::Name(name)
    }
}

/// Raw data for one selected table column over a row range.
#[derive(Debug, Clone, PartialEq)]
#[non_exhaustive]
pub enum TableColumnData {
    Fixed(ColumnData),
    Variable(Vec<ColumnData>),
}

/// One decoded column in a ranged table selection.
#[derive(Debug, Clone)]
pub struct SelectedColumn {
    pub descriptor: Column,
    pub data: TableColumnData,
}

/// Selected columns decoded over a zero-based half-open row range.
#[derive(Debug, Clone)]
pub struct TableSelection {
    pub rows: Range<usize>,
    pub columns: Vec<SelectedColumn>,
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
    /// Reused buffers a section or selection read fills and the result borrows: the
    /// selected section's shape, its assembled samples, the tile rows it touches, and
    /// a compacted table selection.
    section_shape: Vec<usize>,
    section_bytes: Vec<u8>,
    #[cfg(feature = "compression")]
    tile_rows: Vec<usize>,
    selection: Vec<u8>,
}

/// A [`FitsReader`] over a seeking byte source (`Read + Seek`, e.g. a `File`) — the
/// type [`FitsReader::open`] returns. A friendlier name for `FitsReader<StreamSource<R>>`.
pub type StreamReader<R> = FitsReader<StreamSource<R>>;

/// A [`FitsReader`] over an in-memory byte slice — the type [`FitsReader::from_bytes`]
/// returns. The lifetime is that of the borrowed bytes.
pub type SliceReader<'a> = FitsReader<SliceSource<'a>>;

/// A [`FitsReader`] over a memory-mapped file — the type [`FitsReader::open_mmap`]
/// returns. Requires the `mmap` feature.
#[cfg(feature = "mmap")]
pub type MmapReader = FitsReader<source::MmapSource>;

impl<R: Read + Seek> FitsReader<StreamSource<R>> {
    /// Open a seekable byte source (file, cursor). Data units are copied into the
    /// reader's scratch on demand; for an in-memory file prefer
    /// [`FitsReader::from_bytes`], which decodes straight from the bytes.
    pub fn open(source: R) -> Result<StreamReader<R>> {
        FitsReader::from_source(StreamSource::new(source)?)
    }

    /// Consume the reader and recover the underlying seekable input.
    pub fn into_inner(self) -> R {
        self.data.into_source().into_inner()
    }
}

impl<'a> FitsReader<SliceSource<'a>> {
    /// Open an in-memory FITS file — the whole thing as a byte slice (e.g. an mmap,
    /// or bytes already in RAM). Data units decode straight from the borrowed bytes
    /// with no staging copy, and no scratch allocation.
    pub fn from_bytes(bytes: &'a [u8]) -> Result<SliceReader<'a>> {
        FitsReader::from_source(SliceSource::new(bytes))
    }

    /// Consume the reader and recover the original borrowed byte slice.
    pub fn into_bytes(self) -> &'a [u8] {
        self.data.into_source().into_bytes()
    }
}

#[cfg(feature = "mmap")]
impl FitsReader<source::MmapSource> {
    /// Memory-map a FITS file and read it zero-copy: data units decode straight from
    /// the mapped pages (no staging copy, no read syscalls). Best for large files
    /// and random HDU access. Requires the `mmap` feature.
    pub fn open_mmap(path: impl AsRef<std::path::Path>) -> Result<MmapReader> {
        FitsReader::from_source(source::MmapSource::open(path.as_ref())?)
    }
}

impl<S: Source> FitsReader<S> {
    /// Scan the whole HDU sequence, parsing every header and recording the byte
    /// range of each data unit — without reading any data.
    fn from_source(mut source: S) -> Result<FitsReader<S>> {
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
                    hdus.push(Hdu::new(header, kind, sum, data_offset, extent.data_bytes));
                    // Skip past the data unit to the next header. Clamp at the source
                    // end so a declared unit larger than the file just ends the scan
                    // (the HDU is still recorded; a later read bounds-checks it).
                    offset = next.min(source.size());
                }
                NextHeader::End if hdus.is_empty() => return Err(FitsError::UnexpectedEof),
                NextHeader::End => break,
                // §3.5/§3.6: special records and a trailing partial / fill block may
                // follow the last HDU; a reader disregards them. But the same shape
                // *before* any valid HDU means there is no conforming primary.
                NextHeader::Trailing if hdus.is_empty() => return Err(FitsError::UnexpectedEof),
                NextHeader::Trailing => break,
            }
        }
        Ok(FitsReader {
            hdus,
            data: DataSource::new(source, scratch),
            section_shape: Vec::new(),
            section_bytes: Vec::new(),
            #[cfg(feature = "compression")]
            tile_rows: Vec::new(),
            selection: Vec::new(),
        })
    }

    /// The HDU at `index`, or [`FitsError::IndexOutOfBounds`] — the checked form
    /// the `read_*` methods bound-check through.
    fn checked_hdu(&self, index: usize) -> Result<&Hdu> {
        checked_hdu(&self.hdus, index)
    }

    /// The scanned HDU records, read-only and in file order — each carrying its
    /// parsed [`Header`] and [`HduKind`]. Index, iterate, or `.len()` the slice; pick
    /// an index for a `read_*` method (or use [`FitsReader::image_indices`] /
    /// [`FitsReader::hdu_index`] to find one).
    pub fn hdus(&self) -> &[Hdu] {
        &self.hdus
    }

    /// Index of the extension named `name` by its `EXTNAME` keyword (compared
    /// case-insensitively, as `EXTNAME` is conventionally matched), or `None`. When
    /// `version` is `Some`, also require a matching `EXTVER` (which defaults to `1`
    /// where the card is absent, §4.4.1) — the way duplicate extensions like
    /// `('SCI', 1)` and `('SCI', 2)` are told apart. The primary array has no
    /// `EXTNAME`. Pair the returned index with a `read_*` method.
    pub fn hdu_index(&self, name: &str, version: Option<i64>) -> Result<Option<usize>> {
        find_extension(&self.hdus, name, version, None, None)
    }

    /// The indices of every HDU [`FitsReader::read_image`] can read as an image: image
    /// extensions, tiled-compressed images, and a non-empty primary array (an empty
    /// `NAXIS = 0` primary is a container, not an image, and is skipped). A FITS file
    /// may hold any number of images — pick an `index` from this list to pass to
    /// [`FitsReader::read_image`] without inspecting [`HduKind`] yourself.
    pub fn image_indices(&self) -> Vec<usize> {
        self.hdus
            .iter()
            .enumerate()
            .filter(|(_, hdu)| hdu.is_image())
            .map(|(index, _)| index)
            .collect()
    }

    /// Read the raw, still-encoded (big-endian, unscaled) data unit into a fresh,
    /// caller-owned buffer. The returned [`DataUnit`] carries the full block-padded
    /// bytes plus the range of actual data within them, so a decoder consumes
    /// [`DataUnit::data`] and the block fill is never mistaken for samples.
    ///
    /// This is the owned form, backing the table readers (which keep the bytes as
    /// the parsed table's storage). Image and random-groups reads instead stage
    /// through the reader's reused internal scratch — see [`FitsReader::read_image`].
    pub fn read_data_raw(&mut self, index: usize) -> Result<DataUnit> {
        let hdu = checked_hdu(&self.hdus, index)?;
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let bytes = self.data.read_owned(hdu.data_offset, lengths.padded)?;
        Ok(DataUnit {
            bytes,
            data_range: 0..lengths.data,
        })
    }

    /// Read an HDU's image as a [`ReadImage`], transparently handling **both** plain
    /// and tiled-compressed (`ZIMAGE`) images — the caller doesn't need to know which.
    /// Errors with [`FitsError::NotAnImage`] for tables, random groups, and unmodelled
    /// extensions.
    ///
    /// A plain image is **zero-copy**: its big-endian bytes are viewed in place over
    /// the source (or the reader's reused scratch for a seeking source), decoded only
    /// when you ask. A compressed image is decompressed into an owned buffer (with the
    /// `compression` feature; without it this returns [`FitsError::NotAnImage`] for a
    /// `ZIMAGE` HDU). Either way, reach for the samples via [`ReadImage::u8`]
    /// (zero-copy `BITPIX = 8`), [`ReadImage::decode`] (host-endian), or
    /// [`ReadImage::physical`] (scaled). The result borrows the reader, so handle one
    /// image before reading the next.
    pub fn read_image(&mut self, index: usize) -> Result<ReadImage<'_>> {
        let hdu = checked_hdu(&self.hdus, index)?;
        // §10.1: a tiled-compressed image is classified [`HduKind::CompressedImage`]
        // (a `ZIMAGE` BINTABLE). Route it through the decompressor so callers see one
        // image API regardless of storage.
        #[cfg(feature = "compression")]
        if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            let table = self.data.table(hdu)?;
            let samples = tiled.decode(&hdu.header, table)?;
            return Ok(ReadImage::decoded(samples, &image.shape, image.scaling));
        }

        let image = hdu.plain_image_geometry()?;
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let unit = self.data.slice(hdu.data_offset, lengths.padded)?;
        let bytes = &unit[..lengths.data];
        // With PCOUNT = 0 and GCOUNT = 1, `data_extent` sized the unit as
        // `elem · Π(axes)` — an invariant between the scan and the geometry, not a
        // runtime failure mode.
        debug_assert!(image.byte_len().is_ok_and(|len| len == bytes.len()));
        Ok(ReadImage::raw(
            &image.shape,
            image.bitpix,
            image.scaling,
            bytes,
        ))
    }

    /// Read an image as a borrowed, host-endian [`ImageView`], byte-swapping into the
    /// caller-owned `scratch` — the fast, low-copy path for a loop that processes each
    /// image and moves on. Where [`read_image`](FitsReader::read_image)`.decode()`
    /// allocates a fresh owned buffer per call (page-fault-bound — profiling found
    /// that dominates a plain typed read), this reuses `scratch`, so a hot loop pays
    /// the output allocation once and reuses it across reads — even across differing
    /// `BITPIX`. The caller owns `scratch`, so the reader retains nothing image-sized
    /// beyond a seeking source's staging buffer; pass the same `Vec` each call and
    /// drop it when the loop ends.
    ///
    /// `scratch` is `Vec<u64>` so the swapped samples stay 8-byte aligned for the
    /// typed views. A `BITPIX = 8` image needs no swap and the view borrows the source
    /// bytes directly (zero-copy, `scratch` untouched); a compressed image is
    /// decompressed directly into `scratch` from its table viewed in place. The view
    /// borrows the reader and `scratch`, so handle one image before reading the next.
    /// For samples you need to keep, use [`ReadImage::decode`].
    pub fn read_image_view<'a>(
        &'a mut self,
        index: usize,
        scratch: &'a mut Vec<u64>,
    ) -> Result<BorrowedImage<'a>> {
        let hdu = checked_hdu(&self.hdus, index)?;
        #[cfg(feature = "compression")]
        if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            let table = self.data.table(hdu)?;
            return Ok(BorrowedImage {
                shape: &image.shape,
                scaling: image.scaling,
                samples: tiled.decode_into_words(&hdu.header, table, scratch)?,
            });
        }

        let image = hdu.plain_image_geometry()?;
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let unit = self.data.slice(hdu.data_offset, lengths.padded)?;
        let be = &unit[..lengths.data];
        let samples = if image.bitpix == Bitpix::U8 {
            // No byte-swap: the on-disk bytes already are the host-endian samples, so
            // borrow them straight (zero-copy) — `scratch` stays untouched.
            ImageView::U8(be)
        } else {
            swap_into_words(be, image.bitpix, scratch);
            view_words(scratch, image.bitpix, lengths.data)
        };
        Ok(BorrowedImage {
            shape: &image.shape,
            scaling: image.scaling,
            samples,
        })
    }

    /// Read a checked N-dimensional rectangular image section. Axis ranges are
    /// zero-based, half-open, and ordered fastest-axis first.
    pub fn read_image_section(&mut self, index: usize, ranges: &[Range<usize>]) -> Result<Image> {
        let hdu = checked_hdu(&self.hdus, index)?;
        #[cfg(feature = "compression")]
        if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            tiled.select(ranges, &mut self.section_shape, &mut self.tile_rows)?;
            let table = self
                .data
                .table_rows(hdu, &self.tile_rows, &mut self.selection)?;
            let section = TileSection {
                ranges,
                shape: &self.section_shape,
                rows: &self.tile_rows,
            };
            let samples = tiled.decode_section(&hdu.header, table, section)?;
            return Image::new_scaled(self.section_shape.as_slice(), samples, image.scaling);
        }

        let image = hdu.plain_image_geometry()?;
        self.data.plain_section(
            hdu,
            image,
            ranges,
            &mut self.section_shape,
            &mut self.section_bytes,
        )?;
        Image::new_scaled(
            self.section_shape.as_slice(),
            ImageData::decode(&self.section_bytes, image.bitpix),
            image.scaling,
        )
    }

    /// Scratch-reusing counterpart to [`FitsReader::read_image_section`].
    pub fn read_image_section_view<'a>(
        &'a mut self,
        index: usize,
        ranges: &[Range<usize>],
        words: &'a mut Vec<u64>,
    ) -> Result<BorrowedImage<'a>> {
        let hdu = checked_hdu(&self.hdus, index)?;
        #[cfg(feature = "compression")]
        if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            tiled.select(ranges, &mut self.section_shape, &mut self.tile_rows)?;
            let table = self
                .data
                .table_rows(hdu, &self.tile_rows, &mut self.selection)?;
            let section = TileSection {
                ranges,
                shape: &self.section_shape,
                rows: &self.tile_rows,
            };
            return Ok(BorrowedImage {
                shape: &self.section_shape,
                scaling: image.scaling,
                samples: tiled.decode_section_into_words(&hdu.header, table, section, words)?,
            });
        }

        let image = hdu.plain_image_geometry()?;
        self.data.plain_section(
            hdu,
            image,
            ranges,
            &mut self.section_shape,
            &mut self.section_bytes,
        )?;
        let samples = if image.bitpix == Bitpix::U8 {
            ImageView::U8(&self.section_bytes)
        } else {
            swap_into_words(&self.section_bytes, image.bitpix, words);
            view_words(words, image.bitpix, self.section_bytes.len())
        };
        Ok(BorrowedImage {
            shape: &self.section_shape,
            scaling: image.scaling,
            samples,
        })
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`BinTable::column_by_idx`]. Errors with
    /// [`FitsError::NotABinTable`] for any other HDU kind.
    pub fn read_table(&mut self, index: usize) -> Result<BinTable> {
        let schema = checked_hdu(&self.hdus, index)?.table_schema()?.clone();
        let unit = self.read_data_raw(index)?;
        BinTable::new(schema, unit.bytes)
    }

    /// A binary table's schema, parsed from its header when the file was opened. For
    /// a tiled-compressed image or table, the schema of its `BINTABLE` container.
    pub fn table_schema(&self, index: usize) -> Result<&TableSchema> {
        checked_hdu(&self.hdus, index)?.table_schema()
    }

    /// Materialize only the requested contiguous row range, including only the VLA
    /// heap cells referenced by those rows.
    pub fn read_table_rows(&mut self, index: usize, rows: Range<usize>) -> Result<BinTable> {
        let hdu = checked_hdu(&self.hdus, index)?;
        let schema = hdu.table_schema()?;
        let row_count = rows.len();
        let mut data = Vec::new();
        self.data.select_table(
            hdu.data_offset,
            schema,
            TableRows::Range(rows),
            None,
            &mut data,
        )?;
        BinTable::new(schema.compacted(row_count, data.len()), data)
    }

    /// Decode selected columns over a row range without reading unrelated rows.
    pub fn read_table_columns(
        &mut self,
        index: usize,
        rows: Range<usize>,
        columns: &[ColumnSelector],
    ) -> Result<TableSelection> {
        let hdu = checked_hdu(&self.hdus, index)?;
        let schema = hdu.table_schema()?;
        let selected_indices = columns
            .iter()
            .map(|selector| resolve_column_selector(schema, selector))
            .collect::<Result<Vec<_>>>()?;
        let mut selected = vec![false; schema.columns.len()];
        for &index in &selected_indices {
            selected[index] = true;
        }
        self.data.select_table(
            hdu.data_offset,
            schema,
            TableRows::Range(rows.clone()),
            Some(&selected),
            &mut self.selection,
        )?;
        let table =
            TableView::compact(&schema.columns, rows.len(), schema.row_len, &self.selection);
        let mut decoded = Vec::with_capacity(columns.len());
        for &index in &selected_indices {
            let column = table.column_by_idx(index)?;
            let descriptor = column.descriptor().clone();
            let data = if descriptor.tform.kind.is_descriptor() {
                TableColumnData::Variable(column.vla()?)
            } else {
                TableColumnData::Fixed(column.raw()?)
            };
            decoded.push(SelectedColumn { descriptor, data });
        }
        Ok(TableSelection {
            rows,
            columns: decoded,
        })
    }

    /// Decode one fixed or variable-length table cell without materializing other rows.
    pub fn read_table_cell(
        &mut self,
        index: usize,
        row: usize,
        column: ColumnSelector,
    ) -> Result<ColumnData> {
        let hdu = checked_hdu(&self.hdus, index)?;
        let schema = hdu.table_schema()?;
        let column = resolve_column_selector(schema, &column)?;
        self.data
            .read_table_cell(hdu.data_offset, schema, row, column)
    }

    /// Parse an HDU's WCS and resolve any standard `-TAB` coordinate arrays from
    /// their referenced `BINTABLE` extensions. Header-only WCS descriptions use the
    /// same path as [`Header::wcs`]; this method is required for `-TAB` because its
    /// coordinate values and optional index vectors live outside the source header.
    pub fn read_wcs(&mut self, index: usize, alt: Option<char>) -> Result<Wcs> {
        let header = &checked_hdu(&self.hdus, index)?.header;
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
            let table_index = find_extension(
                &self.hdus,
                &reference.extension_name,
                Some(reference.extension_version),
                Some(reference.extension_level),
                Some(HduKind::BinTable),
            )?
            .ok_or_else(|| FitsError::InvalidWcs {
                detail: format!(
                    "TAB BINTABLE {:?}, EXTVER {}, EXTLEVEL {} was not found",
                    reference.extension_name,
                    reference.extension_version,
                    reference.extension_level
                ),
            })?;
            let table_hdu = &self.hdus[table_index];
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
                &mut self.selection,
            )?;
            let table =
                TableView::compact(&schema.columns, row_count, schema.row_len, &self.selection);
            for descriptor in group {
                transforms.push(tabular::TabularTransform::from_table(descriptor, table)?);
            }
        }
        Wcs::from_header_with_tabular(header, alt, transforms)
    }

    /// Read an `TABLE` (ASCII table) extension and parse its column structure.
    /// Errors with [`FitsError::NotAnAsciiTable`] for any other HDU.
    pub fn read_ascii_table(&mut self, index: usize) -> Result<AsciiTable> {
        let hdu = self.checked_hdu(index)?;
        if hdu.kind != HduKind::AsciiTable {
            return Err(FitsError::NotAnAsciiTable);
        }
        let unit = self.read_data_raw(index)?;
        AsciiTable::from_data(&self.hdus[index].header, unit.bytes)
    }

    /// Read and decode a random-groups primary array (§6). Errors with
    /// [`FitsError::NotRandomGroups`] for any other HDU.
    pub fn read_groups(&mut self, index: usize) -> Result<RandomGroups> {
        let hdu = checked_hdu(&self.hdus, index)?;
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::NotRandomGroups);
        }
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let unit = self.data.slice(hdu.data_offset, lengths.padded)?;
        RandomGroups::from_data(&hdu.header, &unit[..lengths.data])
    }

    /// Read a tiled-compressed table (§10.3) — a `BINTABLE` with `ZTABLE = T` —
    /// and uncompress it into the original [`BinTable`]. Fixed-width and `P`/`Q`
    /// variable-length columns are supported (`GZIP_1`/`GZIP_2`/`RICE_1`/
    /// `NOCOMPRESS`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_table(&mut self, index: usize) -> Result<BinTable> {
        let hdu = checked_hdu(&self.hdus, index)?;
        if hdu.kind != HduKind::CompressedTable {
            return Err(FitsError::NotCompressedTable);
        }
        let container = self.data.table(hdu)?;
        let parts = table::uncompress_table(&hdu.header, container)?;
        BinTable::from_data(&parts.header, parts.data)
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J).
    /// Strings containing one or more blanks are reported as
    /// [`ChecksumStatus::Unknown`] because the standard reserves them for
    /// undefined or unknown checksum values.
    pub fn verify_checksum(&mut self, index: usize) -> Result<ChecksumReport> {
        let hdu = checked_hdu(&self.hdus, index)?;
        let stored_datasum = hdu.header.get_text("DATASUM")?;
        let stored_checksum = hdu.header.get_text("CHECKSUM")?;
        let expected_datasum = stored_datasum
            .filter(|value| !value.is_empty() && !is_unknown_checksum(value))
            .and_then(|value| value.trim().parse::<u32>().ok());
        let verify_whole_hdu =
            stored_checksum.is_some_and(|value| !value.is_empty() && !is_unknown_checksum(value));
        let mut report = ChecksumReport {
            datasum: initial_status(stored_datasum),
            checksum: initial_status(stored_checksum),
        };
        if expected_datasum.is_none() && !verify_whole_hdu {
            return Ok(report);
        }
        let lengths = DataLengths::new(hdu.data_bytes)?;
        // The block-padded data unit (length = the padded size — the checksum covers
        // the block fill too).
        let unit = self.data.slice(hdu.data_offset, lengths.padded)?;
        let data_sum = checksum::accumulate(unit, 0);
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
}

/// The HDU at `index`, or [`FitsError::IndexOutOfBounds`]. A free function over the
/// records, so a read can hold an HDU while it fetches bytes from the source.
fn checked_hdu(hdus: &[Hdu], index: usize) -> Result<&Hdu> {
    hdus.get(index).ok_or(FitsError::IndexOutOfBounds {
        indexed: Indexed::Hdu,
        index,
        len: hdus.len(),
    })
}

/// Index of the first HDU of `kind` (any kind when `None`) that
/// [`Hdu::matches_extension`].
fn find_extension(
    hdus: &[Hdu],
    name: &str,
    version: Option<i64>,
    level: Option<i64>,
    kind: Option<HduKind>,
) -> Result<Option<usize>> {
    for (index, hdu) in hdus.iter().enumerate() {
        if kind.is_none_or(|kind| hdu.kind == kind)
            && hdu.matches_extension(name, version, level)?
        {
            return Ok(Some(index));
        }
    }
    Ok(None)
}

fn resolve_column_selector(schema: &TableSchema, selector: &ColumnSelector) -> Result<usize> {
    match selector {
        ColumnSelector::Index(index) => {
            column::validate_index(*index, schema.columns.len())?;
            Ok(*index)
        }
        ColumnSelector::Name(name) => schema.column_index_checked(name),
    }
}

/// Where a stored integrity keyword starts before anything is recomputed: absent,
/// the standard all-blank "unknown", or provisionally invalid until it verifies.
fn initial_status(stored: Option<&str>) -> ChecksumStatus {
    match stored {
        None => ChecksumStatus::Absent,
        Some(value) if is_unknown_checksum(value) => ChecksumStatus::Unknown,
        Some(_) => ChecksumStatus::Invalid,
    }
}

fn is_unknown_checksum(value: &str) -> bool {
    !value.is_empty() && value.bytes().all(|byte| byte == b' ')
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

#[derive(Debug, Clone, Copy)]
struct DataLengths {
    data: usize,
    padded: usize,
}

impl DataLengths {
    fn new(data_bytes: u64) -> Result<DataLengths> {
        let data = usize::try_from(data_bytes)
            .map_err(|_| FitsError::DataUnitTooLarge { bytes: data_bytes })?;
        let padded_bytes = padded_len(data_bytes);
        let padded = usize::try_from(padded_bytes).map_err(|_| FitsError::DataUnitTooLarge {
            bytes: padded_bytes,
        })?;
        Ok(DataLengths { data, padded })
    }
}

/// Outcome of scanning for the next header unit.
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
                    return Err(FitsError::MissingKeyword { name: "SIMPLE" });
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
    use crate::reader::FitsReader;
    use crate::reader::StreamReader;
    use std::fs::File;

    /// Open a fixture from `tests/data/fits` as a streaming reader, reporting which
    /// file failed rather than a bare unwrap — every read-path test starts here.
    pub(crate) fn open_fixture(name: &str) -> StreamReader<File> {
        let path = format!("tests/data/fits/{name}");
        FitsReader::open(File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}")))
            .unwrap_or_else(|e| panic!("parse {name}: {e}"))
    }
}

#[cfg(test)]
mod tests;
