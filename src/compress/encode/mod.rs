//! Tiled-image compression (§10.1).
//!
//! Tile, (quantize then) compress, and lay out the `ZIMAGE` `BINTABLE` data unit
//! (per-tile array descriptors + a heap of compressed tile bytes). Integer images
//! go straight through the codec ([`compress_image`]); float images are per-tile
//! quantized to int32 with a `ZSCALE`/`ZZERO` first ([`compress_float_image`]). The
//! per-codec work lives in the sibling codec modules.

use crate::bintable::descriptor::DescriptorWidth;
use crate::bitpix::Bitpix;
use crate::compress;
use crate::compress::convert;
use crate::compress::plane::{FloatBitpix, FloatSamples, IntBitpix, Samples};
use crate::compress::quantization::{DitherMethod, Levels, Seed, Spacing};
use crate::compress::quantize::{Precision, Quantizer, TileValues};
use crate::compress::tile_geometry::TileGeometry;
use crate::compress::tile_geometry::TileScratch;
use crate::compress::{Compression, CompressionConfig, HcompressScale, ImageCodec, map_tiles};
use crate::compress::{gzip, hcompress, plio, quantize, rice};
use crate::header_model::{Header, keywords, value};

use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::error::Result;
use crate::image_model::scaling::Scaling;
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
            return compress_float_image(FloatImage { image, samples }, config, out);
        }
        Samples::Int(samples) => samples,
    };
    let bitpix = samples.bitpix();
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;

    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = tile_count(total, &geom);
    let (gzip_level, scale) = (compression.gzip_level(), compression.hcompress_scale());
    let plio_offset = compress::plio_offset(bitpix.bitpix(), image.scaling);

    // Compress every tile independently (the compute-bound step — parallel under
    // the `parallel` feature). The heap layout is sequential (each descriptor's
    // offset is the running heap length), so concatenate the cells serially after.
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<TileCell> {
        geom.tile_into(t, &mut s.tile);
        // Gather + widen this tile's pixels straight from the typed source — no
        // whole-image `i64` buffer.
        convert::gather_i64(samples, &s.tile.row_bases, s.tile.row_len, &mut s.ints);
        if codec == ImageCodec::Plio1 && plio_offset != 0 {
            // A u16 image's stream holds the physical values (`compress::plio_offset`).
            for value in &mut s.ints {
                *value += plio_offset;
            }
        }
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
                let tile_scale = scale.tile_scale(vals, &s.tile.tdims, &mut s.quantize)?;
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

    let mut h = lay_out_cells(cells, codec, out)?;
    set_zimage_axes(&mut h, compression.name(), bitpix.bitpix(), dims, &tiles)?;
    match codec {
        ImageCodec::Rice1 => set_rice_parameters(&mut h, bitpix)?,
        ImageCodec::Hcompress1 => {
            h.set_internal(keywords::ZNAME.indexed(1)?, "SCALE")
                .set_internal(keywords::ZVAL.indexed(1)?, scale.keyword_value());
        }
        _ => {}
    }
    image.scaling.add_to_header(&mut h, bitpix.bitpix())?;
    Ok(h)
}

/// The tiles of an image of `sample_count` samples. A `NAXIS = 0` image has no data
/// array; `geom.ntiles()` would otherwise be the empty product (1) and fabricate a
/// phantom one-pixel tile that indexes the empty sample buffer. Zero tiles yields an
/// empty `NAXIS2 = 0` table, which the decoder's matching `ZNAXIS = 0` guard turns back
/// into the empty image.
fn tile_count(sample_count: usize, geom: &TileGeometry) -> usize {
    if sample_count == 0 { 0 } else { geom.ntiles() }
}

