//! The per-tile codec dispatch, fixed once for a whole tiled image.

use crate::bintable::vla_column::VlaCell;
use crate::compress;
use crate::compress::ImageCodec;
use crate::compress::convert;
use crate::compress::decode::float_quantization::Dequant;
use crate::compress::decode::tile_cells::TileCells;
use crate::compress::decode::tile_cells::TileSource;
use crate::compress::decode::tile_scratch_set::CodecScratch;
use crate::compress::decode::tiled_image::TiledImage;
use crate::compress::gzip;
use crate::compress::hcompress;
use crate::compress::plane::{FloatBitpix, IntBitpix, Plane};
use crate::compress::plio;
use crate::compress::quantize;
use crate::compress::rice;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::Header;
use crate::keyword::key;

/// The decode parameters constant across all of a tiled image's tiles: the codec,
/// the integer plane the tiles are stored in (and, for a float image, its float
/// `ZBITPIX`), and the codec knobs. Every per-tile entry point hangs off this, so
/// the helpers take one decoder rather than a long parameter list.
#[derive(Debug)]
pub(super) struct TileDecoder {
    codec: ImageCodec,
    int_bitpix: IntBitpix,
    /// `Some` exactly for a float image.
    float_bitpix: Option<FloatBitpix>,
    params: CodecParams,
}

/// The codec knobs from `ZNAMEi`/`ZVALi`, read in one pass: the Rice block size and
/// pixel width, and the HCOMPRESS `SMOOTH` flag, with the Table 37 defaults of 32, 4
/// and off. A codec reads only its own knobs, so a value another codec would refuse
/// does not fail the image.
#[derive(Debug, Clone, Copy)]
struct CodecParams {
    blocksize: usize,
    bytepix: IntBitpix,
    smooth: bool,
}

impl CodecParams {
    fn read(header: &Header, codec: ImageCodec) -> Result<CodecParams> {
        let mut params = CodecParams {
            blocksize: rice::BLOCKSIZE,
            bytepix: IntBitpix::I32,
            smooth: false,
        };
        for entry in header.iter() {
            let Some(i) = compress::parameter_index(entry.keyword) else {
                continue;
            };
            let Some(name) = header.get_text(entry.keyword)? else {
                continue;
            };
            let value = || header.get_integer(key!("ZVAL{i}").as_str());
            let out_of_range = FitsError::KeywordOutOfRange { name: "ZVALn" };
            match (codec, name) {
                (ImageCodec::Rice1, "BLOCKSIZE") => {
                    if let Some(v) = value()? {
                        params.blocksize = match v {
                            16 => 16,
                            32 => 32,
                            _ => return Err(out_of_range),
                        };
                    }
                }
                (ImageCodec::Rice1, "BYTEPIX") => {
                    if let Some(v) = value()? {
                        params.bytepix = usize::try_from(v)
                            .ok()
                            .and_then(IntBitpix::from_bytepix)
                            .ok_or(out_of_range)?;
                    }
                }
                (ImageCodec::Hcompress1, "SMOOTH") => {
                    params.smooth = value()?.unwrap_or(0) != 0;
                }
                _ => {}
            }
        }
        Ok(params)
    }
}

impl TileDecoder {
    pub(super) fn new(header: &Header, tiled: &TiledImage<'_>) -> Result<TileDecoder> {
        let params = CodecParams::read(header, tiled.codec)?;
        // A float image's tiles arrive quantized, so they decode in whichever integer
        // width the codec stored them at — RICE_1 says so in `BYTEPIX`, the rest use
        // 32-bit — and only then dequantize to `ZBITPIX`.
        let (int_bitpix, float_bitpix) = match Plane::of(tiled.image.bitpix) {
            Plane::Int(bitpix) => (bitpix, None),
            Plane::Float(bitpix) if tiled.codec == ImageCodec::Rice1 => {
                (params.bytepix, Some(bitpix))
            }
            Plane::Float(bitpix) => (IntBitpix::I32, Some(bitpix)),
        };
        Ok(TileDecoder {
            codec: tiled.codec,
            int_bitpix,
            float_bitpix,
            params,
        })
    }

    /// The float `ZBITPIX` of a float image.
    fn zbitpix(&self) -> FloatBitpix {
        self.float_bitpix
            .expect("only a float image decodes in the float plane")
    }

