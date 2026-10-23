//! [`ImageReader`]: one image HDU of an open file, read a region at a time.

#[cfg(feature = "compression")]
use std::mem;
#[cfg(feature = "compression")]
use std::ops::Range;

#[cfg(feature = "compression")]
use crate::bintable::table_view::TableView;
use crate::bitpix::Bitpix;
#[cfg(feature = "compression")]
use crate::compress::decode::tiled_image::{TileSection, TiledImage};
use crate::error::Result;
#[cfg(feature = "compression")]
use crate::hdu::HduKind;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::image_model::data::Data;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::region::Region;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Image, Metadata, extend_physical_be, swap_into_words, view_words};
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::DataSource;
use crate::reader::hdu::Hdu;
use crate::reader::source::Source;
use crate::reader::{ChecksumReport, DataLengths, Scratch};

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

/// The stored big-endian bytes a plain-image read covers, and the shape they hold.
#[derive(Debug)]
struct Stored<'a> {
    bytes: &'a [u8],
    shape: &'a [usize],
}

impl<'r, S: Source> ImageReader<'r, S> {
    /// The image `hdu` holds. Errors with [`FitsError::NotAnImage`] for any other kind,
    /// a tiled-compressed image among them without the `compression` feature, and with
    /// the header's own error when its image keywords do not resolve.
    ///
    /// [`FitsError::NotAnImage`]: crate::FitsError::NotAnImage
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
    pub const fn with_checksum(mut self) -> Self {
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
            let header = &self.hdu.header;
            return match region {
                Region::All => {
                    let table = self.data.table(self.hdu, self.sum.as_mut())?;
                    Image::new_scaled(&*self.image.shape, tiled.decode(header, table)?, scaling)
                }
                Region::Section(ranges) => {
                    let tiles = tiles(self.data, self.hdu, tiled, self.scratch, ranges)?;
                    let section = tiles.section(ranges);
                    let samples = tiled.decode_section(header, tiles.table, section)?;
                    Image::new_scaled(tiles.shape, samples, scaling)
                }
            };
        }
        let bitpix = self.image.bitpix;
        let stored = self.plain(region)?;
        Image::new_scaled(stored.shape, Data::decode(stored.bytes, bitpix), scaling)
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
        let (bitpix, scaling) = (self.image.bitpix, self.image.scaling);
        let stored = self.plain(region)?;
        extend_physical_be(stored.bytes, bitpix, &scaling, out);
        Ok(())
    }

    /// Judge the HDU's `DATASUM` and `CHECKSUM`: the sum [`with_checksum`](Self::with_checksum)
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
            let header = &self.hdu.header;
            return Ok(match region {
                Region::All => {
                    let table = self.data.table(self.hdu, self.sum.as_mut())?;
                    Borrowed {
                        shape: &self.image.shape,
                        scaling,
                        samples: tiled.decode_into_words(header, table, words)?,
                    }
                }
                Region::Section(ranges) => {
                    let tiles = tiles(self.data, self.hdu, tiled, self.scratch, ranges)?;
                    let section = tiles.section(ranges);
                    let samples =
                        tiled.decode_section_into_words(header, tiles.table, section, words)?;
                    Borrowed {
                        shape: tiles.shape,
                        scaling,
                        samples,
                    }
                }
            });
        }
        let bitpix = self.image.bitpix;
        let stored = self.plain(region)?;
        let samples = if bitpix == Bitpix::U8 {
            View::U8(stored.bytes)
        } else {
            swap_into_words(stored.bytes, bitpix, words);
            view_words(words, bitpix, stored.bytes.len())
        };
        Ok(Borrowed {
            shape: stored.shape,
            scaling,
            samples,
        })
    }

    /// The stored bytes of `region` of a plain image, each fed to the checksum when one
    /// is summed: the whole data unit viewed in place over the source (or its staging
    /// buffer), or a section gathered into the reader's scratch, reading only its runs.
    fn plain(&mut self, region: Region<'_>) -> Result<Stored<'_>> {
        match region {
            Region::All => {
                let lengths = DataLengths::new(self.hdu.data_bytes)?;
                // With PCOUNT = 0 and GCOUNT = 1, `data_extent` sized the unit as
                // `elem · Π(axes)` — an invariant between the scan and the geometry, not a
                // runtime failure mode.
                debug_assert!(self.image.byte_len().is_ok_and(|len| len == lengths.data));
                let unit = self.data.slice(self.hdu.data_offset, lengths.padded)?;
                if let Some(sum) = &mut self.sum {
                    sum.feed(self.hdu.data_offset, unit);
                }
                Ok(Stored {
                    bytes: &unit[..lengths.data],
                    shape: &self.image.shape,
                })
            }
            Region::Section(ranges) => {
                let Scratch {
                    section_shape,
                    section_bytes,
                    ..
                } = &mut *self.scratch;
                self.data.plain_section(
                    self.hdu,
                    self.image,
                    ranges,
                    section_shape,
                    section_bytes,
                    self.sum.as_mut(),
                )?;
                Ok(Stored {
                    bytes: section_bytes,
                    shape: section_shape,
                })
            }
        }
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
        selection,
        ..
    } = scratch;
    tiled.select(ranges, section_shape, tile_rows)?;
    let rows: &'a [usize] = tile_rows;
    Ok(Tiles {
        shape: section_shape,
        rows,
        table: data.table_rows(hdu, rows, selection)?,
    })
}
