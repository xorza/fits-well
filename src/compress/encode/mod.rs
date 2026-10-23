//! Tiled-image compression (§10.1).
//!
//! Tile, (quantize then) compress, and lay out the `ZIMAGE` `BINTABLE` data unit
//! (per-tile array descriptors + a heap of compressed tile bytes). Integer images
//! go straight through the codec ([`compress_image`]); float images are per-tile
//! quantized to int32 with a `ZSCALE`/`ZZERO` first ([`compress_float_image`]). The
//! per-codec work lives in the sibling codec modules.

use crate::bintable::descriptor::DescriptorWidth;
use crate::bitpix::Bitpix;
use crate::compress::convert;
use crate::compress::plane::{FloatSamples, IntBitpix, Samples};
use crate::compress::tile_geometry::TileGeometry;
use crate::compress::tile_geometry::TileScratch;
use crate::compress::{CompressionConfig, ImageCodec, map_tiles};
use crate::compress::{gzip, hcompress, plio, quantize, rice};
use crate::header_model::{Header, keywords, value};

use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::error::Result;
use crate::image_model::view::Borrowed;
use crate::keyword::Keyword;

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

/// Encode an image as a tiled-compressed `BINTABLE`: returns the `ZIMAGE` header and
/// writes the data unit (per-tile descriptors + the heap of compressed tile bytes) to
/// `out`. A float image goes through [`compress_float_image`].
pub(crate) fn compress_image(
    image: Borrowed<'_>,
    config: &CompressionConfig,
    out: &mut Vec<u8>,
) -> Result<Header> {
    if image.shape.len() > 999 {
        return Err(FitsError::KeywordOutOfRange {
            keyword: keywords::NAXIS,
        });
    }
    let total = image.samples.len();
    let compression = config.compression;
    let codec = compression.image_codec();
    if codec == ImageCodec::Hcompress1 && image.shape.len() != 2 {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1 requires a two-dimensional image".to_string(),
        });
    }
    let samples = match Samples::of(image.samples) {
        Samples::Float(samples) => {
            return compress_float_image(image, samples, config, out);
        }
        Samples::Int(samples) => samples,
    };
    let bitpix = samples.bitpix();
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;

    let geom = TileGeometry::new(dims, &tiles);
    // A `NAXIS = 0` image has no data array; `geom.ntiles()` would otherwise be the
    // empty product (1) and fabricate a phantom one-pixel tile that indexes the empty
    // sample buffer. Zero tiles yields an empty `NAXIS2 = 0` table, which the decoder's
    // matching `ZNAXIS = 0` guard turns back into the empty image.
    let ntiles = if total == 0 { 0 } else { geom.ntiles() };
    let (gzip_level, scale) = (compression.gzip_level(), compression.hcompress_scale());

    // Compress every tile independently (the compute-bound step — parallel under
    // the `parallel` feature). The heap layout is sequential (each descriptor's
    // offset is the running heap length), so concatenate the cells serially after.
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<TileCell> {
        geom.tile_into(t, &mut s.tile);
        // Gather + widen this tile's pixels straight from the typed source — no
        // whole-image `i64` buffer.
        convert::gather_i64(samples, &s.tile.row_bases, s.tile.row_len, &mut s.ints);
        let vals = &s.ints;
        Ok(match codec {
            ImageCodec::Gzip1 => {
                convert::i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip_encode(&s.be, gzip_level))
            }
            ImageCodec::Gzip2 => {
                convert::i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip2_encode(
                    &s.be,
                    bitpix.elem_size(),
                    gzip_level,
                    &mut s.gzip,
                ))
            }
            ImageCodec::Rice1 => TileCell::Bytes(rice::rice_encode(
                vals,
                bitpix,
                rice::BLOCKSIZE,
                &mut s.rice,
            )),
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
            ImageCodec::NoCompress => TileCell::Bytes(convert::i64_to_be(vals, bitpix)),
        })
    })?;

    let heap_len = cells.iter().try_fold(0usize, |len, cell| {
        len.checked_add(cell.byte_len()?)
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let maxnelem = cells.iter().map(TileCell::nelem).max().unwrap_or(0);
    // §10.1.3: use 64-bit `Q` descriptors once the heap or a tile's element count
    // exceeds the 32-bit `P` range; otherwise the compact `P` form.
    let width = DescriptorWidth::addressing(heap_len, maxnelem);
    let descriptor_width = width.bytes();

    // Data unit: an array descriptor (count, heap offset) per tile, then the heap.
    // Built into the caller's reused buffer (the writer's scratch).
    out.clear();
    let descriptor_len = ntiles
        .checked_mul(descriptor_width)
        .ok_or(FitsError::DataUnitOverflow)?;
    let output_len = descriptor_len
        .checked_add(heap_len)
        .ok_or(FitsError::DataUnitOverflow)?;
    out.reserve_exact(output_len);
    out.resize(descriptor_len, 0);
    for (tile, cell) in cells.into_iter().enumerate() {
        let offset = out.len() - descriptor_len;
        let slot = tile * descriptor_width;
        write_pq_descriptor(
            &mut out[slot..slot + descriptor_width],
            width,
            cell.nelem() as u64,
            offset as u64,
        )?;
        cell.append_to(out);
    }

    let tform_letter = if codec == ImageCodec::Plio1 { 'I' } else { 'B' };
    let mut h = container_header(descriptor_width, ntiles, heap_len, 1)?;
    h.set_internal(keywords::TTYPE.indexed(1)?, "COMPRESSED_DATA");
    h.set_internal(
        keywords::TFORM.indexed(1)?,
        format!("1{}{tform_letter}({maxnelem})", width.letter()),
    );
    set_zimage_axes(&mut h, compression.name(), bitpix.bitpix(), dims, &tiles)?;
    match codec {
        ImageCodec::Rice1 => set_rice_parameters(&mut h, bitpix)?,
        ImageCodec::Hcompress1 => {
            h.set_internal(keywords::ZNAME.indexed(1)?, "SCALE")
                .set_internal(keywords::ZVAL.indexed(1)?, scale);
        }
        _ => {}
    }
    image.scaling.add_to_header(&mut h, bitpix.bitpix())?;
    Ok(h)
}

/// The `BINTABLE` header of a compressed image's container, up to its columns:
/// `ntiles` rows of `row_len` bytes, a heap of `heap_len` bytes, `fields` columns.
fn container_header(row_len: usize, ntiles: usize, heap_len: usize, fields: i64) -> Result<Header> {
    let mut h = Header::new();
    h.set_internal(keywords::XTENSION, "BINTABLE")
        .set_comment_internal(keywords::XTENSION, "binary table extension");
    h.set_internal(keywords::BITPIX, 8)
        .set_internal(keywords::NAXIS, 2);
    h.set_internal(keywords::NAXIS.indexed(1)?, value::fits_i64(row_len)?)
        .set_internal(keywords::NAXIS.indexed(2)?, value::fits_i64(ntiles)?);
    h.set_internal(keywords::PCOUNT, value::fits_i64(heap_len)?)
        .set_internal(keywords::GCOUNT, 1);
    h.set_internal(keywords::TFIELDS, fields);
    Ok(h)
}

/// The `RICE_1` parameters every stream fits-well writes uses (§10.4.1 Table 37).
fn set_rice_parameters(h: &mut Header, bytepix: IntBitpix) -> Result<()> {
    h.set_internal(keywords::ZNAME.indexed(1)?, "BLOCKSIZE")
        .set_internal(
            keywords::ZVAL.indexed(1)?,
            i64::try_from(rice::BLOCKSIZE).unwrap(),
        );
    h.set_internal(keywords::ZNAME.indexed(2)?, "BYTEPIX")
        .set_internal(
            keywords::ZVAL.indexed(2)?,
            i64::try_from(bytepix.elem_size()).unwrap(),
        );
    Ok(())
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
    if absolute > f64::from(i32::MAX) {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1 tile scale exceeds the 32-bit stream range".to_string(),
        });
    }
    Ok(absolute as i32)
}

