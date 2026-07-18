//! Tiled-image compression (§10.1).
//!
//! Tile, (quantize then) compress, and lay out the `ZIMAGE` `BINTABLE` data unit
//! (per-tile array descriptors + a heap of compressed tile bytes). Integer images
//! go straight through the codec ([`compress_image`]); float images are per-tile
//! quantized to int32 with a `ZSCALE`/`ZZERO` first ([`compress_float_image`]). The
//! per-codec work lives in the sibling codec modules.

use crate::compress::convert::float_to_be_into;
use crate::compress::convert::gather_f64;
use crate::compress::convert::gather_i64;
use crate::compress::convert::i32_to_be_into;
use crate::compress::convert::i64_to_be;
use crate::compress::convert::i64_to_be_into;
use crate::compress::geometry::TileGeometry;
use crate::compress::geometry::TileScratch;
use crate::compress::{
    Compression, CompressionOptions, DitherMethod, ImageCodec, map_tiles, needs_wide,
};
use crate::compress::{gzip, hcompress, plio, quantize, rice};

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

/// Per-worker tile-encode scratch, reused across the tiles one rayon worker
/// handles (via `map_tiles`'s `init`): the tile geometry plus the gather and codec
/// workspaces. Reusing them means steady-state tile compression allocates only each
/// tile's retained compressed output.
#[derive(Debug, Default)]
struct EncodeScratch {
    tile: TileScratch,
    /// The tile's pixels widened to `i64` for integer codecs.
    ints: Vec<i64>,
    /// The tile's pixels as `f64` (float images only; stays empty otherwise).
    floats: Vec<f64>,
    /// The tile's pixels packed to big-endian bytes — the gzip codecs' input,
    /// reused so each tile allocates only its compressed output.
    be: Vec<u8>,
    quantize: quantize::QuantizeScratch,
    gzip: gzip::GzipScratch,
    rice: rice::RiceScratch,
    hcompress: hcompress::HcompressScratch,
}

