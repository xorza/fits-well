//! [`ImageReader`]: one image HDU of an open file, read a region at a time.

#[cfg(feature = "compression")]
use crate::allocation;
#[cfg(feature = "compression")]
use crate::bintable::selection::Fetch;
#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
use crate::checksum::checksum_report::ChecksumReport;
#[cfg(feature = "compression")]
use crate::compression::decode::decode_buffer::DecodeBuffer;
#[cfg(feature = "compression")]
use crate::compression::decode::decode_capacities::DecodeCapacities;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_index::TileIndex;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_indexes::{LoadedIndex, TileIndexes};
#[cfg(feature = "compression")]
use crate::compression::decode::tile_section::TileSection;
#[cfg(feature = "compression")]
use crate::compression::decode::tiled_image::TiledImage;
#[cfg(feature = "compression")]
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::hdu::Hdu;
#[cfg(feature = "compression")]
use crate::hdu::hdu_kind::HduKind;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::image_model::bitpix::Bitpix;
#[cfg(feature = "compression")]
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::region::Region;
#[cfg(feature = "compression")]
use crate::image_model::shape_product;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Image, Metadata, check_image_region};
use crate::reader::Scratch;
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source;
use crate::reader::data_source::DataSource;
use crate::reader::plain_read::{PlainRead, StoredRuns};
use crate::reader::source::Source;

/// One image HDU of an open file — a plain image or a tiled-compressed one alike —
/// read a [`Region`] at a time, from
/// [`FitsReader::image`](crate::FitsReader::image). The geometry, and a compressed
/// image's codec and tiles, are resolved once, when the handle is made.
///
/// A read borrows the handle until its result is dropped. The handle borrows the
/// reader, but its [`header`](Self::header) lives as long as the reader borrow, so it
/// stays usable beside the reads.
#[derive(Debug)]
pub struct ImageReader<'r, S> {
    hdu: &'r Hdu,
    image: &'r ImageGeometry,
    /// The length of the data unit, bounded by the source when the handle was made.
    unit_len: usize,
    fetcher: Fetcher<'r, S>,
    #[cfg(feature = "compression")]
    compressed: Option<Compressed<'r>>,
}

/// What a read changes: the source and its staging, the reader's reused buffers, and the
/// checksum the reads feed, when one is summed.
#[derive(Debug)]
struct Fetcher<'r, S> {
    data: &'r mut DataSource<S>,
    scratch: &'r mut Scratch,
    sum: Option<DataChecksum>,
}

/// A compressed image's codec and tiles, and the tile index the reader holds for it.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct Compressed<'r> {
    tiled: TiledImage<'r>,
    index: &'r TileIndex,
    /// The heap of every tile index the reader holds, this one's among them.
    indexes_bytes: usize,
}

/// The stored runs of a region of a plain image, and the read that fetches them.
#[derive(Debug)]
struct Plain<'a, S> {
    runs: StoredRuns<'a>,
    read: PlainRead<'a, S>,
}

/// The container rows of the tiles a region of a compressed image covers, fetched, and the
/// section they hold; `None` for the whole image.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct Fetched<'a> {
    table: TableView<'a>,
    section: Option<TileSection<'a>>,
}

