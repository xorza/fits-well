use std::io::Read;
use std::io::Seek;
use std::ops::{Bound, Range, RangeBounds};

use crate::alternate::Alternate;
use crate::ascii::Ascii;
use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
#[cfg(feature = "compression")]
use crate::bintable::selection::validate_row_range;
use crate::bintable::selection::{Selection, TableCell, TableRows, TableSelection, row_range};
use crate::bintable::table_view::TableView;
use crate::checksum::checksum_report::ChecksumReport;
use crate::column::ColumnSelector;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_indexes::TileIndexes;
use crate::fits_error::ExpectedHdu;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::{TabularDefect, WcsDefect};
use crate::groups::RandomGroups;
use crate::hdu::Hdu;
use crate::hdu::hdu_kind::HduKind;
use crate::hdu::hdu_selector::HduSelector;
use crate::hdu::hdus::Hdus;
use crate::ragged::Ragged;
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::DataSource;
#[cfg(feature = "compression")]
use crate::reader::data_source::TiledRead;
use crate::reader::image_reader::ImageReader;
use crate::reader::plain_read::{PlainRead, StoredRuns};
use crate::world_coordinates::Wcs;
use crate::world_coordinates::tabular;
use crate::world_coordinates::tabular::{TableArray, TabularArrays, TabularDescriptor};

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod data_checksum;
pub(crate) mod data_source;
pub(crate) mod image_reader;
mod plain_read;
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
    /// The tile index of each compressed image opened, which every later handle on that
    /// image reuses.
    #[cfg(feature = "compression")]
    tile_indexes: TileIndexes,
}

