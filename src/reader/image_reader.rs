//! [`ImageReader`]: one image HDU of an open file, read a region at a time.

#[cfg(feature = "compression")]
use std::mem;
#[cfg(feature = "compression")]
use std::ops::Range;

use crate::allocation;
#[cfg(feature = "compression")]
use crate::bintable::selection::Fetch;
#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
use crate::bitpix::Bitpix;
use crate::checksum::checksum_report::ChecksumReport;
#[cfg(feature = "compression")]
use crate::compression::decode::decode_capacities::DecodeCapacities;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_index::TileIndex;
#[cfg(feature = "compression")]
use crate::compression::decode::tile_section::TileSection;
#[cfg(feature = "compression")]
use crate::compression::decode::tiled_image::TiledImage;
use crate::fits_error::{FitsError, Result};
#[cfg(feature = "compression")]
use crate::hdu::HduKind;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::region::{ByteRun, Region, SectionRuns};
use crate::image_model::sample::Sample;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Image, Metadata};
use crate::image_model::{check_image_region, shape_product, validate_image_region};
use crate::reader::data_checksum::{DataChecksum, FINISH_CHUNK};
#[cfg(feature = "compression")]
use crate::reader::data_source;
use crate::reader::data_source::DataSource;
use crate::reader::data_source::offset_at;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;
use crate::reader::{Scratch, data_len};
use crate::words;

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
    data: &'r mut DataSource<S>,
    scratch: &'r mut Scratch,
    #[cfg(feature = "compression")]
    tiled: Option<TiledImage<'r>>,
    sum: Option<DataChecksum>,
}

/// The tiles a section of a compressed image covers: the shape the section selects,
/// the container rows of its tiles, and their compacted table; with the image's tile
/// index.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct Tiles<'a> {
    shape: &'a [usize],
    rows: &'a [usize],
    table: TableView<'a>,
    index: &'a TileIndex,
}

/// Where the stored bytes of a region of a plain image lie in its data unit: one run for
/// the whole image, or the runs of a section.
#[derive(Debug, Clone, Copy)]
pub(crate) enum StoredRuns<'a> {
    All { len: usize, shape: &'a [usize] },
    Section { runs: SectionRuns<'a>, len: usize },
}

/// A plain image and the length of its data unit.
#[derive(Debug, Clone, Copy)]
struct ImageUnit<'a> {
    image: &'a ImageGeometry,
    data_bytes: u64,
}

impl<'a> StoredRuns<'a> {
    /// The runs of the stored bytes of `region` of the plain image of `unit`; a section's
    /// shape is written to `section_shape`.
    fn new(
        unit: ImageUnit<'a>,
        region: Region<'a>,
        section_shape: &'a mut Vec<usize>,
    ) -> Result<Self> {
        let ImageUnit { image, data_bytes } = unit;
        let element_size = image.bitpix.elem_size();
        match region {
            Region::All => {
                let len = data_len(data_bytes)?;
                // With PCOUNT = 0 and GCOUNT = 1, `data_extent` sized the unit as
                // `elem · Π(axes)` — an invariant between the scan and the geometry, not a
                // runtime failure mode.
                debug_assert!(image.byte_len().is_ok_and(|image| image == len));
                Ok(Self::All {
                    len,
                    shape: &image.shape,
                })
            }
            Region::Section(ranges) => {
                validate_image_region(ranges, &image.shape, section_shape)?;
                let len = shape_product(section_shape)?
                    .checked_mul(element_size)
                    .ok_or(FitsError::DataUnitOverflow)?;
                Ok(Self::Section {
                    runs: SectionRuns {
                        shape: &image.shape,
                        ranges,
                        selected_shape: section_shape,
                        element_size,
                    },
                    len,
                })
            }
        }
    }

    /// The stored bytes of the region.
    const fn len(&self) -> usize {
        match self {
            Self::All { len, .. } | Self::Section { len, .. } => *len,
        }
    }

    /// The shape of the region.
    const fn shape(&self) -> &'a [usize] {
        match self {
            Self::All { shape, .. } => shape,
            Self::Section { runs, .. } => runs.selected_shape,
        }
    }

    /// Give `visit` each run, in order.
    fn visit(self, mut visit: impl FnMut(ByteRun) -> Result<()>) -> Result<()> {
        match self {
            Self::All { len, .. } => visit(ByteRun { offset: 0, len }),
            Self::Section { runs, .. } => runs.visit(visit),
        }
    }
}

