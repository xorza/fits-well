//! The per-tile codec dispatch, fixed once for a whole tiled image.

use crate::bintable::element_kind::ElementKind;
use crate::bintable::vla_column::VlaCell;
use crate::compression;
use crate::compression::convert;
use crate::compression::decode::float_quantization::FloatEncoding;
use crate::compression::decode::float_sample::FloatSample;
use crate::compression::decode::tile_scratch_set::TileScratchSet;
use crate::compression::decode::tile_sources::{TileSource, TileSources};
use crate::compression::gzip;
use crate::compression::hcompress;
use crate::compression::hcompress::Smoothing;
use crate::compression::plane::{FloatBitpix, IntBitpix, Plane};
use crate::compression::plio;
use crate::compression::rice;
use crate::compression::tile_codec::{CodecCapacities, CodecScratch, Tile, TileCodec};
use crate::compression::tile_geometry::TileScratch;
use crate::fits_error::FitsError;
use crate::fits_error::KeywordDefect;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use crate::fits_error::{CompressedDataDefect, TileCell};
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::record::RecordView;
use crate::header_model::{Header, keywords};
use std::mem;

/// What a decoded value outside the image's `BITPIX` range means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Overshoot {
    /// A lossy codec's reconstruction may overshoot the range its original values lay
    /// in, so the value clamps to the range, as cfitsio clips it.
    Clamp,
    /// A lossless stream an encoder wrote never leaves the range, so the value is
    /// corrupt data (cfitsio `NUM_OVERFLOW`).
    Refuse,
}

/// The decode parameters constant across all of a tiled image's tiles: the codec,
/// the integer plane the tiles are stored in (and, for a float image, its float
/// `ZBITPIX`), and the codec knobs. Every per-tile entry point hangs off this, so
/// the helpers take one decoder rather than a long parameter list.
#[derive(Debug)]
pub(super) struct TileDecoder {
    /// The `ZCMPTYPE` codec with its `ZNAMEi`/`ZVALi` parameters.
    codec: TileCodec,
    int_bitpix: IntBitpix,
    /// `Some` exactly for a float image.
    float_bitpix: Option<FloatBitpix>,
}

/// `codec` with its knobs from `ZNAMEi`/`ZVALi`, read in one pass: the Rice block size
/// and pixel width, and the HCOMPRESS `SMOOTH` flag, the Table 37 defaults until a
/// keyword sets one. A codec reads only its own knobs, so a value another codec would
/// refuse does not fail the image.
fn read_parameters(header: &Header, mut codec: TileCodec) -> Result<TileCodec> {
    {
        for record in header {
            let RecordView::Valued { keyword, .. } = record else {
                continue;
            };
            let Some(i) = compression::parameter_index(keyword) else {
                continue;
            };
            let Some(name) = header.get_text(keyword)? else {
                continue;
            };
            let zval = keywords::ZVAL.indexed(i)?;
            let value = || header.get_integer(zval);
            let out_of_range = FitsError::InvalidKeywordValue {
                keyword: zval,
                defect: KeywordDefect::Range,
            };
            match (&mut codec, name) {
                (TileCodec::Rice(rice::Codec { blocksize, .. }), "BLOCKSIZE") => {
                    if let Some(v) = value()? {
                        *blocksize = match v {
                            16 => 16,
                            32 => 32,
                            _ => return Err(out_of_range),
                        };
                    }
                }
                (TileCodec::Rice(rice::Codec { bytepix, .. }), "BYTEPIX") => {
                    if let Some(v) = value()? {
                        *bytepix = usize::try_from(v)
                            .ok()
                            .and_then(IntBitpix::from_bytepix)
                            .ok_or(out_of_range)?;
                    }
                }
                (TileCodec::Hcompress(hcompress::Codec { smoothing, .. }), "SMOOTH") => {
                    *smoothing = if value()?.unwrap_or(0) == 0 {
                        Smoothing::Off
                    } else {
                        Smoothing::On
                    };
                }
                _ => {}
            }
        }
    }
    Ok(codec)
}

impl TileDecoder {
    pub(super) fn new(header: &Header, image: &ImageGeometry, codec: TileCodec) -> Result<Self> {
        let codec = match read_parameters(header, codec)? {
            TileCodec::Plio(_) => TileCodec::Plio(plio::Codec {
                offset: compression::plio_offset(image.bitpix, image.scaling),
            }),
            codec => codec,
        };
        // A float image's tiles arrive quantized, so they decode in whichever integer
        // width the codec stored them at — RICE_1 says so in `BYTEPIX`, the rest use
        // 32-bit — and only then dequantize to `ZBITPIX`.
        let (int_bitpix, float_bitpix) = match (Plane::of(image.bitpix), codec) {
            (Plane::Int(bitpix), _) => (bitpix, None),
            (Plane::Float(bitpix), TileCodec::Rice(rice::Codec { bytepix, .. })) => {
                (bytepix, Some(bitpix))
            }
            (Plane::Float(bitpix), _) => (IntBitpix::I32, Some(bitpix)),
        };
        Ok(Self {
            codec,
            int_bitpix,
            float_bitpix,
        })
    }