/// Encode an integer [`Image`] as a tiled-compressed `BINTABLE`: returns the
/// `ZIMAGE` header and the data unit (per-tile `P` descriptors + the heap of
/// compressed tile bytes). Float images and codecs without an encoder are rejected.
pub(crate) fn compress_image(
    image: &Image,
    compression: Compression,
    options: &CompressionOptions,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let total = validate_image(image)?;
    let codec = compression.image_codec();
    if codec == ImageCodec::Hcompress1 && image.shape.len() != 2 {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1 requires a two-dimensional image".to_string(),
        });
    }
    let bitpix = image.samples.bitpix();
    if bitpix.is_float() {
        return compress_float_image(image, compression, options, total, out);
    }
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, &options.tile_shape)?;

    let geom = TileGeometry::new(dims, &tiles);
    // A `NAXIS = 0` image has no data array; `geom.ntiles()` would otherwise be the
    // empty product (1) and fabricate a phantom one-pixel tile that indexes the empty
    // sample buffer. Zero tiles yields an empty `NAXIS2 = 0` table, which the decoder's
    // matching `ZNAXIS = 0` guard turns back into the empty image.
    let ntiles = if total == 0 { 0 } else { geom.ntiles() };
    let bytepix = bitpix.elem_size();
    let (gzip_level, scale) = (compression.gzip_level(), compression.hcompress_scale());

    // Compress every tile independently (the compute-bound step — parallel under
    // the `parallel` feature). The heap layout is sequential (each descriptor's
    // offset is the running heap length), so concatenate the cells serially after.
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<TileCell> {
        geom.tile_into(t, &mut s.tile);
        // Gather + widen this tile's pixels straight from the typed source — no
        // whole-image `i64` buffer.
        gather_i64(
            &image.samples,
            &s.tile.row_bases,
            s.tile.row_len,
            &mut s.ints,
        );
        let vals = &s.ints;
        Ok(match codec {
            ImageCodec::Gzip1 => {
                i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip_encode(&s.be, gzip_level))
            }
            ImageCodec::Gzip2 => {
                i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip2_encode(&s.be, bytepix, gzip_level, &mut s.gzip))
            }
            ImageCodec::Rice1 => TileCell::Bytes(rice::rice_encode(vals, bytepix, 32, &mut s.rice)),
            ImageCodec::Plio1 => TileCell::I16(plio::plio_encode(vals)?),
            ImageCodec::Hcompress1 => {
                let tile_scale = hcompress_tile_scale(
                    vals,
                    &s.tile.tdims,
                    scale,
                    &mut s.floats,
                    &mut s.quantize,
                )?;
                if tile_scale > 1
                    && image
                        .scaling
                        .blank
                        .is_some_and(|blank| vals.contains(&blank))
                {
                    return Err(FitsError::UnsupportedCompression {
                        name: "lossy HCOMPRESS_1 with undefined pixels requires a null mask"
                            .to_string(),
                    });
                }
                TileCell::Bytes(hcompress::hcompress_tile_encode(
                    vals,
                    &s.tile.tdims,
                    tile_scale,
                    &mut s.hcompress,
                )?)
            }
            // §10.4: store the tile's raw big-endian pixels, uncompressed.
            ImageCodec::NoCompress => TileCell::Bytes(i64_to_be(vals, bitpix)),
        })
    })?;

    let heap_len = cells.iter().try_fold(0usize, |len, cell| {
        len.checked_add(cell.byte_len()?)
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let maxnelem = cells.iter().map(TileCell::nelem).max().unwrap_or(0);
    // §10.1.3: use 64-bit `Q` descriptors once the heap or a tile's element count
    // exceeds the 32-bit `P` range; otherwise the compact `P` form.
    let wide = needs_wide(heap_len, maxnelem);

    // Data unit: an array descriptor (count, heap offset) per tile, then the heap.
    // Built into the caller's reused buffer (the writer's scratch).
    out.clear();
    let descriptor_len = ntiles
        .checked_mul(if wide { 16 } else { 8 })
        .ok_or(FitsError::DataUnitOverflow)?;
    let output_len = descriptor_len
        .checked_add(heap_len)
        .ok_or(FitsError::DataUnitOverflow)?;
    out.reserve_exact(output_len);
    out.resize(descriptor_len, 0);
    let descriptor_width = if wide { 16 } else { 8 };
    for (tile, cell) in cells.into_iter().enumerate() {
        let offset = out.len() - descriptor_len;
        let slot = tile * descriptor_width;
        write_pq_descriptor(
            &mut out[slot..slot + descriptor_width],
            wide,
            cell.nelem() as u64,
            offset as u64,
        )?;
        cell.append_to(out);
    }

    let tform_letter = if codec == ImageCodec::Plio1 { 'I' } else { 'B' };
    let desc = if wide { 'Q' } else { 'P' };
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .comment_internal("XTENSION", "binary table extension");
    h.set_internal("BITPIX", 8).set_internal("NAXIS", 2);
    h.set_internal("NAXIS1", if wide { 16 } else { 8 })
        .set_internal("NAXIS2", fits_i64(ntiles)?);
    h.set_internal("PCOUNT", fits_i64(heap_len)?)
        .set_internal("GCOUNT", 1);
    h.set_internal("TFIELDS", 1);
    h.set_internal("TTYPE1", "COMPRESSED_DATA");
    h.set_internal("TFORM1", format!("1{desc}{tform_letter}({maxnelem})"));
    set_zimage_axes(&mut h, compression.name(), bitpix, dims, &tiles)?;
    match codec {
        ImageCodec::Rice1 => {
            h.set_internal("ZNAME1", "BLOCKSIZE")
                .set_internal("ZVAL1", 32);
            h.set_internal("ZNAME2", "BYTEPIX")
                .set_internal("ZVAL2", bytepix as i64);
        }
        ImageCodec::Hcompress1 => {
            h.set_internal("ZNAME1", "SCALE")
                .set_internal("ZVAL1", scale);
        }
        _ => {}
    }
    image.scaling.add_to_header(&mut h, bitpix)?;
    Ok(h)
}

fn hcompress_tile_scale(
    values: &[i64],
    dimensions: &[usize],
    factor: f64,
    floats: &mut Vec<f64>,
    scratch: &mut quantize::QuantizeScratch,
) -> Result<i32> {
    if factor == 0.0 {
        return Ok(0);
    }
    floats.clear();
    floats.extend(values.iter().map(|&value| value as f64));
    let nx = dimensions[0];
    let ny = dimensions[1];
    let absolute = (factor * quantize::noise3_estimate(floats, nx, ny, scratch)).round();
    if absolute > i32::MAX as f64 {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1 tile scale exceeds the 32-bit stream range".to_string(),
        });
    }
    Ok(absolute as i32)
}

