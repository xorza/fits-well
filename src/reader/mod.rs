use std::io::Read;
use std::io::Seek;
use std::ops::Range;

use crate::allocation;
use crate::ascii::AsciiTable;
use crate::bitpix::Bitpix;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::padded_len;
use crate::checksum;
use crate::data::BorrowedImage;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::ImageView;
use crate::data::ReadImage;
use crate::data::Scaling;
use crate::data::shape_product;
use crate::data::swap_into_words;
use crate::data::view_words;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::groups::RandomGroups;
use crate::hdu::HduKind;
use crate::hdu::HduPosition;
use crate::hdu::HduRole;
use crate::hdu::data_extent;
use crate::header::Header;
use crate::header::card::is_end_record;
use crate::table_impl::BinTable;
use crate::table_impl::Column;
use crate::table_impl::ColumnData;
use crate::table_impl::TableSchema;
use crate::table_impl::TformKind;
use crate::table_impl::decode_fixed_cell;
use crate::table_impl::decode_vla_cell;
use crate::table_impl::descriptor::PqDescriptor;
use crate::wcs::Wcs;
use crate::wcs::image_axis_count;
use crate::wcs::tabular;

pub(crate) mod source;

use source::SliceSource;
use source::Source;
use source::StreamSource;

#[cfg(feature = "compression")]
use crate::compress::{decode, table};

/// One Header/Data Unit located by the reader.
///
/// The data unit itself is read lazily via [`FitsReader::read_data_raw`]; this
/// record only carries the parsed header, the inferred [`HduKind`], and the
/// data unit's byte range within the source.
#[derive(Debug)]
pub struct Hdu {
    pub header: Header,
    pub kind: HduKind,
    /// One's-complement sum of the exact block-padded header bytes as read.
    pub(crate) header_sum: u32,
    /// Byte offset of the data unit from the start of the source.
    pub(crate) data_offset: u64,
    /// Unpadded data length (`Nbits / 8`) — where the meaningful data ends within
    /// the padded unit. The on-disk length is `padded_len(data_bytes)`.
    pub(crate) data_bytes: u64,
}