impl<'r, S: Source> ImageReader<'r, S> {
    /// The image `hdu` holds. Errors with [`FitsError::HduKindMismatch`] for any other kind,
    /// a tiled-compressed image among them without the `compression` feature, and with
    /// the header's own error when its image keywords do not resolve.
    ///
    /// [`FitsError::HduKindMismatch`]: crate::FitsError::HduKindMismatch
    pub(super) fn new(
        hdu: &'r Hdu,
        data: &'r mut DataSource<S>,
        scratch: &'r mut Scratch,
        #[cfg(feature = "compression")] tile_indexes: &'r mut TileIndexes,
    ) -> Result<Self> {
        let unit_len = data.unit_len(hdu)?;
        #[cfg(feature = "compression")]
        let (image, compressed) = if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            let LoadedIndex {
                index,
                indexes_bytes,
            } = tile_indexes.load(hdu.index, hdu.container_schema()?, |len, rows| {
                data.unit(hdu).append(0..len, rows)
            })?;
            let compressed = Compressed {
                tiled,
                index,
                indexes_bytes,
            };
            (image, Some(compressed))
        } else {
            (hdu.plain_image_geometry()?, None)
        };
        #[cfg(not(feature = "compression"))]
        let image = hdu.plain_image_geometry()?;
        Ok(Self {
            hdu,
            image,
            unit_len,
            fetcher: Fetcher {
                data,
                scratch,
                sum: None,
            },
            #[cfg(feature = "compression")]
            compressed,
        })
    }

    /// The HDU's header, for as long as the reader is borrowed.
    pub const fn header(&self) -> &'r Header {
        &self.hdu.header
    }

    /// The shape, stored element type and scaling of the whole image — for a
    /// tiled-compressed image, those of the image it encodes.
    pub const fn metadata(&self) -> Metadata<'r> {
        self.image.metadata()
    }

    /// Sum the data unit's checksum from the bytes the reads fetch, for
    /// [`finish`](Self::finish) to complete. Each read's stored bytes are summed as
    /// they are read, as long as they reach past what is summed: a plain image's
    /// sections and whole reads; a compressed image's tile index, which the reader holds,
    /// its whole reads, and from a stream the cells of its sections in the order the heap
    /// stores them. The finish reads what was not, and sums a resident source in place.
    #[must_use]
    #[cfg_attr(
        not(feature = "compression"),
        expect(
            clippy::missing_const_for_fn,
            reason = "a compressed image's tile index feeds the sum, which no const fn can"
        )
    )]
    pub fn with_checksums(mut self) -> Self {
        #[cfg_attr(
            not(feature = "compression"),
            expect(
                unused_mut,
                reason = "only a compressed image's tile index feeds the sum here"
            )
        )]
        let mut sum = DataChecksum::of(self.hdu);
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            sum.feed(self.hdu.data_offset, compressed.index.rows());
        }
        self.fetcher.sum = Some(sum);
        self
    }

    /// The `ZTILEn` tile extents of a compressed image, an extent an axis; `None` for a
    /// plain image.
    #[cfg_attr(
        not(feature = "compression"),
        expect(
            clippy::missing_const_for_fn,
            reason = "a compressed image's tiles are a slice of its grid, which no const fn lends"
        )
    )]
    pub fn tiles(&self) -> Option<&[usize]> {
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            return Some(compressed.tiled.tiles());
        }
        None
    }

    /// The samples of `region`, host-endian, in `words` — or, for a whole `BITPIX = 8`
    /// image of a resident source, borrowed from the source's bytes, which need no swap.
    /// A loop that passes the same `words` to every read allocates only while it grows;
    /// `words` is `u64` so the samples stay aligned for every type.
    pub fn view<'a, 'g>(
        &'a mut self,
        region: impl Into<Region<'g>>,
        words: &'a mut Vec<u64>,
    ) -> Result<Borrowed<'a>> {
        self.view_region(region.into(), words)
    }

    /// The samples of `region`, copied into an owned [`Image`].
    pub fn read<'g>(&mut self, region: impl Into<Region<'g>>) -> Result<Image> {
        let region = region.into();
        let scaling = self.image.scaling;
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            let Fetched { table, section } = self.fetcher.fetch(self.hdu, compressed, region)?;
            let shape = section
                .as_ref()
                .map_or(&*self.image.shape, |section| section.shape);
            let mut samples = Data::zeroed(self.image.bitpix, shape_product(shape)?)?;
            let out = DecodeBuffer::from_samples(&mut samples);
            compressed
                .tiled
                .decode_into(table, compressed.index, section, out)?;
            return Ok(Image::of_geometry(shape.to_vec(), samples, scaling));
        }
        let bitpix = self.image.bitpix;
        let Plain { runs, mut read } = self.plain(region)?;
        let samples = read.data(runs, bitpix)?;
        Ok(Image::of_geometry(runs.shape().to_vec(), samples, scaling))
    }

    /// Append the physical values of `region` to `out`, as
    /// [`Borrowed::physical_into`] does. A plain image converts straight from its
    /// stored big-endian bytes, in one pass; a compressed one converts each tile as it
    /// decodes it.
    pub fn physical_into<'g, T: PhysicalSample>(
        &mut self,
        region: impl Into<Region<'g>>,
        out: &mut Vec<T>,
    ) -> Result<()> {
        let region = region.into();
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            let Fetched { table, section } = self.fetcher.fetch(self.hdu, compressed, region)?;
            return compressed
                .tiled
                .physical_into(table, compressed.index, section, out);
        }
        let image = self.image;
        let Plain { runs, mut read } = self.plain(region)?;
        read.physical(runs, image, out)
    }

    /// The most heap the reader and this handle hold viewing each of `regions` in turn
    /// with [`view`](Self::view), beside the samples, which land in the caller's words:
    /// a bound, never below the peak, which it equals on a pool of one worker when the
    /// reader opened no other compressed image.
    ///
    /// The reader keeps the tile index of each compressed image it opened, this one's
    /// read when the handle was made, and what its buffers grow to — a section's shape
    /// and tile rows, the rows and heap cells of the tiles a section touches, a stream's
    /// copy of the whole data unit — each to the largest read's, counted whole where it
    /// must grow past what it holds now, as a buffer that grows holds its old block while
    /// it moves. The handle holds a
    /// compressed image's tile grid, and while a read runs its decode: a wave of
    /// narrowed tiles and a set of tile buffers for each worker that takes a run of the
    /// tiles. A plain image's view holds no more than its section's shape. The figure
    /// counts the workers of the pool this is called on, and holds when no parallel loop
    /// runs the views: a view in a loop's body starts its decode's loops inside it, and a
    /// worker waiting on them takes other runs on its stack, each with a set.
    pub fn view_bytes<'g>(
        &self,
        regions: impl IntoIterator<Item = Region<'g>>,
    ) -> Result<ViewBytes> {
        let axes = self.image.shape.len();
        let mut held = HeldBytes::new(self);
        for region in regions {
            if matches!(region, Region::Section(_)) {
                held.section_shape.need(axes);
            }
            #[cfg(feature = "compression")]
            if let Some(compressed) = &self.compressed {
                held.add_tiled(self, compressed, region)?;
                continue;
            }
            if let Region::Section(ranges) = region {
                check_image_region(ranges, &self.image.shape)?;
            }
        }
        let shape = held.section_shape.bytes::<usize>();
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            return Ok(compressed.held_bytes(&held, shape));
        }
        Ok(ViewBytes {
            kept: shape.kept as u64,
            handle: shape.moving as u64,
        })
    }

    /// The most heap [`finish`](Self::finish) adds to the reader, a bound: a stream's
    /// staging buffer grown to one chunk of what the reads did not sum, at most 1 MiB and
    /// at most the padded unit, counted whole when it grows past what it holds now, as a
    /// buffer that grows holds its old block while it moves; nothing for a resident
    /// source, which the finish borrows.
    pub fn finish_bytes(&self) -> u64 {
        let data = &self.fetcher.data;
        if data.is_resident() {
            return 0;
        }
        let chunk = usize::try_from(self.hdu.padded_data_bytes())
            .unwrap_or(usize::MAX)
            .min(data_source::CHUNK);
        let mut staging = Growth::from(data.staging_capacity());
        staging.need(chunk);
        let growth = staging.bytes::<u8>();
        (growth.kept + growth.moving) as u64
    }

    /// Judge the HDU's `DATASUM` and `CHECKSUM`: the sum [`with_checksums`](Self::with_checksums)
    /// began, completed from whatever the reads did not feed, or, without it, the whole
    /// data unit read now — the report of
    /// [`FitsReader::verify_checksum`](crate::FitsReader::verify_checksum).
    pub fn finish(self) -> Result<ChecksumReport> {
        let Fetcher { data, sum, .. } = self.fetcher;
        let sum = sum.unwrap_or_else(|| DataChecksum::of(self.hdu));
        sum.finish(self.hdu, data)
    }

    fn view_region<'a>(
        &'a mut self,
        region: Region<'_>,
        words: &'a mut Vec<u64>,
    ) -> Result<Borrowed<'a>> {
        let scaling = self.image.scaling;
        let bitpix = self.image.bitpix;
        let image_shape: &'r [usize] = &self.image.shape;
        #[cfg(feature = "compression")]
        if let Some(compressed) = &self.compressed {
            let Fetched { table, section } = self.fetcher.fetch(self.hdu, compressed, region)?;
            let count = shape_product(
                section
                    .as_ref()
                    .map_or(image_shape, |section| section.shape),
            )?;
            let len = count
                .checked_mul(bitpix.elem_size())
                .ok_or(FitsError::DataUnitOverflow)?;
            allocation::try_resize(words, len.div_ceil(8), 0)?;
            let out = DecodeBuffer::from_words(words, bitpix, count);
            compressed
                .tiled
                .decode_into(table, compressed.index, section, out)?;
            return Ok(Borrowed {
                shape: self.region_shape(region),
                scaling,
                samples: bitpix.view_words(words, len),
            });
        }
        // A whole `BITPIX = 8` image of a resident source needs no swap: it is viewed in
        // place.
        if matches!((region, bitpix), (Region::All, Bitpix::U8)) && self.fetcher.data.is_resident()
        {
            let Fetcher { data, sum, .. } = &mut self.fetcher;
            let bytes = data.resident_range(self.hdu.data_offset, self.unit_len)?;
            if let Some(sum) = sum {
                sum.feed(self.hdu.data_offset, bytes);
            }
            return Ok(Borrowed {
                shape: image_shape,
                scaling,
                samples: View::U8(bytes),
            });
        }
        let Plain { runs, mut read } = self.plain(region)?;
        let samples = read.words(runs, bitpix, words)?;
        Ok(Borrowed {
            shape: self.region_shape(region),
            scaling,
            samples,
        })
    }

    /// The shape of `region` after a read of it: the image's, or the section's the read
    /// wrote to the reader's buffer.
    fn region_shape(&self, region: Region<'_>) -> &[usize] {
        match region {
            Region::All => &self.image.shape,
            Region::Section(_) => &self.fetcher.scratch.section_shape,
        }
    }

    /// The stored runs of `region` of this plain image, and the read that fetches them and
    /// feeds the sum.
    fn plain<'a>(&'a mut self, region: Region<'a>) -> Result<Plain<'a, S>> {
        let Fetcher { data, scratch, sum } = &mut self.fetcher;
        let runs = match region {
            Region::All => {
                // With PCOUNT = 0 and GCOUNT = 1, `HduKind::data_bytes` sized the unit as
                // `elem · Π(axes)` — an invariant between the scan and the geometry, not a
                // runtime failure mode.
                debug_assert!(
                    self.image
                        .byte_len()
                        .is_ok_and(|image| image == self.unit_len)
                );
                StoredRuns::All {
                    len: self.unit_len,
                    shape: &self.image.shape,
                }
            }
            Region::Section(ranges) => {
                StoredRuns::section(self.image, ranges, &mut scratch.section_shape)?
            }
        };
        Ok(Plain {
            runs,
            read: PlainRead {
                data,
                data_offset: self.hdu.data_offset,
                sum: sum.as_mut(),
            },
        })
    }
}