/// One encoded float tile.
#[derive(Debug)]
enum FloatTile {
    /// Quantized integers in `COMPRESSED_DATA`, with the `ZSCALE`/`ZZERO` that invert
    /// the quantization and whether a pixel was null.
    Quantized {
        bytes: Vec<u8>,
        zscale: f64,
        zzero: f64,
        has_null: bool,
    },
    /// A constant tile, which does not quantize, as raw gzip'd floats in the
    /// `GZIP_COMPRESSED_DATA` fallback.
    Raw { bytes: Vec<u8> },
}

impl FloatTile {
    fn bytes(&self) -> &[u8] {
        match self {
            Self::Quantized { bytes, .. } | Self::Raw { bytes } => bytes,
        }
    }
}

/// Encode a float image as a quantized, tiled-compressed `BINTABLE`, dithered by
/// the method `config` selects. Each tile is quantized to int32 with a per-tile
/// `ZSCALE`/`ZZERO`, then compressed with the selected lossless integer codec;
/// a tile that can't be quantized (constant data) is stored as raw gzip'd floats
/// in `GZIP_COMPRESSED_DATA`. The table has four columns: `COMPRESSED_DATA`,
/// `GZIP_COMPRESSED_DATA`, `ZSCALE`, `ZZERO`.
fn compress_float_image(
    image: Borrowed<'_>,
    samples: FloatSamples<'_>,
    config: &CompressionConfig,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let compression = config.compression;
    let codec = compression.image_codec();
    if !matches!(
        codec,
        ImageCodec::Gzip1 | ImageCodec::Gzip2 | ImageCodec::Rice1 | ImageCodec::NoCompress
    ) {
        return Err(FitsError::UnsupportedCompression {
            name: format!("{} for float images (write)", compression.name()),
        });
    }
    let zbitpix = samples.bitpix();
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;

    let geom = TileGeometry::new(dims, &tiles);
    // See `compress_image`: zero tiles for a `NAXIS = 0` image, avoiding the phantom
    // one-pixel tile the empty-product `ntiles` would otherwise produce.
    let ntiles = if image.samples.is_empty() {
        0
    } else {
        geom.ntiles()
    };

    let zdither0 = 1i64; // deterministic dither seed (any 1..=10000 is valid)
    let quantization = config.quantization.unwrap_or_default();
    let method = quantization.dither;
    let (gzip_level, qlevel) = (compression.gzip_level(), quantization.level);

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
            convert::gather_f64(samples, &s.tile.row_bases, s.tile.row_len, &mut s.floats);
            let irow = i64::try_from(t).expect("a tile row fits i64") + zdither0; // = (1-based tile row) + ZDITHER0 - 1
            let Some(q) =
                quantize::quantize_tile(&s.floats, nx, ny, qlevel, method, irow, &mut s.quantize)
            else {
                // Constant tile: store the raw floats, gzip'd, in the fallback.
                convert::float_to_be_into(&s.floats, zbitpix, &mut s.be);
                return Ok(FloatTile::Raw {
                    bytes: gzip::gzip_encode(&s.be, gzip_level),
                });
            };
            let bytes = match codec {
                ImageCodec::Gzip1 => {
                    convert::i32_to_be_into(&s.quantize.ints, &mut s.be);
                    gzip::gzip_encode(&s.be, gzip_level)
                }
                ImageCodec::Gzip2 => {
                    convert::i32_to_be_into(&s.quantize.ints, &mut s.be);
                    gzip::gzip2_encode(&s.be, 4, gzip_level, &mut s.gzip)
                }
                ImageCodec::Rice1 => rice::rice_encode(
                    &s.quantize.ints,
                    IntBitpix::I32,
                    rice::BLOCKSIZE,
                    &mut s.rice,
                ),
                ImageCodec::NoCompress => {
                    convert::i32_to_be_into(&s.quantize.ints, &mut s.be);
                    s.be.clone()
                }
                _ => unreachable!(),
            };
            Ok(FloatTile::Quantized {
                bytes,
                zscale: q.bscale,
                zzero: q.bzero,
                has_null: q.has_null,
            })
        },
    )?;

    let heap_len = tiles_out.iter().try_fold(0usize, |len, tile| {
        len.checked_add(tile.bytes().len())
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let max_cd = tiles_out
        .iter()
        .filter(|tile| matches!(tile, FloatTile::Quantized { .. }))
        .map(|tile| tile.bytes().len())
        .max()
        .unwrap_or(0);
    let max_gz = tiles_out
        .iter()
        .filter(|tile| matches!(tile, FloatTile::Raw { .. }))
        .map(|tile| tile.bytes().len())
        .max()
        .unwrap_or(0);
    let any_null = tiles_out
        .iter()
        .any(|tile| matches!(tile, FloatTile::Quantized { has_null: true, .. }));

    // The float container's row layout is a fixed pair of 32-bit `P` descriptors, so
    // unlike the integer encoder it can't promote to `Q`; a heap or tile element count
    // past the 32-bit range can't be addressed, so refuse rather than write a truncated
    // descriptor and an unreadable file. Reachable only for an absurdly large image.
    if DescriptorWidth::addressing(heap_len, max_cd.max(max_gz)) == DescriptorWidth::Q {
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
    for (tile_index, tile) in tiles_out.into_iter().enumerate() {
        let row = tile_index * 32;
        let offset = out.len() - rows_len;
        // A raw tile has no quantization to invert, so its `ZSCALE`/`ZZERO` are zero.
        let (mut bytes, zscale, zzero, cd_count, gz_count) = match tile {
            FloatTile::Quantized {
                bytes,
                zscale,
                zzero,
                ..
            } => {
                let len = bytes.len();
                (bytes, zscale, zzero, len, 0)
            }
            FloatTile::Raw { bytes } => {
                let len = bytes.len();
                (bytes, 0.0, 0.0, 0, len)
            }
        };
        write_pq_descriptor(
            &mut out[row..row + 8],
            DescriptorWidth::P,
            cd_count as u64,
            offset as u64,
        )?;
        write_pq_descriptor(
            &mut out[row + 8..row + 16],
            DescriptorWidth::P,
            gz_count as u64,
            offset as u64,
        )?;
        out[row + 16..row + 24].copy_from_slice(&zscale.to_be_bytes());
        out[row + 24..row + 32].copy_from_slice(&zzero.to_be_bytes());
        out.append(&mut bytes);
    }

    let mut h = container_header(32, ntiles, heap_len, 4)?;
    h.set_internal(keywords::TTYPE.indexed(1)?, "COMPRESSED_DATA")
        .set_internal(keywords::TFORM.indexed(1)?, format!("1PB({max_cd})"));
    h.set_internal(keywords::TTYPE.indexed(2)?, "GZIP_COMPRESSED_DATA")
        .set_internal(keywords::TFORM.indexed(2)?, format!("1PB({max_gz})"));
    h.set_internal(keywords::TTYPE.indexed(3)?, "ZSCALE")
        .set_internal(keywords::TFORM.indexed(3)?, "1D");
    h.set_internal(keywords::TTYPE.indexed(4)?, "ZZERO")
        .set_internal(keywords::TFORM.indexed(4)?, "1D");
    set_zimage_axes(&mut h, compression.name(), zbitpix.bitpix(), dims, &tiles)?;
    if codec == ImageCodec::Rice1 {
        set_rice_parameters(&mut h, IntBitpix::I32)?;
    }
    h.set_internal(keywords::ZQUANTIZ, method.name());
    h.set_internal(keywords::ZDITHER0, zdither0);
    if any_null {
        // Quantized nulls are stored as this reserved integer; ZBLANK tells the
        // decoder which value maps back to a blank (NaN) pixel.
        h.set_internal(keywords::ZBLANK, i64::from(quantize::NULL_VALUE));
    }
    image.scaling.add_to_header(&mut h, zbitpix.bitpix())?;
    Ok(h)
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
    const fn nelem(&self) -> usize {
        match self {
            Self::Bytes(b) => b.len(),
            Self::I16(v) => v.len(),
        }
    }

    fn byte_len(&self) -> Result<usize> {
        match self {
            Self::Bytes(bytes) => Ok(bytes.len()),
            Self::I16(values) => values
                .len()
                .checked_mul(2)
                .ok_or(FitsError::DataUnitOverflow),
        }
    }

    fn append_to(self, out: &mut Vec<u8>) {
        match self {
            Self::Bytes(mut bytes) => out.append(&mut bytes),
            Self::I16(v) => {
                for x in v {
                    out.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }
}

/// Resolve the tile shape for an image: an empty `tile_shape` defaults to row-tiling
/// (the first axis whole, the rest 1), or for `HCOMPRESS_1` to cfitsio's default (see
/// [`hcompress_default_rows`]); a non-empty one is used as given (each axis clamped to
/// ≥1) and must match the image rank, else [`FitsError::RankMismatch`]. An
/// `HCOMPRESS_1` tiling with a side under 4 pixels, the last tile's included, is
/// refused rather than adjusted.
fn resolve_tile_shape(
    dims: &[usize],
    tile_shape: &[usize],
    codec: ImageCodec,
) -> Result<Vec<usize>> {
    let tiles: Vec<usize> = if tile_shape.is_empty() {
        (0..dims.len())
            .map(|i| match i {
                0 => dims[i].max(1),
                1 if codec == ImageCodec::Hcompress1 => hcompress_default_rows(dims[1]).max(1),
                _ => 1,
            })
            .collect()
    } else if tile_shape.len() == dims.len() {
        tile_shape.iter().map(|&t| t.max(1)).collect()
    } else {
        return Err(FitsError::RankMismatch {
            ranked: Ranked::TileShape,
            expected: dims.len(),
            got: tile_shape.len(),
        });
    };
    // An empty image has no tiles to refuse.
    if codec == ImageCodec::Hcompress1 && !dims.contains(&0) {
        for (axis, (&length, &tile)) in dims.iter().zip(&tiles).enumerate() {
            let tile = tile.min(length);
            let shortest = match length % tile {
                0 => tile,
                remainder => remainder,
            };
            if shortest < HCOMPRESS_MIN_TILE {
                return Err(FitsError::HcompressTileTooSmall {
                    axis,
                    length: shortest,
                });
            }
        }
    }
    Ok(tiles)
}

/// The shortest tile side cfitsio's `HCOMPRESS_1` writes or reads.
const HCOMPRESS_MIN_TILE: usize = 4;

/// cfitsio's default `HCOMPRESS_1` tile height for an image of `rows` (`imcomp_init_table`,
/// pinned against `fpack -h` output): the whole image up to 30 rows, else the first of
/// 16, 24, 20, 30, 28, 26, 22, 18 and 14 rows that leaves no remainder or one of at
/// least [`HCOMPRESS_MIN_TILE`] rows, else 17.
pub(super) const fn hcompress_default_rows(rows: usize) -> usize {
    const CANDIDATES: [usize; 9] = [16, 24, 20, 30, 28, 26, 22, 18, 14];
    if rows <= 30 {
        return rows;
    }
    let mut i = 0;
    while i < CANDIDATES.len() {
        let remainder = rows % CANDIDATES[i];
        if remainder == 0 || remainder >= HCOMPRESS_MIN_TILE {
            return CANDIDATES[i];
        }
        i += 1;
    }
    17
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
    h.set_internal(keywords::ZIMAGE, true)
        .set_comment_internal(keywords::ZIMAGE, "this is a tiled-compressed image");
    h.set_internal(keywords::ZCMPTYPE, cmptype);
    h.set_internal(keywords::ZBITPIX, zbitpix.code());
    h.set_internal(keywords::ZNAXIS, value::fits_i64(dims.len())?);
    for (i, &n) in dims.iter().enumerate() {
        h.set_internal(
            Keyword::try_from(format_args!("ZNAXIS{}", i + 1))?,
            value::fits_i64(n)?,
        );
    }
    for (i, &t) in tiles.iter().enumerate() {
        h.set_internal(
            Keyword::try_from(format_args!("ZTILE{}", i + 1))?,
            value::fits_i64(t)?,
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests;
