//! A tiled-compressed image ready to decode.

use std::ops::Range;

use crate::allocation;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
use crate::bitpix::Bitpix;
use crate::compression::convert;
use crate::compression::decode::decode_buffer::DecodeBuffer;
use crate::compression::decode::decode_capacities::DecodeCapacities;
use crate::compression::decode::float_quantization::FloatKeywords;
use crate::compression::decode::image_decode_plan::ImageDecodePlan;
use crate::compression::decode::null_mask::{NullMask, NullValues};
use crate::compression::decode::tile_decoder::TileDecoder;
use crate::compression::decode::tile_index::TileIndex;
use crate::compression::decode::tile_section::TileSection;
use crate::compression::decode::tile_sources::TileSources;
use crate::compression::decode::tile_value::TileValue;
use crate::compression::tile_codec::TileCodec;
use crate::compression::tile_geometry::TileGeometry;
use crate::fits_error::CompressionFeature;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords};
use crate::image_model::data::Data;
use crate::image_model::shape_product;
use crate::image_model::validate_image_region;
use crate::image_model::view::View;

/// The image a tiled-compression `BINTABLE` encodes, with what its header says of the
/// codec, the tiles, the nulls and the quantization — everything a decode needs from the
/// header, resolved once for every decode, so a bad keyword fails here rather than at a
/// read.
#[derive(Debug)]
pub(crate) struct TiledImage<'a> {
    pub(super) image: &'a ImageGeometry,
    /// The image's grid of `ZTILEn` tiles.
    pub(super) geometry: TileGeometry,
    pub(super) decoder: TileDecoder,
    /// `ZMASKCMP`, the codec of the null-pixel mask.
    pub(super) mask_codec: Option<TileCodec>,
    /// The `ZBLANK` keyword: the value every tile stores its nulls as.
    pub(super) zblank: Option<i64>,
    /// `Some` exactly for a float image.
    pub(super) float: Option<FloatKeywords<'a>>,
}

impl<'a> TiledImage<'a> {
    pub(crate) fn new(header: &'a Header, image: &'a ImageGeometry) -> Result<Self> {
        let codec = TileCodec::parse(header.get_text(keywords::ZCMPTYPE)?.ok_or(
            FitsError::MissingKeyword {
                name: RecordName::Keyword(keywords::ZCMPTYPE),
            },
        )?)?;
        if matches!(codec, TileCodec::Hcompress(_)) && image.shape.len() != 2 {
            return Err(FitsError::UnsupportedCompression {
                feature: CompressionFeature::HcompressDimensions,
            });
        }
        Ok(TiledImage {
            image,
            geometry: TileGeometry::new(&image.shape, tile_shape(header, &image.shape)?),
            decoder: TileDecoder::new(header, image, codec)?,
            mask_codec: header
                .get_text(keywords::ZMASKCMP)?
                .map(TileCodec::parse)
                .transpose()?,
            zblank: header.get_integer(keywords::ZBLANK)?,
            float: image
                .bitpix
                .is_float()
                .then(|| FloatKeywords::read(header))
                .transpose()?,
        })
    }

    /// The heap bytes the image's description takes.
    pub(crate) const fn heap_bytes(&self) -> usize {
        self.geometry.heap_bytes()
    }

    /// The `ZTILEn` tile extents, one per image axis.
    pub(crate) fn tiles(&self) -> &[usize] {
        self.geometry.tiles()
    }

    /// The tiles `ranges`, a valid region of the image, intersects.
    pub(crate) fn tile_count(&self, ranges: &[Range<usize>]) -> Result<usize> {
        if ranges.iter().any(Range::is_empty) || ranges.is_empty() {
            return Ok(0);
        }
        ranges
            .iter()
            .zip(self.tiles())
            .try_fold(1usize, |count, (range, &tile)| {
                count.checked_mul(first_past_tile(range, tile) - range.start / tile)
            })
            .ok_or(FitsError::DataUnitOverflow)
    }

    /// Validate `ranges` against the image, and write the shape they select to `shape`
    /// and the table rows of the tiles they intersect to `rows`, fastest axis first,
    /// walking them with `coordinates`. Each buffer grows to what this selection holds,
    /// no room past it.
    pub(crate) fn select(
        &self,
        ranges: &[Range<usize>],
        shape: &mut Vec<usize>,
        rows: &mut Vec<usize>,
        coordinates: &mut Vec<usize>,
    ) -> Result<()> {
        let dims = &self.image.shape;
        let tiles = self.tiles();
        validate_image_region(ranges, dims, shape)?;
        rows.clear();
        let tile_count = self.tile_count(ranges)?;
        if tile_count == 0 {
            return Ok(());
        }
        allocation::try_grow_to(rows, tile_count)?;
        allocation::try_grow_to(coordinates, ranges.len())?;
        coordinates.clear();
        coordinates.extend(
            ranges
                .iter()
                .zip(tiles)
                .map(|(range, &tile)| range.start / tile),
        );
        for _ in 0..tile_count {
            let mut stride = 1usize;
            let mut index = 0usize;
            for (axis, &coordinate) in coordinates.iter().enumerate() {
                index = coordinate
                    .checked_mul(stride)
                    .and_then(|offset| index.checked_add(offset))
                    .ok_or(FitsError::DataUnitOverflow)?;
                stride = stride
                    .checked_mul(dims[axis].div_ceil(tiles[axis]))
                    .ok_or(FitsError::DataUnitOverflow)?;
            }
            rows.push(index);
            for (axis, coordinate) in coordinates.iter_mut().enumerate() {
                *coordinate += 1;
                if *coordinate < first_past_tile(&ranges[axis], tiles[axis]) {
                    break;
                }
                *coordinate = ranges[axis].start / tiles[axis];
            }
        }
        Ok(())
    }