impl<S: Source> Fetcher<'_, S> {
    /// The container rows of the tiles of `compressed`, the image `hdu` holds, that
    /// `region` covers: the whole container, or a section's tiles with their heap cells
    /// compacted into the reader's buffers. The bytes are fed to the sum where they
    /// continue what it summed.
    #[cfg(feature = "compression")]
    fn fetch<'a>(
        &'a mut self,
        hdu: &'a Hdu,
        compressed: &Compressed<'_>,
        region: Region<'a>,
    ) -> Result<Fetched<'a>> {
        let Region::Section(ranges) = region else {
            let table = self
                .data
                .tiled_table(hdu, compressed.index, self.sum.as_mut())?;
            return Ok(Fetched {
                table,
                section: None,
            });
        };
        let Scratch {
            section_shape,
            tile_rows,
            tile_coordinates,
            ..
        } = &mut *self.scratch;
        compressed
            .tiled
            .select(ranges, section_shape, tile_rows, tile_coordinates)?;
        let table =
            self.data
                .tile_rows(hdu, compressed.index.rows(), tile_rows, self.sum.as_mut())?;
        Ok(Fetched {
            table,
            section: Some(TileSection {
                ranges,
                shape: section_shape,
                rows: tile_rows,
            }),
        })
    }
}