/// A plain image's data unit read by runs, each fed to the checksum, when one is summed,
/// as it is read.
#[derive(Debug)]
pub(crate) struct PlainRead<'a, S> {
    pub(crate) data: &'a mut DataSource<S>,
    pub(crate) data_offset: u64,
    pub(crate) sum: Option<&'a mut DataChecksum>,
}

impl<S: Source> PlainRead<'_, S> {
    /// Fill `out` with the samples of `runs`, host-endian: decoded from the borrowed
    /// bytes of a resident source, or read into `out` and swapped in place, with no
    /// staging copy.
    fn fill<T: Sample>(&mut self, runs: StoredRuns<'_>, out: &mut [T]) -> Result<()> {
        let size = size_of::<T>();
        debug_assert_eq!(size_of_val(out), runs.len(), "a sample for each stored one");
        let resident = self.data.resident(0, 0).is_some();
        let mut at = 0;
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let samples = &mut out[at..at + run.len / size];
            at += samples.len();
            if resident {
                let bytes = self
                    .data
                    .resident(offset, run.len)
                    .expect("a resident source")?;
                if let Some(sum) = self.sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
                T::decode_be(bytes, samples);
            } else {
                let bytes = words::bytes_mut(samples);
                self.data.read_into(offset, bytes)?;
                if let Some(sum) = self.sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
            }
            Ok(())
        })?;
        if !resident {
            T::swap_be_in_place(out);
        }
        Ok(())
    }

    /// The samples of `runs`, of type `bitpix`, owned.
    pub(crate) fn data(&mut self, runs: StoredRuns<'_>, bitpix: Bitpix) -> Result<Data> {
        let count = runs.len() / bitpix.elem_size();
        Ok(match bitpix {
            Bitpix::U8 => Data::U8(self.filled(runs, count)?),
            Bitpix::I16 => Data::I16(self.filled(runs, count)?),
            Bitpix::I32 => Data::I32(self.filled(runs, count)?),
            Bitpix::I64 => Data::I64(self.filled(runs, count)?),
            Bitpix::F32 => Data::F32(self.filled(runs, count)?),
            Bitpix::F64 => Data::F64(self.filled(runs, count)?),
        })
    }

    /// The samples of `runs`, owned: decoded from the borrowed bytes of a resident source
    /// with no fill first, or read into a zeroed buffer and swapped in place.
    fn filled<T: Sample>(&mut self, runs: StoredRuns<'_>, count: usize) -> Result<Vec<T>> {
        if self.data.resident(0, 0).is_none() {
            let mut values = allocation::try_zeroed(T::default(), count)?;
            self.fill(runs, &mut values)?;
            return Ok(values);
        }
        let mut values = Vec::new();
        allocation::try_reserve(&mut values, count)?;
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let bytes = self
                .data
                .resident(offset, run.len)
                .expect("a resident source")?;
            if let Some(sum) = self.sum.as_deref_mut() {
                sum.feed(offset, bytes);
            }
            T::extend_from_be(bytes, &mut values);
            Ok(())
        })?;
        Ok(values)
    }

    /// The samples of `runs`, of type `bitpix`, in `words`, viewed.
    fn words<'w>(
        &mut self,
        runs: StoredRuns<'_>,
        bitpix: Bitpix,
        words: &'w mut Vec<u64>,
    ) -> Result<View<'w>> {
        let len = runs.len();
        let count = len / bitpix.elem_size();
        words.clear();
        allocation::try_resize(words, len.div_ceil(8), 0)?;
        match bitpix {
            Bitpix::U8 => self.fill(runs, words::samples_mut::<u8>(words, count))?,
            Bitpix::I16 => self.fill(runs, words::samples_mut::<i16>(words, count))?,
            Bitpix::I32 => self.fill(runs, words::samples_mut::<i32>(words, count))?,
            Bitpix::I64 => self.fill(runs, words::samples_mut::<i64>(words, count))?,
            Bitpix::F32 => self.fill(runs, words::samples_mut::<f32>(words, count))?,
            Bitpix::F64 => self.fill(runs, words::samples_mut::<f64>(words, count))?,
        }
        Ok(bitpix.view_words(words, len))
    }

    /// Append the physical values of `runs`, of type `bitpix`, to `out`, from the
    /// borrowed bytes of a resident source or a bounded chunk at a time.
    fn physical<O: PhysicalSample>(
        &mut self,
        runs: StoredRuns<'_>,
        image: &ImageGeometry,
        out: &mut Vec<O>,
    ) -> Result<()> {
        out.reserve(runs.len() / image.bitpix.elem_size());
        runs.visit(|run| {
            let offset = offset_at(self.data_offset, run.offset)?;
            let sum = &mut self.sum;
            self.data.chunks(offset, run.len, |offset, bytes| {
                if let Some(sum) = sum.as_deref_mut() {
                    sum.feed(offset, bytes);
                }
                image.bitpix.extend_physical_be(bytes, &image.scaling, out);
                Ok(())
            })
        })
    }
}