    /// What decoding a whole tile of `shape` fills of a set's codec buffers, whichever
    /// cell of `sources` it is read from; the floats of a `lossless` float image come
    /// back as bytes, every other image's tiles as integers.
    pub(super) fn capacities(
        &self,
        shape: &[usize],
        sources: &TileSources<'_>,
        lossless: bool,
    ) -> Result<CodecCapacities> {
        let ints = Tile {
            shape,
            width: self.int_bitpix,
            blank: None,
        };
        let floats = self.float_bitpix.map(|bitpix| Tile {
            shape,
            width: bitpix.integer_width(),
            blank: None,
        });
        let mut capacities = CodecCapacities::default();
        if sources.has_primary() {
            capacities = capacities.max(match floats {
                Some(floats) if lossless => self.codec.bytes_capacities(floats)?,
                _ => self.codec.ints_capacities(ints)?,
            });
        }
        if sources.has_gzip_fallback() {
            capacities = capacities.max(match floats {
                Some(floats) => FALLBACK_GZIP.bytes_capacities(floats)?,
                None => FALLBACK_GZIP.ints_capacities(ints)?,
            });
        }
        Ok(capacities)
    }

    /// The codec of the tiles' `COMPRESSED_DATA` cells.
    pub(super) const fn codec(&self) -> TileCodec {
        self.codec
    }

    /// `tile` as the codec sees it: its integer samples' width, and no null value, which
    /// matters only to an encode.
    fn tile<'a>(&self, tile: &'a TileScratch) -> Tile<'a> {
        Tile {
            shape: &tile.tdims,
            width: self.int_bitpix,
            blank: None,
        }
    }

    /// What a decoded value outside the image's range means for a tile from `source`:
    /// a lossy `HCOMPRESS_1` cell may overshoot the range of the image's values, and
    /// a fallback cell, stored losslessly, never does.
    const fn overshoot(&self, source: TileSource<'_>) -> Overshoot {
        match (source, self.codec) {
            (TileSource::Compressed(_), TileCodec::Hcompress(_)) => Overshoot::Clamp,
            _ => Overshoot::Refuse,
        }
    }

    /// Decode one tile of an *integer* image into `scratch`'s values, and say what a
    /// value outside the image's range means for it.
    pub(super) fn decode_tile_into(
        &self,
        source: TileSource<'_>,
        scratch: &mut TileScratchSet<i64>,
    ) -> Result<Overshoot> {
        match source {
            TileSource::Compressed(cell) => {
                let decoded = self.decode_cell(cell, &scratch.tile, &mut scratch.codecs)?;
                mem::swap(&mut scratch.values, decoded);
            }
            TileSource::Gzip(cell) => {
                let decoded = FALLBACK_GZIP.decode_ints(
                    self.tile(&scratch.tile),
                    convert::stream_cell(cell, ElementKind::U8)?,
                    &mut scratch.codecs,
                )?;
                mem::swap(&mut scratch.values, decoded);
            }
            TileSource::Uncompressed(cell) => convert::cell_to_i64_into(cell, &mut scratch.values)?,
        }
        Ok(self.overshoot(source))
    }

    /// Decode one tile of a *float* image into `scratch`'s values, in the image's stored
    /// type `F`. A primary `COMPRESSED_DATA` cell holds quantized integers, dequantized
    /// as `encoding` says, or a lossless image's gzip'd floats; the
    /// `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks hold the floats themselves.
    pub(super) fn decode_float_tile_into<F: FloatSample>(
        &self,
        source: TileSource<'_>,
        encoding: FloatEncoding,
        scratch: &mut TileScratchSet<F>,
    ) -> Result<()> {
        debug_assert_eq!(self.float_bitpix.map(FloatBitpix::elem_size), Some(F::SIZE));
        let TileScratchSet {
            tile,
            values: out,
            codecs,
        } = scratch;
        let tile_bytes = tile.nelem().saturating_mul(F::SIZE);
        let floats = Tile {
            shape: &tile.tdims,
            width: self
                .float_bitpix
                .expect("a float tile of a float image")
                .integer_width(),
            blank: None,
        };
        match source {
            TileSource::Compressed(cell) => match encoding {
                FloatEncoding::Quantized(dequant) => {
                    dequant.apply(self.decode_cell(cell, tile, codecs)?, out);
                }
                FloatEncoding::Lossless(codec) => {
                    F::decode_be(
                        codec.decode_bytes(
                            floats,
                            convert::stream_cell(cell, ElementKind::U8)?,
                            codecs,
                        )?,
                        out,
                    );
                }
            },
            TileSource::Gzip(cell) => {
                F::decode_be(
                    FALLBACK_GZIP.decode_bytes(
                        floats,
                        convert::stream_cell(cell, ElementKind::U8)?,
                        codecs,
                    )?,
                    out,
                );
            }
            TileSource::Uncompressed(cell) => match cell.element_type {
                kind if kind == F::KIND => F::decode_be(cell.bytes, out),
                ElementKind::F32 | ElementKind::F64 => F::decode_other_be(cell.bytes, out),
                // The raw bytes of the floats.
                ElementKind::U8 => {
                    if cell.bytes.len() != tile_bytes {
                        return Err(SizeUnit::Bytes.mismatch(tile_bytes, cell.bytes.len()));
                    }
                    F::decode_be(cell.bytes, out);
                }
                element => {
                    return Err(CompressedDataDefect::CellElement {
                        cell: TileCell::Uncompressed,
                        element,
                    }
                    .into());
                }
            },
        }
        Ok(())
    }

    /// Decode one tile's primary `COMPRESSED_DATA` cell into the integer values of `tile`
    /// per `ZCMPTYPE`, and lend them from `codecs`. The cell is a byte array except for
    /// `PLIO_1` (i16).
    fn decode_cell<'s>(
        &self,
        cell: VlaCell<'_>,
        tile: &TileScratch,
        codecs: &'s mut CodecScratch,
    ) -> Result<&'s mut Vec<i64>> {
        let stream = convert::stream_cell(cell, self.codec.stream_kind())?;
        self.codec.decode_ints(self.tile(tile), stream, codecs)
    }
}