#[cfg(feature = "compression")]
impl Compressed<'_> {
    /// What the compressed image holds for a run of views that needs `held`, its
    /// section shape `shape`. The reader keeps the tile index and what the kept buffers
    /// add. The handle holds the tile grid and, while a read runs, the larger of the
    /// largest decode and the largest old block a kept buffer holds while it grows: a read
    /// fetches its cells before it decodes them, and the decode of the read before is gone,
    /// so the two never meet.
    fn held_bytes(&self, held: &HeldBytes, shape: GrowthBytes) -> ViewBytes {
        let buffers = [
            shape,
            held.tile_rows.bytes::<usize>(),
            held.coordinates.bytes::<usize>(),
            held.compacted.bytes::<u8>(),
            held.unit.bytes::<u8>(),
        ];
        let kept = self.indexes_bytes + buffers.iter().map(|buffer| buffer.kept).sum::<usize>();
        let moving = buffers
            .iter()
            .map(|buffer| buffer.moving)
            .fold(held.decode, usize::max);
        ViewBytes {
            kept: kept as u64,
            handle: (self.tiled.heap_bytes() + moving) as u64,
        }
    }
}

/// The most heap a run of views holds, by owner, from [`ImageReader::view_bytes`].
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ViewBytes {
    /// What the reader keeps once the handle is gone: the tile index of each compressed
    /// image it opened, and what its buffers grew to.
    pub kept: u64,
    /// What the handle holds beside, freed with it: a compressed image's tile grid, and
    /// the most one read's decode holds while it runs.
    pub handle: u64,
}