/// Lay out one `COMPRESSED_DATA` cell per tile into `out`: a descriptor for each tile,
/// then the heap, with 64-bit `Q` descriptors once the heap or a tile's element count
/// passes the 32-bit `P` range (§10.1.3). Returns the container's header up to its
/// one column.
fn lay_out_cells(cells: Vec<TileCell>, codec: ImageCodec, out: &mut Vec<u8>) -> Result<Header> {
    let ntiles = cells.len();
    let heap_len = cells.iter().try_fold(0usize, |len, cell| {
        len.checked_add(cell.byte_len()?)
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let maxnelem = cells.iter().map(TileCell::nelem).max().unwrap_or(0);
    let width = DescriptorWidth::addressing(heap_len, maxnelem);
    let descriptor_width = width.bytes();

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

/// A float image to compress, and its samples typed.
#[derive(Debug, Clone, Copy)]
struct FloatImage<'a> {
    image: Borrowed<'a>,
    samples: FloatSamples<'a>,
}

/// Encode a float image as a tiled-compressed `BINTABLE`, losslessly or quantized as
/// `config` selects, with descriptors as wide as its heap needs.
fn compress_float_image(
    float: FloatImage<'_>,
    config: &CompressionConfig,
    out: &mut Vec<u8>,
) -> Result<Header> {
    match config.quantization.unwrap_or_default().spacing() {
        Spacing::None => compress_lossless_float_image(float, config, out),
        Spacing::Quantized(levels) => {
            let encoded = QuantizedFloatImage::encode(float, config, levels)?;
            let width = encoded.addressing()?;
            encoded.lay_out(width, out)
        }
    }
}

/// Encode a float image losslessly (§10.2): each tile's big-endian floats gzip'd in
/// `COMPRESSED_DATA`, byte-shuffled for `GZIP_2`, with `ZQUANTIZ = 'NONE'` and no
/// `ZSCALE`/`ZZERO` column. cfitsio takes no other codec for it (`imcomp_init_table`).
fn compress_lossless_float_image(
    FloatImage { image, samples }: FloatImage<'_>,
    config: &CompressionConfig,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let compression = config.compression;
    let codec = compression.image_codec();
    if !matches!(codec, ImageCodec::Gzip1 | ImageCodec::Gzip2) {
        return Err(FitsError::UnsupportedCompression {
            name: format!("{} for a lossless float image", compression.name()),
        });
    }
    let zbitpix = samples.bitpix();
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;
    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = tile_count(image.samples.len(), &geom);
    let gzip_level = compression.gzip_level();
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<TileCell> {
        geom.tile_into(t, &mut s.tile);
        convert::gather_float_be(samples, &s.tile.row_bases, s.tile.row_len, &mut s.be);
        Ok(TileCell::Bytes(if codec == ImageCodec::Gzip2 {
            gzip::gzip2_encode(&s.be, zbitpix.elem_size(), gzip_level, &mut s.gzip)
        } else {
            gzip::gzip_encode(&s.be, gzip_level)
        }))
    })?;
    let mut h = lay_out_cells(cells, codec, out)?;
    set_zimage_axes(&mut h, compression.name(), zbitpix.bitpix(), dims, &tiles)?;
    h.set_internal(keywords::ZQUANTIZ, "NONE");
    image.scaling.add_to_header(&mut h, zbitpix.bitpix())?;
    Ok(h)
}

/// The integer codec a quantized float image's tiles are stored with.
#[derive(Debug, Clone, Copy)]
enum QuantizedCodec {
    Gzip1,
    Gzip2,
    Rice1,
    Hcompress1(HcompressScale),
    NoCompress,
}

impl QuantizedCodec {
    /// The codec `compression` names for quantized tiles. `PLIO_1` takes a quantized
    /// tile only within its 24-bit range, seldom met; `HCOMPRESS_1` cannot keep
    /// `SUBTRACTIVE_DITHER_2`'s exact zeros, which fpack refuses too (`-qz` with `-h`).
    fn of(compression: Compression, method: DitherMethod) -> Result<Self> {
        Ok(match compression.image_codec() {
            ImageCodec::Gzip1 => Self::Gzip1,
            ImageCodec::Gzip2 => Self::Gzip2,
            ImageCodec::Rice1 => Self::Rice1,
            ImageCodec::NoCompress => Self::NoCompress,
            ImageCodec::Hcompress1 if method == DitherMethod::Subtractive2 => {
                return Err(FitsError::InvalidCompressionParameter {
                    detail: "HCOMPRESS_1 cannot keep SUBTRACTIVE_DITHER_2's zeros".to_string(),
                });
            }
            ImageCodec::Hcompress1 => Self::Hcompress1(compression.hcompress_scale()),
            ImageCodec::Plio1 => {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("{} for float images (write)", compression.name()),
                });
            }
        })
    }

    /// Compress one tile's quantized integers, `has_null` when one is the null marker.
    fn encode(self, has_null: bool, gzip_level: u32, s: &mut EncodeScratch) -> Result<Vec<u8>> {
        Ok(match self {
            Self::Gzip1 => {
                convert::i32_to_be_into(&s.quantize.ints, &mut s.be);
                gzip::gzip_encode(&s.be, gzip_level)
            }
            Self::Gzip2 => {
                convert::i32_to_be_into(&s.quantize.ints, &mut s.be);
                gzip::gzip2_encode(&s.be, 4, gzip_level, &mut s.gzip)
            }
            Self::Rice1 => rice::rice_encode(
                &s.quantize.ints,
                IntBitpix::I32,
                rice::BLOCKSIZE,
                &mut s.rice,
            ),
            Self::Hcompress1(scale) => {
                s.ints.clear();
                s.ints
                    .extend(s.quantize.ints.iter().map(|&value| i64::from(value)));
                let tile_scale = scale.tile_scale(&s.ints, &s.tile.tdims, &mut s.quantize)?;
                if tile_scale > 1 && has_null {
                    return Err(FitsError::UnsupportedCompression {
                        name: "lossy HCOMPRESS_1 with undefined pixels requires a null mask"
                            .to_string(),
                    });
                }
                hcompress::hcompress_tile_encode(
                    &s.ints,
                    &s.tile.tdims,
                    tile_scale,
                    &mut s.hcompress,
                )?
            }
            Self::NoCompress => {
                let mut raw = Vec::new();
                convert::i32_to_be_into(&s.quantize.ints, &mut raw);
                raw
            }
        })
    }
}