    /// Decode one tile of an *integer* image into `out`.
    pub(super) fn decode_tile_into(
        &self,
        cells: TileCells<'_>,
        tile_elems: usize,
        out: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<()> {
        match cells.resolve()? {
            TileSource::Compressed(cell) => self.decode_cell_into(cell, tile_elems, out, scratch),
            TileSource::Gzip(cell) => gzip::gzip_tile_into(
                convert::byte_cell(cell)?,
                self.int_bitpix,
                tile_elems,
                out,
                &mut scratch.gzip,
            ),
            TileSource::Uncompressed(cell) => convert::cell_to_i64_into(cell, out),
        }
    }

    /// Decode one tile of a *float* image into `out`. A primary `COMPRESSED_DATA` cell
    /// holds quantized integers (decoded into the reused `ints` buffer, then dequantized
    /// as `scale·int + zero`); otherwise the `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA`
    /// fallbacks hold the raw float values.
    pub(super) fn decode_float_tile_into(
        &self,
        cells: TileCells<'_>,
        tile_elems: usize,
        dq: Dequant,
        out: &mut Vec<f64>,
        ints: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<()> {
        match cells.resolve()? {
            TileSource::Compressed(cell) => {
                // The primary stream holds quantized integers for every float-image codec.
                self.decode_cell_into(cell, tile_elems, ints, scratch)?;
                quantize::dequantize_into(
                    ints, dq.scale, dq.zero, dq.method, dq.irow, dq.zblank, out,
                );
                Ok(())
            }
            TileSource::Gzip(cell) => {
                // Raw floats, bounded at the tile's known byte size (`tile_elems` floats).
                let max = tile_elems.saturating_mul(self.zbitpix().elem_size());
                gzip::gunzip_into(convert::byte_cell(cell)?, max, &mut scratch.gzip.bytes)?;
                convert::be_floats_into(&scratch.gzip.bytes, self.zbitpix(), out);
                Ok(())
            }
            TileSource::Uncompressed(cell) => convert::cell_to_f64_into(cell, self.zbitpix(), out),
        }
    }

    /// Decode one tile's primary `COMPRESSED_DATA` cell into `tile_elems` integer values
    /// in `out`, per `ZCMPTYPE`. The cell is a byte array except for `PLIO_1` (i16).
    fn decode_cell_into(
        &self,
        cell: VlaCell<'_>,
        tile_elems: usize,
        out: &mut Vec<i64>,
        scratch: &mut CodecScratch,
    ) -> Result<()> {
        let params = self.params;
        match self.codec {
            ImageCodec::Gzip1 => gzip::gzip_tile_into(
                convert::byte_cell(cell)?,
                self.int_bitpix,
                tile_elems,
                out,
                &mut scratch.gzip,
            ),
            ImageCodec::Gzip2 => gzip::gzip2_tile_into(
                convert::byte_cell(cell)?,
                self.int_bitpix,
                tile_elems,
                out,
                &mut scratch.gzip,
            ),
            ImageCodec::Rice1 => rice::rice_decode_into(
                convert::byte_cell(cell)?,
                tile_elems,
                params.bytepix,
                params.blocksize,
                out,
            ),
            ImageCodec::Plio1 => {
                plio::plio_decode_be_into(convert::plio_cell(cell)?, tile_elems, out)
            }
            ImageCodec::Hcompress1 => hcompress::hcompress_tile_into(
                convert::byte_cell(cell)?,
                params.smooth,
                tile_elems,
                out,
                &mut scratch.hcompress,
            ),
            // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
            ImageCodec::NoCompress => {
                let bytes = convert::byte_cell(cell)?;
                let expected = tile_elems
                    .checked_mul(self.int_bitpix.elem_size())
                    .ok_or(FitsError::DataUnitOverflow)?;
                if bytes.len() != expected {
                    return Err(FitsError::DataSizeMismatch {
                        expected,
                        got: bytes.len(),
                    });
                }
                convert::be_to_i64_into(bytes, self.int_bitpix, out);
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compress::ImageCodec;
    use crate::compress::decode::tile_decoder::CodecParams;
    use crate::compress::plane::IntBitpix;
    use crate::error::FitsError;
    use crate::header_model::Header;

    /// Table 37's defaults, the parameters at any `ZNAMEi` index, and a value the
    /// codec refuses — which fails only the codec that reads it.
    #[test]
    fn codec_parameters_read_each_codecs_own_values() {
        let mut header = Header::new();
        let defaults = CodecParams::read(&header, ImageCodec::Rice1).unwrap();
        assert_eq!(
            (defaults.blocksize, defaults.bytepix, defaults.smooth),
            (32, IntBitpix::I32, false)
        );
        header
            .set_internal("ZNAME2", "BYTEPIX")
            .set_internal("ZVAL2", 8)
            .set_internal("ZNAME3", "BLOCKSIZE")
            .set_internal("ZVAL3", 16)
            .set_internal("ZNAME5", "SMOOTH")
            .set_internal("ZVAL5", 1);
        let rice = CodecParams::read(&header, ImageCodec::Rice1).unwrap();
        assert_eq!(
            (rice.blocksize, rice.bytepix, rice.smooth),
            (16, IntBitpix::I64, false)
        );
        assert!(
            CodecParams::read(&header, ImageCodec::Hcompress1)
                .unwrap()
                .smooth
        );

        for (name, value) in [("ZVAL3", 17), ("ZVAL2", 3)] {
            let mut refused = header.clone();
            refused.set_internal(name, value);
            assert!(matches!(
                CodecParams::read(&refused, ImageCodec::Rice1),
                Err(FitsError::KeywordOutOfRange { name: "ZVALn" })
            ));
            assert!(CodecParams::read(&refused, ImageCodec::Gzip1).is_ok());
        }
        header.set_internal("ZVAL2", "not an integer");
        assert!(matches!(
            CodecParams::read(&header, ImageCodec::Rice1),
            Err(FitsError::TypeMismatch { name, expected })
                if name == "ZVAL2" && expected == "integer"
        ));
    }
}