impl<'r, S: Source> ImageReader<'r, S> {
    /// The image `hdu` holds. Errors with [`FitsError::HduKindMismatch`] for any other kind,
    /// a tiled-compressed image among them without the `compression` feature, and with
    /// the header's own error when its image keywords do not resolve.
    ///
    /// [`FitsError::HduKindMismatch`]: crate::FitsError::HduKindMismatch
    pub(crate) fn new(
        hdu: &'r Hdu,
        data: &'r mut DataSource<S>,
        scratch: &'r mut Scratch,
    ) -> Result<Self> {
        #[cfg(feature = "compression")]
        let (image, tiled) = if hdu.kind == HduKind::CompressedImage {
            let image = hdu.image_geometry()?;
            let tiled = TiledImage::new(&hdu.header, image)?;
            scratch
                .tile_indexes
                .load(hdu.data_offset, hdu.container_schema()?, |len, rows| {
                    data.unit(hdu).append(0..len, rows)
                })?;
            (image, Some(tiled))
        } else {
            (hdu.plain_image_geometry()?, None)
        };
        #[cfg(not(feature = "compression"))]
        let image = hdu.plain_image_geometry()?;
        Ok(Self {
            hdu,
            image,
            data,
            scratch,
            #[cfg(feature = "compression")]
            tiled,
            sum: None,
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
        #[cfg(feature = "compression")]
        let sum = {
            let mut sum = DataChecksum::of(self.hdu);
            if self.tiled.is_some() {
                sum.feed(
                    self.hdu.data_offset,
                    self.scratch.tile_indexes.get(self.hdu.data_offset).rows(),
                );
            }
            sum
        };
        #[cfg(not(feature = "compression"))]
        let sum = DataChecksum::of(self.hdu);
        self.sum = Some(sum);
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
        if let Some(tiled) = &self.tiled {
            return Some(tiled.tiles());
        }
        None
    }

    /// The samples of `region`, host-endian, borrowed from `words` — or, for
    /// `BITPIX = 8`, from the source or the reader's scratch, which need no swap. A
    /// loop that passes the same `words` to every read allocates only while it grows;
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
        if let Some(tiled) = &self.tiled {
            return match region {
                Region::All => {
                    let index = self.scratch.tile_indexes.get(self.hdu.data_offset);
                    let table = self.data.tiled_table(self.hdu, index, self.sum.as_mut())?;
                    Ok(Image::of_geometry(
                        self.image.shape.clone(),
                        tiled.decode(table, index)?,
                        scaling,
                    ))
                }
                Region::Section(ranges) => {
                    let Tiles {
                        shape,
                        rows,
                        table,
                        index,
                    } = tiles(
                        self.data,
                        self.hdu,
                        tiled,
                        self.scratch,
                        self.sum.as_mut(),
                        ranges,
                    )?;
                    let section = TileSection {
                        ranges,
                        shape,
                        rows,
                    };
                    let samples = tiled.decode_section(table, index, section)?;
                    Ok(Image::of_geometry(shape.to_vec(), samples, scaling))
                }
            };
        }
        let runs = StoredRuns::new(
            ImageUnit {
                image: self.image,
                data_bytes: self.hdu.data_bytes,
            },
            region,
            &mut self.scratch.section_shape,
        )?;
        let mut plain = PlainRead {
            data: self.data,
            data_offset: self.hdu.data_offset,
            sum: self.sum.as_mut(),
        };
        let samples = plain.data(runs, self.image.bitpix)?;
        Ok(Image::of_geometry(runs.shape().to_vec(), samples, scaling))
    }

    /// Append the physical values of `region` to `out`, as
    /// [`Borrowed::physical_into`] does. A plain image converts straight from its
    /// stored big-endian bytes, in one pass; a compressed one decodes into the reader's
    /// scratch first.
    pub fn physical_into<'g, T: PhysicalSample>(
        &mut self,
        region: impl Into<Region<'g>>,
        out: &mut Vec<T>,
    ) -> Result<()> {
        let region = region.into();
        #[cfg(feature = "compression")]
        if self.tiled.is_some() {
            let mut words = mem::take(&mut self.scratch.words);
            let converted = self
                .view_region(region, &mut words)
                .map(|view| view.physical_into(out));
            self.scratch.words = words;
            return converted;
        }
        let runs = StoredRuns::new(
            ImageUnit {
                image: self.image,
                data_bytes: self.hdu.data_bytes,
            },
            region,
            &mut self.scratch.section_shape,
        )?;
        let mut plain = PlainRead {
            data: self.data,
            data_offset: self.hdu.data_offset,
            sum: self.sum.as_mut(),
        };
        plain.physical(runs, self.image, out)
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
            if let Some(tiled) = &self.tiled {
                held.add_tiled(self, tiled, region)?;
                continue;
            }
            if let Region::Section(ranges) = region {
                check_image_region(ranges, &self.image.shape)?;
            }
        }
        let shape = held.section_shape.bytes::<usize>();
        #[cfg(feature = "compression")]
        if let Some(tiled) = &self.tiled {
            return Ok(self.tiled_bytes(tiled, &held, shape));
        }
        Ok(ViewBytes {
            kept: shape.kept as u64,
            handle: shape.moving as u64,
        })
    }

    /// What the compressed image `tiled` holds for a run of views that needs `held`, its
    /// section shape `shape`. The reader keeps the tile index and what the kept buffers
    /// add. The handle holds the tile grid and, while a read runs, the larger of the
    /// largest decode and the largest old block a kept buffer holds while it grows: a read
    /// fetches its cells before it decodes them, and the decode of the read before is gone,
    /// so the two never meet.
    #[cfg(feature = "compression")]
    fn tiled_bytes(
        &self,
        tiled: &TiledImage<'_>,
        held: &HeldBytes,
        shape: GrowthBytes,
    ) -> ViewBytes {
        let buffers = [
            shape,
            held.tile_rows.bytes::<usize>(),
            held.coordinates.bytes::<usize>(),
            held.compacted.bytes::<u8>(),
            held.unit.bytes::<u8>(),
        ];
        let kept = self.scratch.tile_indexes.heap_bytes()
            + buffers.iter().map(|buffer| buffer.kept).sum::<usize>();
        let moving = buffers
            .iter()
            .map(|buffer| buffer.moving)
            .fold(held.decode, usize::max);
        ViewBytes {
            kept: kept as u64,
            handle: (tiled.heap_bytes() + moving) as u64,
        }
    }

    /// The most heap [`finish`](Self::finish) adds to the reader, a bound: a stream's
    /// staging buffer grown to one chunk of what the reads did not sum, at most 1 MiB and
    /// at most the padded unit, counted whole when it grows past what it holds now, as a
    /// buffer that grows holds its old block while it moves; nothing for a resident
    /// source, which the finish borrows.
    pub fn finish_bytes(&self) -> u64 {
        if self.data.resident(0, 0).is_some() {
            return 0;
        }
        let chunk = usize::try_from(self.hdu.padded_data_bytes())
            .unwrap_or(usize::MAX)
            .min(FINISH_CHUNK);
        let mut staging = Growth::from(self.data.staging_capacity());
        staging.need(chunk);
        let growth = staging.bytes::<u8>();
        (growth.kept + growth.moving) as u64
    }

    /// Judge the HDU's `DATASUM` and `CHECKSUM`: the sum [`with_checksums`](Self::with_checksums)
    /// began, completed from whatever the reads did not feed, or, without it, the whole
    /// data unit read now — the report of
    /// [`FitsReader::verify_checksum`](crate::FitsReader::verify_checksum).
    pub fn finish(self) -> Result<ChecksumReport> {
        let sum = self.sum.unwrap_or_else(|| DataChecksum::of(self.hdu));
        sum.finish(self.hdu, self.data)
    }

    fn view_region<'a>(
        &'a mut self,
        region: Region<'_>,
        words: &'a mut Vec<u64>,
    ) -> Result<Borrowed<'a>> {
        let scaling = self.image.scaling;
        #[cfg(feature = "compression")]
        if let Some(tiled) = &self.tiled {
            return Ok(match region {
                Region::All => {
                    let index = self.scratch.tile_indexes.get(self.hdu.data_offset);
                    let table = self.data.tiled_table(self.hdu, index, self.sum.as_mut())?;
                    Borrowed {
                        shape: &self.image.shape,
                        scaling,
                        samples: tiled.decode_into_words(table, index, words)?,
                    }
                }
                Region::Section(ranges) => {
                    let Tiles {
                        shape,
                        rows,
                        table,
                        index,
                    } = tiles(
                        self.data,
                        self.hdu,
                        tiled,
                        self.scratch,
                        self.sum.as_mut(),
                        ranges,
                    )?;
                    let section = TileSection {
                        ranges,
                        shape,
                        rows,
                    };
                    let samples = tiled.decode_section_into_words(table, index, section, words)?;
                    Borrowed {
                        shape,
                        scaling,
                        samples,
                    }
                }
            });
        }
        let bitpix = self.image.bitpix;
        let runs = StoredRuns::new(
            ImageUnit {
                image: self.image,
                data_bytes: self.hdu.data_bytes,
            },
            region,
            &mut self.scratch.section_shape,
        )?;
        // A whole `BITPIX = 8` image of a resident source needs no swap: it is viewed in
        // place.
        let in_place = matches!((region, bitpix), (Region::All, Bitpix::U8))
            && self.data.resident(0, 0).is_some();
        if in_place {
            let bytes = self
                .data
                .resident(self.hdu.data_offset, runs.len())
                .expect("a resident source")?;
            if let Some(sum) = &mut self.sum {
                sum.feed(self.hdu.data_offset, bytes);
            }
            return Ok(Borrowed {
                shape: &self.image.shape,
                scaling,
                samples: View::U8(bytes),
            });
        }
        let mut plain = PlainRead {
            data: self.data,
            data_offset: self.hdu.data_offset,
            sum: self.sum.as_mut(),
        };
        let samples = plain.words(runs, bitpix, words)?;
        let shape = match region {
            Region::All => &self.image.shape,
            Region::Section(_) => &self.scratch.section_shape,
        };
        Ok(Borrowed {
            shape,
            scaling,
            samples,
        })
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
        Self {
            section_shape: Growth::from(reader.scratch.section_shape.capacity()),
            #[cfg(feature = "compression")]
            tile_rows: Growth::from(reader.scratch.tile_rows.capacity()),
            #[cfg(feature = "compression")]
            coordinates: Growth::from(reader.scratch.tile_coordinates.capacity()),
            #[cfg(feature = "compression")]
            compacted: Growth::from(reader.data.rows_capacity()),
            #[cfg(feature = "compression")]
            unit: Growth::from(reader.data.staging_capacity()),
            #[cfg(feature = "compression")]
            decode: 0,
        }
    }
}