/// The reader's reused buffers a section or selection read fills and the result
/// borrows: the selected section's shape, the tile rows it touches and the odometer
/// that walks them, and a compacted table selection with the columns it selects.
#[derive(Debug, Default)]
struct Scratch {
    section_shape: Vec<usize>,
    #[cfg(feature = "compression")]
    tile_rows: Vec<usize>,
    #[cfg(feature = "compression")]
    tile_coordinates: Vec<usize>,
    selection: Vec<u8>,
    column_indices: Vec<usize>,
    column_mask: Vec<bool>,
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
        let hdus = Hdus::scan(&mut source)?;
        Ok(Self {
            hdus,
            data: DataSource::new(source),
            scratch: Scratch::default(),
            #[cfg(feature = "compression")]
            tile_indexes: TileIndexes::default(),
        })
    }

    /// The scanned HDU records, read-only and in file order — each carrying its
    /// parsed [`Header`](crate::header::Header) and [`HduKind`]. Every read names its
    /// HDU with an [`HduSelector`]: an index into this slice, `"SCI"`, or `("SCI", 2)`.
    pub fn hdus(&self) -> &[Hdu] {
        self.hdus.as_slice()
    }

    /// The HDU `hdu` names: the first that [`Hdu::matches`] it. Errors with
    /// [`FitsError::IndexOutOfBounds`] for an index past the last HDU, with
    /// [`FitsError::DefectiveHdu`] for the index of a header that ended the list (see
    /// [`FitsReader::scan_defect`]), and with [`FitsError::HduNotFound`] for a name no HDU
    /// carries.
    pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu> {
        self.hdus.select(hdu.into())
    }

    /// The error of the header after the last HDU, when one did not read: the scan ends
    /// the HDU list there and keeps the HDUs before it, as cfitsio and astropy do.
    pub fn scan_defect(&self) -> Option<&FitsError> {
        self.hdus.defect()
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
    /// the parsed table's storage). An image read fills the caller's buffer or an owned
    /// image from the source's bytes, a run at a time — see [`FitsReader::image`].
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
        ImageReader::new(
            hdu,
            &mut self.data,
            &mut self.scratch,
            #[cfg(feature = "compression")]
            &mut self.tile_indexes,
        )
    }

    /// Read a `BINTABLE` extension and parse its column structure. Decode
    /// individual columns lazily with [`Table::column`]. Errors as
    /// [`Hdu::table_schema`] does for an HDU that is not a plain binary table.
    pub fn read_table<'h>(&mut self, hdu: impl Into<HduSelector<'h>>) -> Result<Table> {
        let hdu = self.hdus.select(hdu.into())?;
        let schema = hdu.table_schema()?;
        #[cfg(feature = "compression")]
        if let Some(tiled) = hdu.tiled_table()? {
            let read = TiledRead {
                hdu,
                tiled,
                rows: &(0..schema.extent.row_count),
                columns: None,
            };
            let decoded = self.data.decode(read, Vec::new())?;
            return Table::new(decoded.schema, decoded.data);
        }
        Table::new(schema.clone(), self.data.data_unit(hdu)?)
    }

    /// Materialize only the requested contiguous row range, including only the VLA
    /// heap cells referenced by those rows. Errors with [`FitsError::RowRangeOutOfBounds`]
    /// for rows that run backwards or past the table.
    pub fn read_table_rows<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: impl RangeBounds<usize>,
    ) -> Result<Table> {
        self.read_rows(hdu.into(), bounds(&rows))
    }

    fn read_rows(
        &mut self,
        hdu: HduSelector<'_>,
        rows: (Bound<usize>, Bound<usize>),
    ) -> Result<Table> {
        let hdu = self.hdus.select(hdu)?;
        let schema = hdu.table_schema()?;
        let rows = row_range(rows, schema.extent.row_count)?;
        let row_count = rows.len();
        let mut data = Vec::new();
        select_rows(&mut self.data, hdu, rows, None, &mut data)?;
        Table::new(schema.compacted(row_count, data.len()), data)
    }

    /// Decode selected columns over a row range without reading unrelated rows.
    pub fn read_table_columns<'h, 'c>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        rows: impl RangeBounds<usize>,
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
        rows: impl RangeBounds<usize>,
        columns: impl IntoIterator<Item = impl Into<ColumnSelector<'c>>>,
        out: &mut Selection,
    ) -> Result<()> {
        let rows = bounds(&rows);
        let filled = self.hdus.select(hdu.into()).and_then(|hdu| {
            let schema = hdu.table_schema()?;
            let rows = row_range(rows, schema.extent.row_count)?;
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
        if let Some(tiled) = hdu.tiled_table()? {
            let rows = row..row.saturating_add(1);
            validate_row_range(&rows, tiled.schema.extent.row_count)?;
            let selected = &mut self.scratch.column_mask;
            selected.clear();
            selected.resize(schema.columns.len(), false);
            selected[column] = true;
            let read = TiledRead {
                hdu,
                tiled,
                rows: &rows,
                columns: Some(selected),
            };
            let decoded = self.data.decoded(read)?;
            let cell = TableCell {
                schema: &decoded.schema,
                row: row - decoded.first_row,
                column,
            };
            return cell.read(&mut decoded.data.as_slice());
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
        let mut groups = Vec::<Vec<TabularDescriptor>>::new();
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
                let arrays = tabular_arrays(&descriptor, table)?;
                transforms.push(tabular::TabularTransform::from_arrays(&descriptor, arrays)?);
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
            len: self.data.unit_len(hdu)?,
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

/// The bounds of `rows`, owned.
fn bounds(rows: &impl RangeBounds<usize>) -> (Bound<usize>, Bound<usize>) {
    (rows.start_bound().cloned(), rows.end_bound().cloned())
}

/// The first-row arrays the `-TAB` transform `descriptor` declares reads from `table`: its
/// coordinate column, and the index column of each table axis that names one.
pub(crate) fn tabular_arrays(
    descriptor: &TabularDescriptor,
    table: TableView<'_>,
) -> Result<TabularArrays> {
    let first_row = |column: &str| -> Result<TableArray> {
        if table.extent.row_count == 0 {
            return Err(FitsError::InvalidWcs {
                defect: WcsDefect::Tabular(TabularDefect::EmptyTable),
            });
        }
        let reader = table.column(column)?;
        let column = reader.descriptor();
        let values = if column.tform.descriptor_width().is_some() {
            let mut rows = Ragged::<Vec<f64>>::new(Vec::new(), Vec::new());
            reader.vla_physical_into(&mut rows)?;
            rows.row(0).to_vec()
        } else {
            let mut values = Vec::new();
            reader.physical_into(&mut values)?;
            values.truncate(column.tform.repeat());
            values
        };
        let shape = column
            .tdim()?
            .map_or_else(|| vec![values.len()], <[usize]>::to_vec);
        Ok(TableArray { shape, values })
    };
    Ok(TabularArrays {
        coordinates: first_row(&descriptor.reference.coordinate_column)?,
        indices: descriptor
            .index_columns()
            .map(|column| column.map(first_row).transpose())
            .collect::<Result<_>>()?,
    })
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
    if let Some(tiled) = hdu.tiled_table()? {
        validate_row_range(&rows, tiled.schema.extent.row_count)?;
        let read = TiledRead {
            hdu,
            tiled,
            rows: &rows,
            columns,
        };
        let decoded = data.decoded(read)?;
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

/// The container of a tiled-compressed HDU, which tests and benches read.
#[cfg(all(feature = "internals", feature = "compression"))]
mod container_table {
    use crate::bintable::Table;
    use crate::fits_error::Result;
    use crate::reader::FitsReader;
    use crate::reader::source::Source;

    impl<S: Source> FitsReader<S> {
        /// The `BINTABLE` a tiled-compressed HDU is stored as, its cells still compressed.
        pub(crate) fn read_container_table(&mut self, index: usize) -> Result<Table> {
            let hdu = &self.hdus[index];
            Table::new(hdu.container_schema()?.clone(), self.data.data_unit(hdu)?)
        }
    }
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
