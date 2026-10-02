//! A tiled-compressed image ready to decode.

use std::ops::Range;

use crate::allocation;
use crate::compress::ImageCodec;
use crate::compress::convert;
use crate::compress::decode::decode_buffer::DecodeBuffer;
use crate::data::image_data::ImageData;
use crate::data::image_view::ImageView;
use crate::data::shape_product;
use crate::data::validate_image_region;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::table_view::TableView;

/// The image a tiled-compression `BINTABLE` encodes, with the codec and tile shape
/// its header names — everything a decode needs from the header before the first
/// tile, resolved once.
#[derive(Debug)]
pub(crate) struct TiledImage<'a> {
    pub(super) image: &'a ImageGeometry,
    pub(super) codec: ImageCodec,
    /// The `ZTILEn` tile extents, one per image axis.
    pub(super) tiles: Vec<usize>,
}

/// The part of a tiled image a section decode covers: the requested `ranges`, the
/// `shape` they select, and the table rows of the tiles they intersect, in the order
/// the section's table holds them.
#[derive(Debug, Clone, Copy)]
pub(crate) struct TileSection<'s> {
    pub(crate) ranges: &'s [Range<usize>],
    pub(crate) shape: &'s [usize],
    pub(crate) rows: &'s [usize],
}

impl<'a> TiledImage<'a> {
    pub(crate) fn new(header: &Header, image: &'a ImageGeometry) -> Result<TiledImage<'a>> {
        let codec = ImageCodec::parse(
            header
                .get_text("ZCMPTYPE")?
                .ok_or(FitsError::MissingKeyword { name: "ZCMPTYPE" })?,
        )?;
        if codec == ImageCodec::Hcompress1 && image.shape.len() != 2 {
            return Err(FitsError::UnsupportedCompression {
                name: "HCOMPRESS_1 requires a two-dimensional image".to_string(),
            });
        }
        Ok(TiledImage {
            image,
            codec,
            tiles: tile_shape(header, &image.shape)?,
        })
    }

    /// Validate `ranges` against the image, and write the shape they select to `shape`
    /// and the table rows of the tiles they intersect to `rows`, fastest axis first.
    pub(crate) fn select(
        &self,
        ranges: &[Range<usize>],
        shape: &mut Vec<usize>,
        rows: &mut Vec<usize>,
    ) -> Result<()> {
        let dims = &self.image.shape;
        validate_image_region(ranges, dims, shape)?;
        rows.clear();
        if shape.contains(&0) || dims.is_empty() {
            return Ok(());
        }
        let tile_count = ranges
            .iter()
            .zip(&self.tiles)
            .try_fold(1usize, |count, (range, &tile)| {
                count.checked_mul(first_past_tile(range, tile) - range.start / tile)
            })
            .ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_reserve(rows, tile_count)?;
        let mut coordinates: Vec<usize> = ranges
            .iter()
            .zip(&self.tiles)
            .map(|(range, &tile)| range.start / tile)
            .collect();
        for _ in 0..tile_count {
            let mut stride = 1usize;
            let mut index = 0usize;
            for (axis, &coordinate) in coordinates.iter().enumerate() {
                index = coordinate
                    .checked_mul(stride)
                    .and_then(|offset| index.checked_add(offset))
                    .ok_or(FitsError::DataUnitOverflow)?;
                stride = stride
                    .checked_mul(dims[axis].div_ceil(self.tiles[axis]))
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
            rows.push(index);
            for (axis, coordinate) in coordinates.iter_mut().enumerate() {
                *coordinate += 1;
                if *coordinate < first_past_tile(&ranges[axis], self.tiles[axis]) {
                    break;
                }
                *coordinate = ranges[axis].start / self.tiles[axis];
            }
        }
        Ok(())
    }

    /// Decode every tile of the image from its container `table`.
    pub(crate) fn decode(&self, header: &Header, table: TableView<'_>) -> Result<ImageData> {
        let mut samples = convert::zeroed_samples(self.image.bitpix, self.image.len())?;
        if self.image.len() != 0 {
            DecodeBuffer::from_samples(&mut samples).decode_image(header, table, self)?;
        }
        Ok(samples)
    }

    /// [`TiledImage::decode`] into the caller's reused `words`.
    pub(crate) fn decode_into_words<'w>(
        &self,
        header: &Header,
        table: TableView<'_>,
        words: &'w mut Vec<u64>,
    ) -> Result<ImageView<'w>> {
        let nbytes = self.image.byte_len()?;
        allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
        if self.image.len() != 0 {
            DecodeBuffer::from_words(words, self.image.bitpix, self.image.len())
                .decode_image(header, table, self)?;
        }
        Ok(view_words(words, self.image.bitpix, nbytes))
    }

    /// Decode `section` from a `table` that holds just its tiles' rows.
    pub(crate) fn decode_section(
        &self,
        header: &Header,
        table: TableView<'_>,
        section: TileSection<'_>,
    ) -> Result<ImageData> {
        let len = shape_product(section.shape)?;
        let mut samples = convert::zeroed_samples(self.image.bitpix, len)?;
        if len != 0 {
            DecodeBuffer::from_samples(&mut samples)
                .decode_section(header, table, self, section)?;
        }
        Ok(samples)
    }

    /// [`TiledImage::decode_section`] into the caller's reused `words`.
    pub(crate) fn decode_section_into_words<'w>(
        &self,
        header: &Header,
        table: TableView<'_>,
        section: TileSection<'_>,
        words: &'w mut Vec<u64>,
    ) -> Result<ImageView<'w>> {
        let len = shape_product(section.shape)?;
        let nbytes = len
            .checked_mul(self.image.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
        if len != 0 {
            DecodeBuffer::from_words(words, self.image.bitpix, len)
                .decode_section(header, table, self, section)?;
        }
        Ok(view_words(words, self.image.bitpix, nbytes))
    }
}

/// The index of the first tile along an axis past the non-empty `range`.
const fn first_past_tile(range: &Range<usize>, tile: usize) -> usize {
    (range.end - 1) / tile + 1
}

/// The `ZTILEn` tile extents, one per image axis. `ZTILE1` defaults to a whole row
/// and the higher axes to one, so an absent tiling is row-at-a-time.
fn tile_shape(header: &Header, dims: &[usize]) -> Result<Vec<usize>> {
    (1..=dims.len())
        .map(|i| -> Result<usize> {
            let default = if i == 1 { dims[0].max(1) } else { 1 };
            match header.get_integer(key!("ZTILE{i}").as_str())? {
                Some(value) => usize::try_from(value)
                    .ok()
                    .filter(|&value| value > 0)
                    .ok_or(FitsError::KeywordOutOfRange { name: "ZTILEn" }),
                None => Ok(default),
            }
        })
        .collect()
}