#[cfg(feature = "compression")]
impl HeldBytes {
    /// Grow to what `reader` needs viewing `region` of its compressed image `tiled`.
    fn add_tiled<S: Source>(
        &mut self,
        reader: &ImageReader<'_, S>,
        tiled: &TiledImage<'_>,
        region: Region<'_>,
    ) -> Result<()> {
        let schema = reader.hdu.container_schema()?;
        let index = reader.scratch.tile_indexes.get(reader.hdu.data_offset);
        let count = match region {
            Region::All => {
                if reader.data.resident(0, 0).is_none() {
                    self.unit.need(data_len(reader.hdu.data_bytes)?);
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

/// The tiles of `tiled` that `ranges` covers, their heap cells read into the reader's
/// scratch, and fed to `sum` where they continue what it summed: the section's shape,
/// its tiles' rows, and their compacted table.
#[cfg(feature = "compression")]
fn tiles<'a, S: Source>(
    data: &'a mut DataSource<S>,
    hdu: &'a Hdu,
    tiled: &TiledImage<'_>,
    scratch: &'a mut Scratch,
    sum: Option<&mut DataChecksum>,
    ranges: &[Range<usize>],
) -> Result<Tiles<'a>> {
    let Scratch {
        section_shape,
        tile_rows,
        tile_coordinates,
        tile_indexes,
        ..
    } = scratch;
    tiled.select(ranges, section_shape, tile_rows, tile_coordinates)?;
    let rows: &'a [usize] = tile_rows;
    let index = tile_indexes.get(hdu.data_offset);
    Ok(Tiles {
        shape: section_shape,
        rows,
        table: data.tile_rows(hdu, index.rows(), rows, sum)?,
        index,
    })
}