impl Hdu {
    /// Validate that this HDU is a readable plain image array — a `Primary`/`Image`
    /// kind with no group structure — the shared precondition of
    /// [`FitsReader::read_image`] and [`FitsReader::read_image_view`].
    fn ensure_plain_image(&self) -> Result<()> {
        if !matches!(self.kind, HduKind::Primary | HduKind::Image) {
            return Err(FitsError::NotAnImage);
        }
        // §4.3: a plain image array has no group structure, so reserved extension /
        // random-groups counts cannot qualify the samples returned here.
        if self.header.pcount()? != 0 || self.header.gcount()? != 1 {
            return Err(FitsError::ImageHasGroups);
        }
        Ok(())
    }
}

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
    source: S,
    /// The scanned HDU records; exposed read-only via [`FitsReader::hdus`].
    pub(crate) hdus: Vec<Hdu>,
    /// Reused staging buffer for the seeking-source reads: a [`StreamSource`] copies
    /// each data unit here before decoding (an in-memory source borrows instead, so
    /// this stays empty). Grows once to the largest unit touched, then holds.
    scratch: Vec<u8>,
    /// Reused assembly buffer for disjoint image-section runs.
    section_scratch: Vec<u8>,
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
        self.source.into_inner()
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
        self.source.into_bytes()
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
                    hdus.push(Hdu {
                        header,
                        kind,
                        header_sum: sum,
                        data_offset,
                        data_bytes: extent.data_bytes,
                    });
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
            source,
            hdus,
            scratch,
            section_scratch: Vec::new(),
        })
    }

    /// The HDU at `index`, or [`FitsError::HduIndexOutOfBounds`] — the checked form
    /// the `read_*` methods bound-check through.
    fn checked_hdu(&self, index: usize) -> Result<&Hdu> {
        self.hdus.get(index).ok_or(FitsError::HduIndexOutOfBounds {
            index,
            len: self.hdus.len(),
        })
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
        for (index, hdu) in self.hdus.iter().enumerate() {
            let name_matches = hdu
                .header
                .get_text("EXTNAME")?
                .is_some_and(|value| value.eq_ignore_ascii_case(name));
            if !name_matches {
                continue;
            }
            let version_matches = match version {
                Some(value) => hdu.header.get_integer("EXTVER")?.unwrap_or(1) == value,
                None => true,
            };
            if version_matches {
                return Ok(Some(index));
            }
        }
        Ok(None)
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
            .filter(|(_, h)| match h.kind {
                HduKind::Image | HduKind::CompressedImage => true,
                HduKind::Primary => h.header.naxis().is_ok_and(|n| n > 0),
                _ => false,
            })
            .map(|(i, _)| i)
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
        let hdu = self.checked_hdu(index)?;
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let lengths = DataLengths::new(data_bytes)?;
        let bytes = self.source.read_owned(data_offset, lengths.padded)?;
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
    /// `compression` feature; without it a `ZIMAGE` HDU reads as a plain `BINTABLE`, so
    /// this returns [`FitsError::NotAnImage`]). Either way, reach for the samples via
    /// [`ReadImage::u8`] (zero-copy `BITPIX = 8`), [`ReadImage::decode`] (host-endian),
    /// or [`ReadImage::physical`] (scaled). The result borrows the reader, so handle
    /// one image before reading the next.
    pub fn read_image(&mut self, index: usize) -> Result<ReadImage<'_>> {
        // §10.1: a tiled-compressed image is classified [`HduKind::CompressedImage`]
        // (a `ZIMAGE` BINTABLE). Route it through the decompressor so callers see one
        // image API regardless of storage.
        #[cfg(feature = "compression")]
        if self.checked_hdu(index)?.kind == HduKind::CompressedImage {
            let table = self.read_table(index)?;
            let img = decode::decompress_image(&self.hdus[index].header, &table)?;
            return Ok(ReadImage::decoded(img.samples, img.shape, img.scaling));
        }

        let hdu = self.checked_hdu(index)?;
        hdu.ensure_plain_image()?;
        let bitpix = hdu.header.bitpix()?;
        let shape = hdu.header.axes()?;
        let scaling = hdu.header.scaling()?;
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let lengths = DataLengths::new(data_bytes)?;
        let unit = self
            .source
            .slice(data_offset, lengths.padded, &mut self.scratch)?;
        let bytes = &unit[..lengths.data];

        // With PCOUNT=0/GCOUNT=1 (checked above), `data_extent` sized the unit as
        // `elem · Π(axes)`, so the borrowed data is exactly `shape_product` elements
        // wide. This is an invariant between `data_extent` and the shape, not a
        // runtime failure mode — assert it rather than return an error that can't occur.
        let expected_bytes = shape_product(&shape)?
            .checked_mul(bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        debug_assert_eq!(
            bytes.len(),
            expected_bytes,
            "image data length must match the axis product"
        );
        Ok(ReadImage::raw(shape, bitpix, scaling, bytes))
    }

    /// Read an image as a borrowed, host-endian [`ImageView`], byte-swapping into the
    /// caller-owned `scratch` — the fast, low-copy path for a loop that processes each
    /// image and moves on. Where [`read_image`](FitsReader::read_image)`.decode()`
    /// allocates a fresh owned buffer per call (page-fault-bound — profiling found
    /// that dominates a plain typed read), this reuses `scratch`, so a hot loop pays
    /// the output allocation once and reuses it across reads — even across differing
    /// `BITPIX`. The caller owns `scratch`, so the reader retains nothing image-sized;
    /// pass the same `Vec` each call and drop it when the loop ends.
    ///
    /// `scratch` is `Vec<u64>` so the swapped samples stay 8-byte aligned for the
    /// typed views. A `BITPIX = 8` image needs no swap and the view borrows the source
    /// bytes directly (zero-copy, `scratch` untouched); a compressed image is
    /// decompressed directly into `scratch`. The view borrows the reader and
    /// `scratch`, so handle one image before reading the next. For samples you need to
    /// keep, use [`ReadImage::decode`].
    pub fn read_image_view<'a>(
        &'a mut self,
        index: usize,
        scratch: &'a mut Vec<u64>,
    ) -> Result<BorrowedImage<'a>> {
        #[cfg(feature = "compression")]
        if self.checked_hdu(index)?.kind == HduKind::CompressedImage {
            let table = self.read_table(index)?;
            return decode::decompress_image_into_words(&self.hdus[index].header, &table, scratch);
        }

        let hdu = self.checked_hdu(index)?;
        hdu.ensure_plain_image()?;
        let bitpix = hdu.header.bitpix()?;
        let shape = hdu.header.axes()?;
        let scaling = hdu.header.scaling()?;
        let lengths = DataLengths::new(hdu.data_bytes)?;
        let data_offset = hdu.data_offset;
        // `hdu` (the self.hdus borrow) is unused past here, so the source/scratch
        // borrows below don't conflict — same staging as `read_image`.
        let unit = self
            .source
            .slice(data_offset, lengths.padded, &mut self.scratch)?;
        let be = &unit[..lengths.data];
        if bitpix == Bitpix::U8 {
            // No byte-swap: the on-disk bytes already are the host-endian samples, so
            // borrow them straight (zero-copy) — `scratch` stays untouched.
            return Ok(BorrowedImage {
                shape,
                scaling,
                samples: ImageView::U8(be),
            });
        }
        swap_into_words(be, bitpix, scratch);
        Ok(BorrowedImage {
            shape,
            scaling,
            samples: view_words(scratch, bitpix, lengths.data),
        })
    }

    /// Read a checked N-dimensional rectangular image section. Axis ranges are
    /// zero-based, half-open, and ordered fastest-axis first.
    pub fn read_image_section(&mut self, index: usize, ranges: &[Range<usize>]) -> Result<Image> {
        #[cfg(feature = "compression")]
        if self.checked_hdu(index)?.kind == HduKind::CompressedImage {
            let header = self.hdus[index].header.clone();
            let tile_rows = decode::compressed_image_tile_rows(&header, ranges)?;
            let schema = self.table_schema(index)?;
            let table = self.read_table_sparse_rows(index, &schema, &tile_rows)?;
            return decode::decompress_image_section(&header, &table, &tile_rows, ranges);
        }

        let layout = self.read_plain_image_section(index, ranges)?;
        Image::new_scaled(
            layout.shape,
            ImageData::decode(&self.section_scratch, layout.bitpix),
            layout.scaling,
        )
    }

    /// Scratch-reusing counterpart to [`FitsReader::read_image_section`].
    pub fn read_image_section_view<'a>(
        &'a mut self,
        index: usize,
        ranges: &[Range<usize>],
        words: &'a mut Vec<u64>,
    ) -> Result<BorrowedImage<'a>> {
        #[cfg(feature = "compression")]
        if self.checked_hdu(index)?.kind == HduKind::CompressedImage {
            let header = self.hdus[index].header.clone();
            let tile_rows = decode::compressed_image_tile_rows(&header, ranges)?;
            let schema = self.table_schema(index)?;
            let table = self.read_table_sparse_rows(index, &schema, &tile_rows)?;
            return decode::decompress_image_section_into_words(
                &header, &table, &tile_rows, ranges, words,
            );
        }

        let layout = self.read_plain_image_section(index, ranges)?;
        let nbytes = self.section_scratch.len();
        let samples = if layout.bitpix == Bitpix::U8 {
            ImageView::U8(&self.section_scratch)
        } else {
            swap_into_words(&self.section_scratch, layout.bitpix, words);
            view_words(words, layout.bitpix, nbytes)
        };
        Ok(BorrowedImage {
            shape: layout.shape,
            scaling: layout.scaling,
            samples,
        })
    }

    fn read_plain_image_section(
        &mut self,
        index: usize,
        ranges: &[Range<usize>],
    ) -> Result<ImageSectionLayout> {
        let hdu = self.checked_hdu(index)?;
        hdu.ensure_plain_image()?;
        let shape = hdu.header.axes()?;
        let selected_shape = validate_image_region(ranges, &shape)?;
        let scaling = hdu.header.scaling()?;
        let bitpix = hdu.header.bitpix()?;
        let data_offset = hdu.data_offset;
        let nbytes = shape_product(&selected_shape)?
            .checked_mul(bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        self.section_scratch.clear();
        allocation::try_reserve(&mut self.section_scratch, nbytes)?;
        let source = &mut self.source;
        let scratch = &mut self.scratch;
        let section = &mut self.section_scratch;
        visit_image_region_runs(&shape, ranges, &selected_shape, bitpix.elem_size(), |run| {
            let bytes = source.slice(
                data_offset
                    .checked_add(run.offset as u64)
                    .ok_or(FitsError::DataUnitOverflow)?,
                run.len,
                scratch,
            )?;
            section.extend_from_slice(bytes);
            Ok(())
        })?;
        debug_assert_eq!(self.section_scratch.len(), nbytes);
        Ok(ImageSectionLayout {
            shape: selected_shape,
            scaling,
            bitpix,
        })
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`BinTable::column_by_idx`]. Errors with
    /// [`FitsError::NotABinTable`] for any other HDU kind.
    pub fn read_table(&mut self, index: usize) -> Result<BinTable> {
        let hdu = self.checked_hdu(index)?;
        // Compressed images/tables are structurally BINTABLEs; the compression layer
        // reads their raw table form through here, so accept those kinds too.
        if !matches!(
            hdu.kind,
            HduKind::BinTable | HduKind::CompressedImage | HduKind::CompressedTable
        ) {
            return Err(FitsError::NotABinTable);
        }
        let unit = self.read_data_raw(index)?;
        BinTable::from_data(&self.hdus[index].header, unit.bytes)
    }

    /// Parse a binary-table schema from its header without reading table bytes.
    pub fn table_schema(&self, index: usize) -> Result<TableSchema> {
        let hdu = self.checked_hdu(index)?;
        if !matches!(
            hdu.kind,
            HduKind::BinTable | HduKind::CompressedImage | HduKind::CompressedTable
        ) {
            return Err(FitsError::NotABinTable);
        }
        BinTable::schema(&hdu.header)
    }

    /// Materialize only the requested contiguous row range, including only the VLA
    /// heap cells referenced by those rows.
    pub fn read_table_rows(&mut self, index: usize, rows: Range<usize>) -> Result<BinTable> {
        let schema = self.table_schema(index)?;
        validate_row_range(&rows, schema.nrows)?;
        self.read_table_range(index, &schema, rows, None)
    }

    fn read_table_range(
        &mut self,
        index: usize,
        schema: &TableSchema,
        rows: Range<usize>,
        selected_columns: Option<&[usize]>,
    ) -> Result<BinTable> {
        let hdu = self.checked_hdu(index)?;
        let data_offset = hdu.data_offset;
        let main_len = rows
            .len()
            .checked_mul(schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let data = if rows.is_empty() {
            Vec::new()
        } else {
            let offset = data_offset
                .checked_add(
                    u64::try_from(rows.start)
                        .ok()
                        .and_then(|row| row.checked_mul(schema.row_len as u64))
                        .ok_or(FitsError::DataUnitOverflow)?,
                )
                .ok_or(FitsError::DataUnitOverflow)?;
            self.source.read_owned(offset, main_len)?
        };
        self.finish_table_selection(index, schema, rows.len(), data, selected_columns)
    }

    #[cfg(feature = "compression")]
    fn read_table_sparse_rows(
        &mut self,
        index: usize,
        schema: &TableSchema,
        rows: &[usize],
    ) -> Result<BinTable> {
        let data_offset = self.checked_hdu(index)?.data_offset;
        let main_len = rows
            .len()
            .checked_mul(schema.row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let mut data = allocation::try_zeroed(0u8, main_len)?;
        for (destination, &source_row) in rows.iter().enumerate() {
            if source_row >= schema.nrows {
                return Err(FitsError::RowRangeOutOfBounds {
                    start: source_row,
                    end: source_row.saturating_add(1),
                    len: schema.nrows,
                });
            }
            let offset = data_offset
                .checked_add(
                    u64::try_from(source_row)
                        .ok()
                        .and_then(|row| row.checked_mul(schema.row_len as u64))
                        .ok_or(FitsError::DataUnitOverflow)?,
                )
                .ok_or(FitsError::DataUnitOverflow)?;
            let row = self
                .source
                .slice(offset, schema.row_len, &mut self.scratch)?;
            let start = destination * schema.row_len;
            data[start..start + schema.row_len].copy_from_slice(row);
        }
        self.finish_table_selection(index, schema, rows.len(), data, None)
    }

    fn finish_table_selection(
        &mut self,
        index: usize,
        schema: &TableSchema,
        row_count: usize,
        mut data: Vec<u8>,
        selected_columns: Option<&[usize]>,
    ) -> Result<BinTable> {
        let data_offset = self.checked_hdu(index)?.data_offset;
        let selected_columns = selected_columns.map(|indices| {
            let mut selected = vec![false; schema.columns.len()];
            for &index in indices {
                selected[index] = true;
            }
            selected
        });
        let mut heap = Vec::new();
        for compact_row in 0..row_count {
            for (column_index, column) in schema.columns.iter().enumerate() {
                let wide = match column.tform.kind {
                    TformKind::ArrayDesc32 => false,
                    TformKind::ArrayDesc64 => true,
                    _ => continue,
                };
                let slot_start = compact_row * schema.row_len + column.byte_offset;
                let slot_end = slot_start + column.tform.byte_width();
                if column.tform.repeat == 0 {
                    continue;
                }
                if selected_columns
                    .as_ref()
                    .is_some_and(|selected| !selected[column_index])
                {
                    write_pq_descriptor(&mut data[slot_start..slot_end], wide, 0, 0)?;
                    continue;
                }
                let slot = &data[slot_start..slot_end];
                let span = vla_cell_span(schema, column, slot, wide)?;
                let bytes = self.source.slice(
                    data_offset
                        .checked_add(span.heap_range.start as u64)
                        .ok_or(FitsError::DataUnitOverflow)?,
                    span.heap_range.len(),
                    &mut self.scratch,
                )?;
                let new_offset = heap.len();
                write_pq_descriptor(
                    &mut data[slot_start..slot_end],
                    wide,
                    span.count as u64,
                    new_offset as u64,
                )?;
                heap.extend_from_slice(bytes);
            }
        }
        data.extend_from_slice(&heap);
        let mut header = self.hdus[index].header.clone();
        header.set_internal("NAXIS2", row_count as i64);
        header.set_internal("PCOUNT", heap.len() as i64);
        header.remove_all("THEAP");
        header.remove_all("CHECKSUM");
        header.remove_all("DATASUM");
        BinTable::from_data(&header, data)
    }

    /// Decode selected columns over a row range without reading unrelated rows.
    pub fn read_table_columns(
        &mut self,
        index: usize,
        rows: Range<usize>,
        columns: &[ColumnSelector],
    ) -> Result<TableSelection> {
        let schema = self.table_schema(index)?;
        validate_row_range(&rows, schema.nrows)?;
        let selected_indices = columns
            .iter()
            .map(|selector| resolve_column_selector(&schema, selector))
            .collect::<Result<Vec<_>>>()?;
        let table = self.read_table_range(index, &schema, rows.clone(), Some(&selected_indices))?;
        let selected_rows = rows.clone();
        let mut decoded = Vec::with_capacity(columns.len());
        for &index in &selected_indices {
            let column = table.column_by_idx(index)?;
            let descriptor = column.descriptor().clone();
            let data = if matches!(
                descriptor.tform.kind,
                TformKind::ArrayDesc32 | TformKind::ArrayDesc64
            ) {
                TableColumnData::Variable(column.vla()?)
            } else {
                TableColumnData::Fixed(column.raw()?)
            };
            decoded.push(SelectedColumn { descriptor, data });
        }
        Ok(TableSelection {
            rows: selected_rows,
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
        let schema = self.table_schema(index)?;
        let end = row.checked_add(1).ok_or(FitsError::RowRangeOutOfBounds {
            start: row,
            end: usize::MAX,
            len: schema.nrows,
        })?;
        validate_row_range(&(row..end), schema.nrows)?;
        let column_index = resolve_column_selector(&schema, &column)?;
        let column = &schema.columns[column_index];
        let data_offset = self.checked_hdu(index)?.data_offset;
        let cell_offset = data_offset
            .checked_add(
                u64::try_from(row)
                    .ok()
                    .and_then(|row| row.checked_mul(schema.row_len as u64))
                    .and_then(|offset| offset.checked_add(column.byte_offset as u64))
                    .ok_or(FitsError::DataUnitOverflow)?,
            )
            .ok_or(FitsError::DataUnitOverflow)?;
        let width = column.tform.byte_width();
        let wide = match column.tform.kind {
            TformKind::ArrayDesc32 => false,
            TformKind::ArrayDesc64 => true,
            _ => {
                let bytes = self.source.slice(cell_offset, width, &mut self.scratch)?;
                return Ok(decode_fixed_cell(column, bytes));
            }
        };
        if column.tform.repeat == 0 {
            return decode_vla_cell(column, &[], 0);
        }
        let descriptor_bytes = self.source.slice(cell_offset, width, &mut self.scratch)?;
        let span = vla_cell_span(&schema, column, descriptor_bytes, wide)?;
        let bytes = self.source.slice(
            data_offset
                .checked_add(span.heap_range.start as u64)
                .ok_or(FitsError::DataUnitOverflow)?,
            span.heap_range.len(),
            &mut self.scratch,
        )?;
        decode_vla_cell(column, bytes, span.count)
    }

    /// Parse an HDU's WCS and resolve any standard `-TAB` coordinate arrays from
    /// their referenced `BINTABLE` extensions. Header-only WCS descriptions use the
    /// same path as [`Header::wcs`]; this method is required for `-TAB` because its
    /// coordinate values and optional index vectors live outside the source header.
    pub fn read_wcs(&mut self, index: usize, alt: Option<char>) -> Result<Wcs> {
        let header = self.checked_hdu(index)?.header.clone();
        let descriptors = tabular::descriptors(&header, image_axis_count(&header, alt)?, alt)?;
        if descriptors.is_empty() {
            return Wcs::from_header(&header, alt);
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
            let mut table_index = None;
            for (index, hdu) in self.hdus.iter().enumerate() {
                if hdu.kind != HduKind::BinTable {
                    continue;
                }
                let Some(name) = hdu.header.get_text("EXTNAME")? else {
                    continue;
                };
                let version = hdu.header.get_integer("EXTVER")?.unwrap_or(1);
                let level = hdu.header.get_integer("EXTLEVEL")?.unwrap_or(1);
                if name.eq_ignore_ascii_case(&reference.extension_name)
                    && version == reference.extension_version
                    && level == reference.extension_level
                {
                    table_index = Some(index);
                    break;
                }
            }
            let table_index = table_index.ok_or_else(|| FitsError::InvalidValue {
                card: format!(
                    "TAB BINTABLE {:?}, EXTVER {}, EXTLEVEL {} was not found",
                    reference.extension_name,
                    reference.extension_version,
                    reference.extension_level
                ),
            })?;
            let schema = self.table_schema(table_index)?;
            let mut selected = vec![false; schema.columns.len()];
            for descriptor in &group {
                for name in descriptor.referenced_columns() {
                    selected[resolve_column_name(&schema, name)?] = true;
                }
            }
            let selected_indices = selected
                .into_iter()
                .enumerate()
                .filter_map(|(index, selected)| selected.then_some(index))
                .collect::<Vec<_>>();
            let table = self.read_table_range(
                table_index,
                &schema,
                0..usize::from(schema.nrows != 0),
                Some(&selected_indices),
            )?;
            for descriptor in group {
                transforms.push(tabular::TabularTransform::from_table(descriptor, &table)?);
            }
        }
        Wcs::from_header_with_tabular(&header, alt, transforms)
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
        let hdu = self.checked_hdu(index)?;
        if hdu.kind != HduKind::RandomGroups {
            return Err(FitsError::NotRandomGroups);
        }
        let (data_offset, data_bytes) = (hdu.data_offset, hdu.data_bytes);
        let lengths = DataLengths::new(data_bytes)?;
        let unit = self
            .source
            .slice(data_offset, lengths.padded, &mut self.scratch)?;
        RandomGroups::from_data(&self.hdus[index].header, &unit[..lengths.data])
    }

    /// Read a tiled-compressed table (§10.3) — a `BINTABLE` with `ZTABLE = T` —
    /// and uncompress it into the original [`BinTable`]. Fixed-width and `P`/`Q`
    /// variable-length columns are supported (`GZIP_1`/`GZIP_2`/`RICE_1`/
    /// `NOCOMPRESS`). Requires the `compression` feature.
    #[cfg(feature = "compression")]
    pub fn read_compressed_table(&mut self, index: usize) -> Result<BinTable> {
        let table = self.read_table(index)?;
        let parts = table::uncompress_table(&self.hdus[index].header, &table)?;
        BinTable::from_data(&parts.header, parts.data)
    }

    /// Verify the `DATASUM`/`CHECKSUM` integrity keywords of an HDU (§J).
    /// Strings containing one or more blanks are reported as
    /// [`ChecksumStatus::Unknown`] because the standard reserves them for
    /// undefined or unknown checksum values.
    pub fn verify_checksum(&mut self, index: usize) -> Result<ChecksumReport> {
        let hdu = self.checked_hdu(index)?;
        let stored_datasum = hdu.header.get_text("DATASUM")?;
        let stored_checksum = hdu.header.get_text("CHECKSUM")?;
        let expected_datasum = stored_datasum
            .filter(|value| !value.is_empty() && !is_unknown_checksum(value))
            .and_then(|value| value.trim().parse::<u32>().ok());
        let verify_whole_hdu =
            stored_checksum.is_some_and(|value| !value.is_empty() && !is_unknown_checksum(value));
        let mut report = ChecksumReport {
            datasum: match stored_datasum {
                None => ChecksumStatus::Absent,
                Some(value) if is_unknown_checksum(value) => ChecksumStatus::Unknown,
                Some(_) => ChecksumStatus::Invalid,
            },
            checksum: match stored_checksum {
                None => ChecksumStatus::Absent,
                Some(value) if is_unknown_checksum(value) => ChecksumStatus::Unknown,
                Some(_) => ChecksumStatus::Invalid,
            },
        };
        if expected_datasum.is_none() && !verify_whole_hdu {
            return Ok(report);
        }
        let (data_offset, data_bytes, header_sum) =
            (hdu.data_offset, hdu.data_bytes, hdu.header_sum);
        let lengths = DataLengths::new(data_bytes)?;
        // The block-padded data unit (length = the padded size — the checksum covers
        // the block fill too).
        let unit = self
            .source
            .slice(data_offset, lengths.padded, &mut self.scratch)?;
        let data_sum = checksum::accumulate(unit, 0);
        if let Some(expected) = expected_datasum {
            report.datasum = if expected == data_sum {
                ChecksumStatus::Valid
            } else {
                ChecksumStatus::Invalid
            };
        }
        if verify_whole_hdu {
            report.checksum = if checksum::combine(header_sum, data_sum) == 0xFFFF_FFFF {
                ChecksumStatus::Valid
            } else {
                ChecksumStatus::Invalid
            };
        }
        Ok(report)
    }
}

#[derive(Debug)]
struct VlaCellSpan {
    count: usize,
    heap_range: Range<usize>,
}

fn vla_cell_span(
    schema: &TableSchema,
    column: &Column,
    descriptor_bytes: &[u8],
    wide: bool,
) -> Result<VlaCellSpan> {
    let descriptor = PqDescriptor::decode(descriptor_bytes, wide)?;
    let element_type = column
        .tform
        .vla_elem
        .expect("validated VLA format carries an element type");
    Ok(VlaCellSpan {
        count: descriptor.count,
        heap_range: descriptor.heap_range(element_type, schema.heap_offset, schema.heap_end)?,
    })
}

fn validate_row_range(rows: &Range<usize>, len: usize) -> Result<()> {
    if rows.start > rows.end || rows.end > len {
        return Err(FitsError::RowRangeOutOfBounds {
            start: rows.start,
            end: rows.end,
            len,
        });
    }
    Ok(())
}

fn resolve_column_selector(schema: &TableSchema, selector: &ColumnSelector) -> Result<usize> {
    match selector {
        ColumnSelector::Index(index) if *index < schema.columns.len() => Ok(*index),
        ColumnSelector::Index(index) => Err(FitsError::ColumnIndexOutOfBounds {
            index: *index,
            len: schema.columns.len(),
        }),
        ColumnSelector::Name(name) => resolve_column_name(schema, name),
    }
}

fn resolve_column_name(schema: &TableSchema, name: &str) -> Result<usize> {
    schema
        .columns
        .iter()
        .position(|column| {
            column
                .name
                .as_deref()
                .is_some_and(|candidate| candidate.eq_ignore_ascii_case(name))
        })
        .ok_or_else(|| FitsError::ColumnNotFound {
            name: name.to_string(),
        })
}

#[derive(Debug)]
struct ImageSectionLayout {
    shape: Vec<usize>,
    scaling: Scaling,
    bitpix: Bitpix,
}

#[derive(Debug, Clone, Copy)]
struct ByteRun {
    offset: usize,
    len: usize,
}

fn validate_image_region(ranges: &[Range<usize>], shape: &[usize]) -> Result<Vec<usize>> {
    if ranges.len() != shape.len() {
        return Err(FitsError::ImageRegionRankMismatch {
            region_rank: ranges.len(),
            image_rank: shape.len(),
        });
    }
    let mut selected = Vec::with_capacity(shape.len());
    for (axis, (range, &len)) in ranges.iter().zip(shape).enumerate() {
        if range.start > range.end || range.end > len {
            return Err(FitsError::ImageRegionOutOfBounds {
                axis,
                start: range.start,
                end: range.end,
                len,
            });
        }
        selected.push(range.end - range.start);
    }
    Ok(selected)
}

fn visit_image_region_runs(
    shape: &[usize],
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    element_size: usize,
    mut visit: impl FnMut(ByteRun) -> Result<()>,
) -> Result<()> {
    debug_assert_eq!(shape.len(), ranges.len());
    debug_assert_eq!(shape.len(), selected_shape.len());
    if selected_shape.is_empty() || selected_shape.contains(&0) {
        return Ok(());
    }
    let row_count = selected_shape[1..]
        .iter()
        .try_fold(1usize, |count, &len| count.checked_mul(len))
        .ok_or(FitsError::DataUnitOverflow)?;
    let row_bytes = selected_shape[0]
        .checked_mul(element_size)
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut pending: Option<ByteRun> = None;
    for row_index in 0..row_count {
        let mut remainder = row_index;
        let mut element_offset = ranges[0].start;
        let mut stride = shape[0];
        for axis in 1..shape.len() {
            let coordinate = ranges[axis].start + remainder % selected_shape[axis];
            remainder /= selected_shape[axis];
            element_offset = coordinate
                .checked_mul(stride)
                .and_then(|value| element_offset.checked_add(value))
                .ok_or(FitsError::DataUnitOverflow)?;
            if axis + 1 < shape.len() {
                stride = stride
                    .checked_mul(shape[axis])
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
        }
        let next = ByteRun {
            offset: element_offset
                .checked_mul(element_size)
                .ok_or(FitsError::DataUnitOverflow)?,
            len: row_bytes,
        };
        if let Some(previous) = pending.as_mut() {
            let end = previous
                .offset
                .checked_add(previous.len)
                .ok_or(FitsError::DataUnitOverflow)?;
            if end == next.offset {
                previous.len = previous
                    .len
                    .checked_add(next.len)
                    .ok_or(FitsError::DataUnitOverflow)?;
                continue;
            }
        }
        if let Some(previous) = pending.replace(next) {
            visit(previous)?;
        }
    }
    if let Some(run) = pending {
        visit(run)?;
    }
    Ok(())
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
    block.chunks_exact(CARD_SIZE).any(is_end_record)
}

#[cfg(test)]
mod tests;