/// A quantized float image's encoded tiles, with what its container header records.
#[derive(Debug)]
struct QuantizedFloatImage<'a> {
    tiles: Vec<FloatTile>,
    compression: Compression,
    codec: QuantizedCodec,
    zbitpix: FloatBitpix,
    shape: &'a [usize],
    tile_shape: Vec<usize>,
    method: DitherMethod,
    zdither0: Option<i64>,
    scaling: Scaling,
}

impl<'a> QuantizedFloatImage<'a> {
    /// Quantize each tile to int32 with its own `ZSCALE`/`ZZERO`, then compress it with
    /// the selected integer codec; a tile that can't be quantized (constant or all-null
    /// data) is stored as raw gzip'd floats for `GZIP_COMPRESSED_DATA`.
    fn encode(
        FloatImage { image, samples }: FloatImage<'a>,
        config: &CompressionConfig,
        Levels { step, dither }: Levels,
    ) -> Result<Self> {
        let compression = config.compression;
        let method = dither.method();
        let codec = QuantizedCodec::of(compression, method)?;
        let zbitpix = samples.bitpix();
        let shape = image.shape;
        let tile_shape = resolve_tile_shape(shape, &config.tiles, compression.image_codec())?;
        let geom = TileGeometry::new(shape, &tile_shape);
        let ntiles = tile_count(image.samples.len(), &geom);
        let zdither0 = dither.seed().map(|seed| match seed.source() {
            Seed::FromSamples => quantize::sample_seed(samples),
            Seed::Fixed(seed) => i64::from(seed),
        });
        let quantizer = Quantizer {
            precision: match zbitpix {
                FloatBitpix::F32 => Precision::Single,
                FloatBitpix::F64 => Precision::Double,
            },
            step,
            method,
            zdither0: zdither0.unwrap_or(1),
        };
        let gzip_level = compression.gzip_level();
        // Quantize + compress each tile independently (the compute-bound step —
        // parallel under the `parallel` feature); the heap is assembled serially after.
        let tiles = map_tiles(
            ntiles,
            EncodeScratch::default,
            |s, t| -> Result<FloatTile> {
                geom.tile_into(t, &mut s.tile);
                convert::gather_f64(samples, &s.tile.row_bases, s.tile.row_len, &mut s.floats);
                let tile = TileValues {
                    values: &s.floats,
                    width: s.tile.tdims[0],
                    index: t,
                };
                let Some(q) = quantizer.quantize(tile, &mut s.quantize) else {
                    convert::gather_float_be(samples, &s.tile.row_bases, s.tile.row_len, &mut s.be);
                    return Ok(FloatTile::Raw {
                        bytes: gzip::gzip_encode(&s.be, gzip_level),
                    });
                };
                Ok(FloatTile::Quantized {
                    bytes: codec.encode(q.has_null, gzip_level, s)?,
                    zscale: q.bscale,
                    zzero: q.bzero,
                    has_null: q.has_null,
                })
            },
        )?;
        Ok(Self {
            tiles,
            compression,
            codec,
            zbitpix,
            shape,
            tile_shape,
            method,
            zdither0,
            scaling: image.scaling,
        })
    }

    fn heap_len(&self) -> Result<usize> {
        self.tiles.iter().try_fold(0usize, |len, tile| {
            len.checked_add(tile.bytes().len())
                .ok_or(FitsError::DataUnitOverflow)
        })
    }

    /// The longest cell of the `quantized` or the raw tiles.
    fn longest_cell(&self, quantized: bool) -> usize {
        self.tiles
            .iter()
            .filter(|tile| matches!(tile, FloatTile::Quantized { .. }) == quantized)
            .map(|tile| tile.bytes().len())
            .max()
            .unwrap_or(0)
    }