    /// Decode every tile of the image from its container `table`, whose rows `index`
    /// holds.
    pub(crate) fn decode(&self, table: TableView<'_>, index: &TileIndex) -> Result<Data> {
        let mut samples = convert::zeroed_samples(self.image.bitpix, self.image.len())?;
        if self.image.len() != 0 {
            self.decode_all(table, index, DecodeBuffer::from_samples(&mut samples))?;
        }
        Ok(samples)
    }

    /// [`TiledImage::decode`] into the caller's reused `words`.
    pub(crate) fn decode_into_words<'w>(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        words: &'w mut Vec<u64>,
    ) -> Result<View<'w>> {
        let nbytes = self.image.byte_len()?;
        allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
        if self.image.len() != 0 {
            let buffer = DecodeBuffer::from_words(words, self.image.bitpix, self.image.len());
            self.decode_all(table, index, buffer)?;
        }
        Ok(self.image.bitpix.view_words(words, nbytes))
    }

    /// Decode `section` from a `table` that holds just its tiles' rows.
    pub(crate) fn decode_section(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        section: TileSection<'_>,
    ) -> Result<Data> {
        let len = shape_product(section.shape)?;
        let mut samples = convert::zeroed_samples(self.image.bitpix, len)?;
        if len != 0 {
            let buffer = DecodeBuffer::from_samples(&mut samples);
            self.decode_section_into(table, index, section, buffer)?;
        }
        Ok(samples)
    }

    /// [`TiledImage::decode_section`] into the caller's reused `words`.
    pub(crate) fn decode_section_into_words<'w>(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        section: TileSection<'_>,
        words: &'w mut Vec<u64>,
    ) -> Result<View<'w>> {
        let len = shape_product(section.shape)?;
        let nbytes = len
            .checked_mul(self.image.bitpix.elem_size())
            .ok_or(FitsError::DataUnitOverflow)?;
        allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
        if len != 0 {
            let buffer = DecodeBuffer::from_words(words, self.image.bitpix, len);
            self.decode_section_into(table, index, section, buffer)?;
        }
        Ok(self.image.bitpix.view_words(words, nbytes))
    }

    /// The most a decode holds: of `count` tiles a section touches, or of the whole image
    /// for `None`. The container's `index` gives its columns; no cell is read.
    pub(crate) fn decode_capacities(
        &self,
        schema: &Schema,
        index: &TileIndex,
        count: Option<usize>,
    ) -> Result<DecodeCapacities> {
        if self.image.len() == 0 || count == Some(0) {
            return Ok(DecodeCapacities::default());
        }
        let plan = self.plan(index.table(schema), index)?;
        match self.image.bitpix {
            Bitpix::U8 => plan.decode_capacities::<u8>(count),
            Bitpix::I16 => plan.decode_capacities::<i16>(count),
            Bitpix::I32 => plan.decode_capacities::<i32>(count),
            Bitpix::I64 => plan.decode_capacities::<i64>(count),
            Bitpix::F32 => plan.decode_capacities::<f32>(count),
            Bitpix::F64 => plan.decode_capacities::<f64>(count),
        }
    }

    /// What a decode of `table`, whose rows `index` holds, reads beside the header: the
    /// tile sources and the null mask of the table's cells, and the per-tile parameters
    /// of the index.
    fn plan<'p>(
        &'p self,
        table: TableView<'p>,
        index: &'p TileIndex,
    ) -> Result<ImageDecodePlan<'p>> {
        let zblank = TileValue::new(self.zblank, index.zblank());
        let blank = self.image.scaling.blank;
        let (stored_null, quantization) = match &self.float {
            None => (zblank.or(blank), None),
            Some(keywords) => (
                TileValue::Absent,
                Some(keywords.quantization(zblank, index, self.decoder.codec())?),
            ),
        };
        Ok(ImageDecodePlan {
            geometry: &self.geometry,
            decoder: &self.decoder,
            sources: TileSources::read(table)?,
            nulls: NullMask::read(
                table,
                self.mask_codec,
                NullValues {
                    blank,
                    stored: stored_null,
                },
            )?,
            quantization,
        })
    }

    fn decode_all(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        buffer: DecodeBuffer<'_>,
    ) -> Result<()> {
        debug_assert_eq!(buffer.bitpix(), self.image.bitpix);
        buffer.decode_all(&self.plan(table, index)?)
    }

    fn decode_section_into(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        section: TileSection<'_>,
        buffer: DecodeBuffer<'_>,
    ) -> Result<()> {
        debug_assert_eq!(buffer.bitpix(), self.image.bitpix);
        debug_assert_eq!(table.extent.row_count, section.rows.len());
        buffer.decode_section(&self.plan(table, index)?, section)
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
            let keyword = keywords::ZTILE.indexed(i)?;
            match header.get_integer(keyword)? {
                Some(value) => usize::try_from(value)
                    .ok()
                    .filter(|&value| value > 0)
                    .ok_or(FitsError::KeywordOutOfRange { keyword }),
                None => Ok(default),
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::Table;
    use crate::compression::decode::tile_index::TileIndex;
    use crate::compression::decode::tiled_image::TiledImage;
    use crate::fits_error::Result;
    use crate::image_model::data::Data;

    impl TiledImage<'_> {
        /// Decode every tile of the image from its container `table`, held in memory.
        pub(crate) fn decode_table(&self, table: &Table) -> Result<Data> {
            let index = TileIndex::of_table(table.schema(), table.view())?;
            self.decode(table.view(), &index)
        }
    }
}
