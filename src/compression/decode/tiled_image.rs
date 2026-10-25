//! A tiled-compressed image ready to decode.

use crate::fits_error::KeywordDefect;
use std::ops::Range;

use crate::allocation;
use crate::bintable::schema::Schema;
use crate::bintable::table_view::TableView;
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
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords};
use crate::image_model::bitpix::Bitpix;
use crate::image_model::physical_sample::PhysicalSample;
use crate::image_model::{shape_product, validate_image_region};

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
        let tiles = tile_shape(header, &image.shape)?;
        let codec = codec.fitted(&image.shape, &tiles)?;
        let geometry = TileGeometry::new(&image.shape, tiles);
        // §10.1.2: the container holds one row a tile, so its `NAXIS2` is the tile count.
        let container_rows = header.require_usize(keywords::NAXIS.indexed(2)?)?;
        if container_rows != geometry.ntiles() {
            return Err(SizeUnit::Rows.mismatch(geometry.ntiles(), container_rows));
        }
        Ok(TiledImage {
            image,
            geometry,
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

    /// The tiles `ranges`, a valid section of the image, intersects.
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

    /// Decode the tiles of `section`, or of the whole image for `None`, from their
    /// container `table`, whose rows `index` holds, into `out`, which holds the section's
    /// samples or the image's.
    pub(crate) fn decode_into(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        section: Option<TileSection<'_>>,
        out: DecodeBuffer<'_>,
    ) -> Result<()> {
        debug_assert_eq!(out.bitpix(), self.image.bitpix);
        if out.is_empty() {
            return Ok(());
        }
        if let Some(section) = &section {
            debug_assert_eq!(table.extent.row_count, section.rows.len());
        }
        out.decode(&self.plan(table, index)?, section)
    }

    /// Append the physical values of `section`, or of the whole image for `None`, to
    /// `out`, each tile converted as it is decoded from its container `table`, whose rows
    /// `index` holds: no plane of stored samples is held.
    pub(crate) fn physical_into<O: PhysicalSample>(
        &self,
        table: TableView<'_>,
        index: &TileIndex,
        section: Option<TileSection<'_>>,
        out: &mut Vec<O>,
    ) -> Result<()> {
        let count = match &section {
            Some(section) => {
                debug_assert_eq!(table.extent.row_count, section.rows.len());
                shape_product(section.shape)?
            }
            None => self.image.len(),
        };
        if count == 0 {
            return Ok(());
        }
        let start = out.len();
        allocation::try_resize(out, start + count, O::from_f64(0.0))?;
        let plane = &mut out[start..];
        let decoded = self.plan(table, index).and_then(|plan| {
            let scaling = self.image.scaling;
            match self.image.bitpix {
                Bitpix::U8 => plan.decode_into(section, plane, &scaling.physical::<u8, O>()),
                Bitpix::I16 => plan.decode_into(section, plane, &scaling.physical::<i16, O>()),
                Bitpix::I32 => plan.decode_into(section, plane, &scaling.physical::<i32, O>()),
                Bitpix::I64 => plan.decode_into(section, plane, &scaling.physical::<i64, O>()),
                Bitpix::F32 => plan.decode_into(section, plane, &scaling.physical::<f32, O>()),
                Bitpix::F64 => plan.decode_into(section, plane, &scaling.physical::<f64, O>()),
            }
        });
        if decoded.is_err() {
            out.truncate(start);
        }
        decoded
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
                    .ok_or(FitsError::InvalidKeywordValue {
                        keyword,
                        defect: KeywordDefect::Range,
                    }),
                None => Ok(default),
            }
        })
        .collect()
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::Table;
    use crate::compression::decode::decode_buffer::DecodeBuffer;
    use crate::compression::decode::tile_index::TileIndex;
    use crate::compression::decode::tiled_image::TiledImage;
    use crate::fits_error::Result;
    use crate::image_model::data::Data;

    impl TiledImage<'_> {
        /// Decode every tile of the image from its container `table`, held in memory.
        pub(crate) fn decode_table(&self, table: &Table) -> Result<Data> {
            let index = TileIndex::of_table(table.schema(), table.view())?;
            let mut samples = Data::zeroed(self.image.bitpix, self.image.len())?;
            let out = DecodeBuffer::from_samples(&mut samples);
            self.decode_into(table.view(), &index, None, out)?;
            Ok(samples)
        }
    }
}