    /// The narrowest descriptors that address this heap (§10.1.3).
    fn addressing(&self) -> Result<DescriptorWidth> {
        let longest = self.longest_cell(true).max(self.longest_cell(false));
        Ok(DescriptorWidth::addressing(self.heap_len()?, longest))
    }

    /// Lay out the container into `out` with `width` descriptors: for each tile the
    /// `COMPRESSED_DATA` and `GZIP_COMPRESSED_DATA` descriptors and the `ZSCALE`/`ZZERO`
    /// doubles, then the heap. Returns the container's header.
    fn lay_out(self, width: DescriptorWidth, out: &mut Vec<u8>) -> Result<Header> {
        let heap_len = self.heap_len()?;
        let (max_cd, max_gz) = (self.longest_cell(true), self.longest_cell(false));
        let any_null = self
            .tiles
            .iter()
            .any(|tile| matches!(tile, FloatTile::Quantized { has_null: true, .. }));
        let descriptor = width.bytes();
        let row_len = 2 * descriptor + 16;
        let ntiles = self.tiles.len();
        out.clear();
        let rows_len = ntiles
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let output_len = rows_len
            .checked_add(heap_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        out.reserve_exact(output_len);
        out.resize(rows_len, 0);
        for (tile_index, tile) in self.tiles.into_iter().enumerate() {
            let row = tile_index * row_len;
            let offset = (out.len() - rows_len) as u64;
            // A raw tile has no quantization to invert, so its `ZSCALE`/`ZZERO` are zero.
            let (mut bytes, zscale, zzero, quantized) = match tile {
                FloatTile::Quantized {
                    bytes,
                    zscale,
                    zzero,
                    ..
                } => (bytes, zscale, zzero, true),
                FloatTile::Raw { bytes } => (bytes, 0.0, 0.0, false),
            };
            let len = bytes.len() as u64;
            let (cd_count, gz_count) = if quantized { (len, 0) } else { (0, len) };
            write_pq_descriptor(&mut out[row..row + descriptor], width, cd_count, offset)?;
            write_pq_descriptor(
                &mut out[row + descriptor..row + 2 * descriptor],
                width,
                gz_count,
                offset,
            )?;
            let scales = row + 2 * descriptor;
            out[scales..scales + 8].copy_from_slice(&zscale.to_be_bytes());
            out[scales + 8..scales + 16].copy_from_slice(&zzero.to_be_bytes());
            out.append(&mut bytes);
        }

        let letter = width.letter();
        let mut h = container_header(row_len, ntiles, heap_len, 4)?;
        h.set_internal(keywords::TTYPE.indexed(1)?, "COMPRESSED_DATA")
            .set_internal(keywords::TFORM.indexed(1)?, format!("1{letter}B({max_cd})"));
        h.set_internal(keywords::TTYPE.indexed(2)?, "GZIP_COMPRESSED_DATA")
            .set_internal(keywords::TFORM.indexed(2)?, format!("1{letter}B({max_gz})"));
        h.set_internal(keywords::TTYPE.indexed(3)?, "ZSCALE")
            .set_internal(keywords::TFORM.indexed(3)?, "1D");
        h.set_internal(keywords::TTYPE.indexed(4)?, "ZZERO")
            .set_internal(keywords::TFORM.indexed(4)?, "1D");
        let zbitpix = self.zbitpix.bitpix();
        set_zimage_axes(
            &mut h,
            self.compression.name(),
            zbitpix,
            self.shape,
            &self.tile_shape,
        )?;
        match self.codec {
            QuantizedCodec::Rice1 => set_rice_parameters(&mut h, IntBitpix::I32)?,
            QuantizedCodec::Hcompress1(scale) => {
                h.set_internal(keywords::ZNAME.indexed(1)?, "SCALE")
                    .set_internal(keywords::ZVAL.indexed(1)?, scale.keyword_value());
            }
            QuantizedCodec::Gzip1 | QuantizedCodec::Gzip2 | QuantizedCodec::NoCompress => {}
        }
        h.set_internal(keywords::ZQUANTIZ, self.method.name());
        if let Some(zdither0) = self.zdither0 {
            h.set_internal(keywords::ZDITHER0, zdither0);
        }
        if any_null {
            // Quantized nulls are stored as this reserved integer; ZBLANK tells the
            // decoder which value maps back to a blank (NaN) pixel.
            h.set_internal(keywords::ZBLANK, i64::from(quantize::NULL_VALUE));
        }
        self.scaling.add_to_header(&mut h, zbitpix)?;
        Ok(h)
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