/// One encoded float tile: its compressed bytes plus the per-tile dequantization
/// metadata. `quantized` distinguishes a normal tile (bytes → `COMPRESSED_DATA`,
/// `zscale`/`zzero` meaningful) from a constant tile stored as raw gzip'd floats in
/// the `GZIP_COMPRESSED_DATA` fallback.
#[derive(Debug)]
struct FloatTile {
    bytes: Vec<u8>,
    zscale: f64,
    zzero: f64,
    quantized: bool,
    has_null: bool,
}

/// Encode a float [`Image`] as a quantized, tiled-compressed `BINTABLE`
/// (`SUBTRACTIVE_DITHER_1`). Each tile is quantized to int32 with a per-tile
/// `ZSCALE`/`ZZERO`, then compressed with the selected lossless integer codec;
/// a tile that can't be quantized (constant data) is stored as raw gzip'd floats
/// in `GZIP_COMPRESSED_DATA`. The table has four columns: `COMPRESSED_DATA`,
/// `GZIP_COMPRESSED_DATA`, `ZSCALE`, `ZZERO`.
fn compress_float_image(
    image: &Image,
    compression: Compression,
    options: &CompressionOptions,
    total: usize,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let codec = compression.image_codec();
    if !matches!(
        codec,
        ImageCodec::Gzip1 | ImageCodec::Gzip2 | ImageCodec::Rice1 | ImageCodec::NoCompress
    ) {
        return Err(FitsError::UnsupportedCompression {
            name: format!("{} for float images (write)", compression.name()),
        });
    }
    let zbitpix = image.samples.bitpix();
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, &options.tile_shape)?;

    let geom = TileGeometry::new(dims, &tiles);
    // See `compress_image`: zero tiles for a `NAXIS = 0` image, avoiding the phantom
    // one-pixel tile the empty-product `ntiles` would otherwise produce.
    let ntiles = if total == 0 { 0 } else { geom.ntiles() };

    let zdither0 = 1i64; // deterministic dither seed (any 1..=10000 is valid)
    let method = options.quantization.dither;
    let (gzip_level, qlevel) = (compression.gzip_level(), options.quantization.level);

    // Quantize + compress each tile independently (the compute-bound step —
    // parallel under the `parallel` feature); the §10 row layout and heap offsets
    // are assembled serially after, since they are sequential.
    let tiles_out = map_tiles(
        ntiles,
        EncodeScratch::default,
        |s, t| -> Result<FloatTile> {
            geom.tile_into(t, &mut s.tile);
            let nx = s.tile.tdims[0];
            let ny = s.tile.row_bases.len();
            // Gather + widen this tile's pixels straight from the typed source.
            gather_f64(
                &image.samples,
                &s.tile.row_bases,
                s.tile.row_len,
                &mut s.floats,
            );
            let irow = t as i64 + zdither0; // = (1-based tile row) + ZDITHER0 - 1
            Ok(
                match quantize::quantize_tile(
                    &s.floats,
                    nx,
                    ny,
                    qlevel,
                    method,
                    irow,
                    &mut s.quantize,
                ) {
                    Some(q) => {
                        let bytes = match codec {
                            ImageCodec::Gzip1 => {
                                i32_to_be_into(&s.quantize.ints, &mut s.be);
                                gzip::gzip_encode(&s.be, gzip_level)
                            }
                            ImageCodec::Gzip2 => {
                                i32_to_be_into(&s.quantize.ints, &mut s.be);
                                gzip::gzip2_encode(&s.be, 4, gzip_level, &mut s.gzip)
                            }
                            ImageCodec::Rice1 => {
                                rice::rice_encode(&s.quantize.ints, 4, 32, &mut s.rice)
                            }
                            ImageCodec::NoCompress => {
                                i32_to_be_into(&s.quantize.ints, &mut s.be);
                                s.be.clone()
                            }
                            _ => unreachable!(),
                        };
                        FloatTile {
                            bytes,
                            zscale: q.bscale,
                            zzero: q.bzero,
                            quantized: true,
                            has_null: q.has_null,
                        }
                    }
                    // Constant tile: store the raw floats, gzip'd, in the fallback.
                    None => {
                        float_to_be_into(&s.floats, zbitpix, &mut s.be);
                        FloatTile {
                            bytes: gzip::gzip_encode(&s.be, gzip_level),
                            zscale: 0.0,
                            zzero: 0.0,
                            quantized: false,
                            has_null: false,
                        }
                    }
                },
            )
        },
    )?;

    let heap_len = tiles_out.iter().try_fold(0usize, |len, tile| {
        len.checked_add(tile.bytes.len())
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let max_cd = tiles_out
        .iter()
        .filter(|tile| tile.quantized)
        .map(|tile| tile.bytes.len())
        .max()
        .unwrap_or(0);
    let max_gz = tiles_out
        .iter()
        .filter(|tile| !tile.quantized)
        .map(|tile| tile.bytes.len())
        .max()
        .unwrap_or(0);
    let any_null = tiles_out.iter().any(|tile| tile.has_null);

    // The float container's row layout is a fixed pair of 32-bit `P` descriptors, so
    // unlike the integer encoder it can't promote to `Q`; a heap or tile element count
    // past the 32-bit range can't be addressed, so refuse rather than write a truncated
    // descriptor and an unreadable file. Reachable only for an absurdly large image.
    if needs_wide(heap_len, max_cd.max(max_gz)) {
        return Err(FitsError::DataUnitOverflow);
    }

    // Fixed table: per tile, the two `P` descriptors then the `ZSCALE`/`ZZERO`
    // doubles (row width 32), followed by the heap.
    out.clear();
    let rows_len = ntiles.checked_mul(32).ok_or(FitsError::DataUnitOverflow)?;
    let output_len = rows_len
        .checked_add(heap_len)
        .ok_or(FitsError::DataUnitOverflow)?;
    out.reserve_exact(output_len);
    out.resize(rows_len, 0);
    for (tile_index, mut tile) in tiles_out.into_iter().enumerate() {
        let row = tile_index * 32;
        let offset = out.len() - rows_len;
        let (cd_count, gz_count) = if tile.quantized {
            (tile.bytes.len(), 0)
        } else {
            (0, tile.bytes.len())
        };
        write_pq_descriptor(
            &mut out[row..row + 8],
            false,
            cd_count as u64,
            offset as u64,
        )?;
        write_pq_descriptor(
            &mut out[row + 8..row + 16],
            false,
            gz_count as u64,
            offset as u64,
        )?;
        out[row + 16..row + 24].copy_from_slice(&tile.zscale.to_be_bytes());
        out[row + 24..row + 32].copy_from_slice(&tile.zzero.to_be_bytes());
        out.append(&mut tile.bytes);
    }

    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .comment_internal("XTENSION", "binary table extension");
    h.set_internal("BITPIX", 8).set_internal("NAXIS", 2);
    h.set_internal("NAXIS1", 32)
        .set_internal("NAXIS2", fits_i64(ntiles)?);
    h.set_internal("PCOUNT", fits_i64(heap_len)?)
        .set_internal("GCOUNT", 1);
    h.set_internal("TFIELDS", 4);
    h.set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("TFORM1", format!("1PB({max_cd})"));
    h.set_internal("TTYPE2", "GZIP_COMPRESSED_DATA")
        .set_internal("TFORM2", format!("1PB({max_gz})"));
    h.set_internal("TTYPE3", "ZSCALE")
        .set_internal("TFORM3", "1D");
    h.set_internal("TTYPE4", "ZZERO")
        .set_internal("TFORM4", "1D");
    set_zimage_axes(&mut h, compression.name(), zbitpix, dims, &tiles)?;
    if codec == ImageCodec::Rice1 {
        h.set_internal("ZNAME1", "BLOCKSIZE")
            .set_internal("ZVAL1", 32);
        h.set_internal("ZNAME2", "BYTEPIX").set_internal("ZVAL2", 4);
    }
    h.set_internal("ZQUANTIZ", dither_name(method));
    h.set_internal("ZDITHER0", zdither0);
    if any_null {
        // Quantized nulls are stored as this reserved integer; ZBLANK tells the
        // decoder which value maps back to a blank (NaN) pixel.
        h.set_internal("ZBLANK", quantize::NULL_VALUE as i64);
    }
    image.scaling.add_to_header(&mut h, zbitpix)?;
    Ok(h)
}

/// The `ZQUANTIZ` keyword string for a dither method.
fn dither_name(method: DitherMethod) -> &'static str {
    match method {
        DitherMethod::None => "NO_DITHER",
        DitherMethod::Subtractive1 => "SUBTRACTIVE_DITHER_1",
        DitherMethod::Subtractive2 => "SUBTRACTIVE_DITHER_2",
    }
}

