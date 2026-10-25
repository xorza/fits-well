//! The optional per-tile null-pixel mask (§10.1.2) and how it is applied.

use crate::bintable::table_view::TableView;
use crate::bintable::vla_column::VlaColumn;
use crate::compression::convert;
use crate::compression::decode::ensure_tile_size;
use crate::compression::decode::float_sample::FloatSample;
use crate::compression::decode::tile_scratch_set::TileScratchSet;
use crate::compression::decode::tile_value::TileValue;
use crate::compression::plane::IntBitpix;
use crate::compression::rice;
use crate::compression::tile_codec::{CodecCapacities, CodecScratch, Tile, TileCodec};
use crate::fits_error::CompressedDataDefect;
use crate::fits_error::CompressionFeature;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::header_model::keywords;
use crate::header_model::record_name::RecordName;

/// How an image spells its nulls: the `BLANK` value an integer image's nulls take (a
/// float image's take `NaN`), and the value an integer tile stores its nulls as.
#[derive(Debug)]
pub(super) struct NullValues<'a> {
    pub(super) blank: Option<i64>,
    /// An integer tile's stored null: `ZBLANK` as a keyword, else as a column, else
    /// `BLANK`, as cfitsio resolves it. Absent for a float image, whose quantized nulls
    /// the dequantization maps.
    pub(super) stored: TileValue<'a, i64>,
}

/// An image's nulls and everything applying them needs: the per-tile mask column, the
/// `ZMASKCMP` codec that encodes it, and how the image spells its nulls.
#[derive(Debug)]
pub(super) struct NullMask<'a> {
    column: Option<VlaColumn<'a>>,
    /// `ZMASKCMP`, its RICE samples a byte wide.
    codec: Option<TileCodec>,
    values: NullValues<'a>,
}

impl<'a> NullMask<'a> {
    pub(super) fn read(
        table: TableView<'a>,
        codec: Option<TileCodec>,
        values: NullValues<'a>,
    ) -> Result<Self> {
        let column = first_column(
            table,
            &[
                "NULL_PIXEL_MASK",
                "NULL_PIXEL_MASK_COLUMN",
                "NULL PIXEL MASK",
            ],
        )?;
        if column.is_some() && matches!(codec, Some(TileCodec::Hcompress(_))) {
            return Err(FitsError::UnsupportedCompression {
                feature: CompressionFeature::HcompressNullMask,
            });
        }
        Ok(NullMask {
            column,
            codec: codec.map(|codec| match codec {
                TileCodec::Rice(rice::Codec { blocksize, .. }) => TileCodec::Rice(rice::Codec {
                    blocksize,
                    bytepix: IntBitpix::U8,
                }),
                codec => codec,
            }),
            values,
        })
    }

    /// Give every null of integer tile `tile`, in `scratch`, the image's `BLANK`: an
    /// element the tile stores as its null value, and an element the mask in its
    /// `table_row` marks.
    pub(super) fn apply_integer(
        &self,
        table_row: usize,
        tile: usize,
        scratch: &mut TileScratchSet<i64>,
    ) -> Result<()> {
        let values = &mut scratch.values;
        let NullValues {
            blank: declared,
            stored,
        } = &self.values;
        let blank = || {
            declared.ok_or(FitsError::MissingKeyword {
                name: RecordName::Keyword(keywords::BLANK),
            })
        };
        if let Some(stored) = stored.at(tile)
            && Some(stored) != *declared
            && values.contains(&stored)
        {
            let blank = blank()?;
            for value in values.iter_mut().filter(|value| **value == stored) {
                *value = blank;
            }
        }
        if let Some(mask) = self.decode(table_row, scratch.tile.nelem(), &mut scratch.codecs)? {
            fill_masked(values, mask, blank()?);
        }
        Ok(())
    }

    /// Replace every masked element of a float tile in `scratch` with `NaN`.
    pub(super) fn apply_float<F: FloatSample>(
        &self,
        table_row: usize,
        scratch: &mut TileScratchSet<F>,
    ) -> Result<()> {
        if let Some(mask) = self.decode(table_row, scratch.tile.nelem(), &mut scratch.codecs)? {
            fill_masked(&mut scratch.values, mask, F::NAN);
        }
        Ok(())
    }

    /// What decoding the mask of a tile of `tile_elems` elements fills of a set's codec
    /// buffers: nothing for an image without a mask column.
    pub(super) fn capacities(&self, tile_elems: usize) -> Result<CodecCapacities> {
        let (Some(_), Some(codec)) = (self.column, self.codec) else {
            return Ok(CodecCapacities::default());
        };
        codec.ints_capacities(Tile {
            shape: &[tile_elems],
            width: IntBitpix::U8,
            blank: None,
        })
    }

    /// One tile's mask, lent from `codecs`, or `None` when the tile has none.
    fn decode<'s>(
        &self,
        table_row: usize,
        tile_elems: usize,
        codecs: &'s mut CodecScratch,
    ) -> Result<Option<&'s [i64]>> {
        let Some(column) = self.column else {
            return Ok(None);
        };
        let cell = column.cell(table_row)?;
        if cell.element_count == 0 {
            return Ok(None);
        }
        let codec = self.codec.ok_or(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::ZMASKCMP),
        })?;
        let tile = Tile {
            shape: &[tile_elems],
            width: IntBitpix::U8,
            blank: None,
        };
        let stream = convert::stream_cell(cell, codec.stream_kind())?;
        let mask = codec.decode_ints(tile, stream, codecs)?;
        ensure_tile_size(tile_elems, mask.len())?;
        if mask.iter().any(|&value| !matches!(value, 0 | 1)) {
            return Err(CompressedDataDefect::NullMask.into());
        }
        Ok(Some(mask))
    }
}

/// The first of `names` the table carries as a variable-length column — writers
/// disagree on the mask column's spelling, so all three are accepted.
fn first_column<'a>(table: TableView<'a>, names: &[&str]) -> Result<Option<VlaColumn<'a>>> {
    for &name in names {
        if let Some(column) = table.optional_vla_column(name)? {
            return Ok(Some(column));
        }
    }
    Ok(None)
}

/// Overwrite every element the mask marks (§10.1.2: a mask value of 1 is a null)
/// with `fill` — `BLANK` for an integer plane, `NaN` for a float one.
fn fill_masked<T: Copy>(values: &mut [T], mask: &[i64], fill: T) {
    for (value, &masked) in values.iter_mut().zip(mask) {
        if masked == 1 {
            *value = fill;
        }
    }
}
