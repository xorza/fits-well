//! [`ImageReader`]: one image HDU of an open file, read a region at a time.

#[cfg(feature = "compression")]
use std::mem;
#[cfg(feature = "compression")]
use std::ops::Range;

use crate::allocation;
#[cfg(feature = "compression")]
use crate::bintable::selection::TableRows;
#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
use crate::bitpix::Bitpix;
use crate::checksum::checksum_report::ChecksumReport;
#[cfg(feature = "compression")]
use crate::compress::decode::tile_section::TileSection;
#[cfg(feature = "compression")]
use crate::compress::decode::tiled_image::TiledImage;
use crate::error::{FitsError, Result};
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
use crate::image_model::{shape_product, validate_image_region};
use crate::reader::data_checksum::DataChecksum;
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
/// the container rows of its tiles, and their compacted table.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct Tiles<'a> {
    shape: &'a [usize],
    rows: &'a [usize],
    table: TableView<'a>,
}

#[cfg(feature = "compression")]
impl Tiles<'_> {
    /// Where the section of `ranges` lies among these tiles.
    const fn section<'s>(&'s self, ranges: &'s [Range<usize>]) -> TileSection<'s> {
        TileSection {
            ranges,
            shape: self.shape,
            rows: self.rows,
        }
    }
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
            (image, Some(TiledImage::new(&hdu.header, image)?))
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
    /// they are read, as long as they continue where the last read ended: a plain
    /// image's sections and whole reads, and a compressed image's whole reads. A
    /// compressed section's tiles are not, and the finish reads what was not.
    #[must_use]
    pub const fn with_checksums(mut self) -> Self {
        self.sum = Some(DataChecksum::of(self.hdu));
        self
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
                    let table = self.data.table(self.hdu, self.sum.as_mut())?;
                    Ok(Image::of_geometry(
                        self.image.shape.clone(),
                        tiled.decode(table)?,
                        scaling,
                    ))
                }
                Region::Section(ranges) => {
                    let tiles = tiles(self.data, self.hdu, tiled, self.scratch, ranges)?;
                    let section = tiles.section(ranges);
                    let samples = tiled.decode_section(tiles.table, section)?;
                    Ok(Image::of_geometry(tiles.shape.to_vec(), samples, scaling))
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
                    let table = self.data.table(self.hdu, self.sum.as_mut())?;
                    Borrowed {
                        shape: &self.image.shape,
                        scaling,
                        samples: tiled.decode_into_words(table, words)?,
                    }
                }
                Region::Section(ranges) => {
                    let tiles = tiles(self.data, self.hdu, tiled, self.scratch, ranges)?;
                    let section = tiles.section(ranges);
                    let samples = tiled.decode_section_into_words(tiles.table, section, words)?;
                    Borrowed {
                        shape: tiles.shape,
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

/// The tiles of `tiled` that `ranges` covers, their container rows read into the
/// reader's scratch: the section's shape, its tiles' rows, and their compacted table.
#[cfg(feature = "compression")]
fn tiles<'a, S: Source>(
    data: &'a mut DataSource<S>,
    hdu: &'a Hdu,
    tiled: &TiledImage<'_>,
    scratch: &'a mut Scratch,
    ranges: &[Range<usize>],
) -> Result<Tiles<'a>> {
    let Scratch {
        section_shape,
        tile_rows,
        ..
    } = scratch;
    tiled.select(ranges, section_shape, tile_rows)?;
    let rows: &'a [usize] = tile_rows;
    Ok(Tiles {
        shape: section_shape,
        rows,
        table: data.table_rows(hdu, TableRows::Picked(rows))?,
    })
}