/// One tile's compressed payload: a byte stream (`1PB`) for most codecs, or an
/// i16 instruction list (`1PI`) for `PLIO_1`.
#[derive(Debug)]
enum TileCell {
    Bytes(Vec<u8>),
    I16(Vec<i16>),
}

impl TileCell {
    /// Element count for the `P` descriptor (bytes, or i16 words).
    fn nelem(&self) -> usize {
        match self {
            TileCell::Bytes(b) => b.len(),
            TileCell::I16(v) => v.len(),
        }
    }

    fn byte_len(&self) -> Result<usize> {
        match self {
            TileCell::Bytes(bytes) => Ok(bytes.len()),
            TileCell::I16(values) => values
                .len()
                .checked_mul(2)
                .ok_or(FitsError::DataUnitOverflow),
        }
    }

    fn append_to(self, out: &mut Vec<u8>) {
        match self {
            TileCell::Bytes(mut bytes) => out.append(&mut bytes),
            TileCell::I16(v) => {
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }
}

/// Resolve the tile shape for an image: an empty `tile_shape` defaults to row-tiling
/// (the first axis whole, the rest 1); a non-empty one is used as given (each axis
/// clamped to ≥1) and must match the image rank, else [`FitsError::TileShapeRankMismatch`].
fn resolve_tile_shape(dims: &[usize], tile_shape: &[usize]) -> Result<Vec<usize>> {
    if tile_shape.is_empty() {
        Ok((0..dims.len())
            .map(|i| if i == 0 { dims[i].max(1) } else { 1 })
            .collect())
    } else if tile_shape.len() == dims.len() {
        Ok(tile_shape.iter().map(|&t| t.max(1)).collect())
    } else {
        Err(FitsError::TileShapeRankMismatch {
            tile_rank: tile_shape.len(),
            image_rank: dims.len(),
        })
    }
}

/// Append the `ZIMAGE`/`ZCMPTYPE`/`ZBITPIX`/`ZNAXIS` keywords plus the per-axis
/// `ZNAXISn`/`ZTILEn` series — the block shared verbatim by both the integer and
/// float `ZIMAGE` headers.
fn set_zimage_axes(
    h: &mut Header,
    cmptype: &str,
    zbitpix: Bitpix,
    dims: &[usize],
    tiles: &[usize],
) -> Result<()> {
    h.set_internal("ZIMAGE", true)
        .comment_internal("ZIMAGE", "this is a tiled-compressed image");
    h.set_internal("ZCMPTYPE", cmptype);
    h.set_internal("ZBITPIX", zbitpix.code());
    h.set_internal("ZNAXIS", fits_i64(dims.len())?);
    for (i, &n) in dims.iter().enumerate() {
        h.set_internal(key!("ZNAXIS{}", i + 1).as_str(), fits_i64(n)?);
    }
    for (i, &t) in tiles.iter().enumerate() {
        h.set_internal(key!("ZTILE{}", i + 1).as_str(), fits_i64(t)?);
    }
    Ok(())
}

fn validate_image(image: &Image) -> Result<usize> {
    image.scaling.validate(image.samples.bitpix())?;
    if image.shape.len() > 999 {
        return Err(FitsError::KeywordOutOfRange { name: "NAXIS" });
    }
    image.validate_geometry()
}

fn fits_i64(value: usize) -> Result<i64> {
    i64::try_from(value).map_err(|_| FitsError::DataUnitOverflow)
}
