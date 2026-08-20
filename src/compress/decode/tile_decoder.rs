//! The per-tile codec dispatch, fixed once for a whole tiled image.

use crate::bitpix::Bitpix;
use crate::compress;
use crate::compress::ImageCodec;
use crate::compress::convert;
use crate::compress::decode::float_quantization::Dequant;
use crate::compress::decode::image_layout::ImageLayout;
use crate::compress::decode::tile_cells::TileCells;
use crate::compress::decode::tile_cells::TileSource;
use crate::compress::decode::tile_scratch_set::CodecScratch;
use crate::compress::gzip;
use crate::compress::hcompress;
use crate::compress::plio;
use crate::compress::quantize;
use crate::compress::rice;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::vla_column::VlaCell;

/// The decode parameters constant across all of a tiled image's tiles: the codec,
/// the stored/quantized integer bitpix (and float `ZBITPIX`), and the codec knobs.
/// Every per-tile entry point hangs off this, so the helpers take one decoder rather
/// than a long parameter list.
#[derive(Debug)]
pub(super) struct TileDecoder {
    codec: ImageCodec,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    params: CodecParams,
}

/// The codec knobs from `ZNAMEi`/`ZVALi`: Rice block size & pixel width, and the
/// HCOMPRESS `SMOOTH` flag.
#[derive(Debug, Clone, Copy)]
struct CodecParams {
    blocksize: usize,
    bytepix: usize,
    smooth: bool,
}

impl TileDecoder {
    pub(super) fn new(header: &Header, layout: &ImageLayout) -> Result<TileDecoder> {
        let rice = rice::rice_params(header)?;
        // A float image's tiles arrive quantized, so they decode in whichever integer
        // width the codec stored them at — RICE_1 says so in `BYTEPIX`, the rest use
        // 32-bit — and only then dequantize to `ZBITPIX`.
        let int_bitpix = if !layout.bitpix.is_float() {
            layout.bitpix
        } else if layout.codec == ImageCodec::Rice1 {
            convert::bytepix_to_bitpix(rice.bytepix)
        } else {
            Bitpix::I32
        };
        Ok(TileDecoder {
            codec: layout.codec,
            zbitpix: layout.bitpix,
            int_bitpix,
            params: CodecParams {
                blocksize: rice.blocksize,
                bytepix: rice.bytepix,
                smooth: hcompress_smooth(header)?,
            },
        })
    }

    /// Whether the image decodes in the float plane (`ZBITPIX` is a float type).
    pub(super) fn is_float(&self) -> bool {
        self.zbitpix.is_float()
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
            TileSource::Uncompressed(cell) => {
                convert::cell_to_i64_into(cell, out);
                Ok(())
            }
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
                let max = tile_elems.saturating_mul(self.zbitpix.elem_size());
                gzip::gunzip_into(convert::byte_cell(cell)?, max, &mut scratch.gzip.bytes)?;
                convert::be_floats_into(&scratch.gzip.bytes, self.zbitpix, out);
                Ok(())
            }
            TileSource::Uncompressed(cell) => {
                convert::cell_to_f64_into(cell, self.zbitpix, out);
                Ok(())
            }
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
            ImageCodec::Rice1 => {
                if !matches!(params.bytepix, 1 | 2 | 4 | 8) {
                    return Err(FitsError::UnsupportedCompression {
                        name: format!("RICE_1 with BYTEPIX = {}", params.bytepix),
                    });
                }
                rice::rice_decode_into(
                    convert::byte_cell(cell)?,
                    tile_elems,
                    params.bytepix,
                    params.blocksize,
                    out,
                )
            }
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

/// HCOMPRESS smoothing flag: the `SMOOTH` `ZVALn` is non-zero (cfitsio applies
/// inverse-transform smoothing to suppress blocking in lossy images).
fn hcompress_smooth(header: &Header) -> Result<bool> {
    for entry in header.iter() {
        let Some(i) = compress::parameter_index(entry.keyword) else {
            continue;
        };
        let Some(name) = header.get_text(entry.keyword)? else {
            continue;
        };
        if name == "SMOOTH" {
            return Ok(header.get_integer(key!("ZVAL{i}").as_str())?.unwrap_or(0) != 0);
        }
    }
    Ok(false)
}