/// The buffers the reader keeps as a run of views grows them — the section shape, and a
/// compressed image's tile rows and their odometer, compacted rows and cells, and a
/// stream's copy of the whole data unit — and the most one view's decode holds.
#[derive(Debug)]
struct HeldBytes {
    section_shape: Growth,
    #[cfg(feature = "compression")]
    tile_rows: Growth,
    #[cfg(feature = "compression")]
    coordinates: Growth,
    #[cfg(feature = "compression")]
    compacted: Growth,
    #[cfg(feature = "compression")]
    unit: Growth,
    #[cfg(feature = "compression")]
    decode: usize,
}

impl HeldBytes {
    /// The buffers of `reader` as they stand, before the views.
    const fn new<S: Source>(reader: &ImageReader<'_, S>) -> Self {
        let fetcher = &reader.fetcher;
        Self {
            section_shape: Growth::from(fetcher.scratch.section_shape.capacity()),
            #[cfg(feature = "compression")]
            tile_rows: Growth::from(fetcher.scratch.tile_rows.capacity()),
            #[cfg(feature = "compression")]
            coordinates: Growth::from(fetcher.scratch.tile_coordinates.capacity()),
            #[cfg(feature = "compression")]
            compacted: Growth::from(fetcher.data.rows_capacity()),
            #[cfg(feature = "compression")]
            unit: Growth::from(fetcher.data.staging_capacity()),
            #[cfg(feature = "compression")]
            decode: 0,
        }
    }
}

#[cfg(feature = "compression")]
impl HeldBytes {
    /// Grow to what `reader` needs viewing `region` of its compressed image `compressed`.
    fn add_tiled<S: Source>(
        &mut self,
        reader: &ImageReader<'_, S>,
        compressed: &Compressed<'_>,
        region: Region<'_>,
    ) -> Result<()> {
        let schema = reader.hdu.container_schema()?;
        let Compressed { tiled, index, .. } = compressed;
        let count = match region {
            Region::All => {
                if !reader.fetcher.data.is_resident() {
                    self.unit.need(reader.unit_len);
                }
                None
            }
            Region::Section(ranges) => {
                let (mut shape, mut rows, mut coordinates) = (Vec::new(), Vec::new(), Vec::new());
                tiled.select(ranges, &mut shape, &mut rows, &mut coordinates)?;
                if !rows.is_empty() {
                    self.tile_rows.need(rows.len());
                    self.coordinates.need(ranges.len());
                    let compacted = data_source::tile_rows_len(reader.hdu, index.rows(), &rows)?;
                    self.compacted.need(compacted);
                }
                Some(rows.len())
            }
        };
        let decode: DecodeCapacities = tiled.decode_capacities(schema, index, count)?;
        self.decode = self.decode.max(decode.heap_bytes());
        Ok(())
    }
}

/// A buffer the reader keeps, as a run of views grows it: what it holds before them, what
/// it holds after, and the block it held before it last grew, which it holds beside the
/// new one while it moves.
#[derive(Debug, Clone, Copy)]
struct Growth {
    start: usize,
    capacity: usize,
    old: usize,
}

/// The bytes a kept buffer adds by the end of a run of views, and the most it holds beside
/// them while it grows.
#[derive(Debug, Clone, Copy, Default)]
struct GrowthBytes {
    kept: usize,
    moving: usize,
}

impl Growth {
    /// A buffer that holds `capacity` elements before the views.
    const fn from(capacity: usize) -> Self {
        Self {
            start: capacity,
            capacity,
            old: 0,
        }
    }

    /// Grow, as a view does, to hold `need` elements.
    const fn need(&mut self, need: usize) {
        if need > self.capacity {
            self.old = self.capacity;
            self.capacity = need;
        }
    }

    /// What the buffer of elements of `T` adds, and holds beside while it grows.
    const fn bytes<T>(&self) -> GrowthBytes {
        GrowthBytes {
            kept: (self.capacity - self.start) * size_of::<T>(),
            moving: self.old * size_of::<T>(),
        }
    }
}