/// The codec of a `GZIP_COMPRESSED_DATA` fallback cell, whose level a decode does not
/// read.
const FALLBACK_GZIP: TileCodec = TileCodec::Gzip1 {
    level: gzip::DEFAULT_GZIP_LEVEL,
};

#[cfg(test)]
mod tests {
    use crate::compression::decode::tile_decoder::read_parameters;
    use crate::compression::hcompress::Smoothing;
    use crate::compression::plane::IntBitpix;
    use crate::compression::tile_codec::TileCodec;
    use crate::compression::{hcompress, rice};
    use crate::fits_error::{FitsError, ValueKind};
    use crate::header_model::Header;
    use crate::header_model::record_name::RecordName;
    use crate::keyword::Keyword;
    use std::assert_matches;

    /// Table 37's defaults, the parameters at any `ZNAMEi` index, and a value the
    /// codec refuses — which fails only the codec that reads it.
    #[test]
    fn codec_parameters_read_each_codecs_own_values() {
        let rice = TileCodec::parse("RICE_1").unwrap();
        let hcompress = TileCodec::parse("HCOMPRESS_1").unwrap();
        let gzip = TileCodec::parse("GZIP_1").unwrap();
        let mut header = Header::new();
        assert_eq!(
            read_parameters(&header, rice).unwrap(),
            TileCodec::Rice(rice::Codec {
                blocksize: 32,
                bytepix: IntBitpix::I32
            })
        );
        header
            .set_internal("ZNAME2", "BYTEPIX")
            .set_internal("ZVAL2", 8)
            .set_internal("ZNAME3", "BLOCKSIZE")
            .set_internal("ZVAL3", 16)
            .set_internal("ZNAME5", "SMOOTH")
            .set_internal("ZVAL5", 1);
        assert_eq!(
            read_parameters(&header, rice).unwrap(),
            TileCodec::Rice(rice::Codec {
                blocksize: 16,
                bytepix: IntBitpix::I64
            })
        );
        assert_matches!(
            read_parameters(&header, hcompress).unwrap(),
            TileCodec::Hcompress(hcompress::Codec {
                smoothing: Smoothing::On,
                ..
            })
        );

        for (name, value) in [("ZVAL3", 17), ("ZVAL2", 3)] {
            let mut refused = header.clone();
            refused.set_internal(name, value);
            assert_matches!(
                read_parameters(&refused, rice),
                Err(FitsError::InvalidKeywordValue { keyword, .. }) if keyword == name
            );
            // GZIP reads no `ZVALn`, so none of them refuses it.
            assert_eq!(read_parameters(&refused, gzip).unwrap(), gzip);
        }
        header.set_internal("ZVAL2", "not an integer");
        assert_matches!(
            read_parameters(&header, rice),
            Err(FitsError::ValueTypeMismatch { name, expected: ValueKind::Integer })
                if name == RecordName::Keyword(Keyword::new("ZVAL2"))
        );
    }
}
