//! Tiled-image compression (§10.1).
//!
//! Tile, (quantize then) compress, and lay out the `ZIMAGE` `BINTABLE` data unit
//! (per-tile array descriptors + a heap of compressed tile bytes). Integer images
//! go straight through the codec ([`compress_image`]); float images are per-tile
//! quantized to int32 with a `ZSCALE`/`ZZERO` first ([`compress_float_image`]). The
//! per-codec work lives in the sibling codec modules.

use crate::allocation;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::compression;
use crate::compression::Compression;
use crate::compression::convert;
use crate::compression::encoded_unit::EncodedUnit;
use crate::compression::hcompress;
use crate::compression::plane::{FloatBitpix, FloatSamples, IntBitpix, Samples};
use crate::compression::quantization::{DitherMethod, Levels, Seed, Spacing};
use crate::compression::quantize::{Precision, Quantizer, TileValues};
use crate::compression::rice;
use crate::compression::tile_codec::{CodecScratch, Tile, TileCodec, TileSamples};
use crate::compression::tile_geometry::TileGeometry;
use crate::compression::tile_geometry::TileScratch;
use crate::compression::{CompressionConfig, map_tiles};
use crate::compression::{gzip, quantize};
use crate::fits_error::KeywordDefect;
use crate::hdu::prologue::{Opening, Prologue, Xtension};
use crate::header_model::{Header, keywords, value};
use crate::image_model::bitpix::Bitpix;

use crate::column::TableShape;
use crate::fits_error::FitsError;
use crate::fits_error::Ranked;
use crate::fits_error::Result;
use crate::fits_error::{CompressionFeature, CompressionParameter};
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
    /// A float tile's pixels as big-endian bytes.
    be: Vec<u8>,
    quantize: quantize::QuantizeScratch,
    codec: CodecScratch,
}

/// Encode an image as a tiled-compressed `BINTABLE`: its `ZIMAGE` header and its data
/// unit, the per-tile descriptors and the heap of compressed tiles. A float image goes
/// through [`compress_float_image`].
pub(crate) fn compress_image(
    image: Borrowed<'_>,
    config: &CompressionConfig,
) -> Result<EncodedUnit> {
    if image.shape.len() > 999 {
        return Err(FitsError::InvalidKeywordValue {
            keyword: keywords::NAXIS,
            defect: KeywordDefect::Range,
        });
    }
    let total = image.samples.len();
    let compression = config.compression;
    let samples = match Samples::of(image.samples) {
        Samples::Float(samples) => {
            return compress_float_image(FloatImage { image, samples }, config);
        }
        Samples::Int(_) if config.quantizes() => {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::IntegerQuantization,
            });
        }
        Samples::Int(samples) => samples,
    };
    let bitpix = samples.bitpix();
    let codec = compression.tile_codec(
        bitpix,
        compression::plio_offset(bitpix.bitpix(), image.scaling),
    );
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;
    let codec = codec.fitted(dims, &tiles)?;
    if let TileCodec::Plio(plio) = codec {
        plio.check(samples)?;
    }

    let geom = TileGeometry::new(dims, tiles.as_slice());
    let ntiles = tile_count(total, &geom);

    // Compress every tile independently (the compute-bound step — parallel under
    // the `parallel` feature). The heap layout is sequential (each descriptor's
    // offset is the running heap length), so concatenate the cells serially after.
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<Vec<u8>> {
        geom.tile_into(t, &mut s.tile);
        // Gather + widen this tile's pixels straight from the typed source — no
        // whole-image `i64` buffer.
        convert::gather_i64(samples, &s.tile.row_bases, s.tile.row_len, &mut s.ints);
        let tile = Tile {
            shape: &s.tile.tdims,
            width: bitpix,
            blank: image.scaling.blank,
        };
        Ok(codec
            .encode(tile, TileSamples::Ints(&s.ints), &mut s.codec)?
            .to_vec())
    })?;

    let mut unit = lay_out_cells(cells, codec)?;
    let h = &mut unit.header;
    set_zimage_axes(h, codec.name(), bitpix.bitpix(), dims, &tiles)?;
    set_codec_parameters(h, codec)?;
    image.scaling.add_to_header(h, bitpix.bitpix())?;
    Ok(unit)
}

/// The tiles of an image of `sample_count` samples: none for an image of no samples,
/// whose shape the grid does not count over, so a shape whose product overflows makes no
/// tile list. Zero tiles yields an empty `NAXIS2 = 0` table, which the decoder turns back
/// into the empty image.
fn tile_count(sample_count: usize, geom: &TileGeometry) -> usize {
    if sample_count == 0 { 0 } else { geom.ntiles() }
}

/// Lay out one `COMPRESSED_DATA` cell per tile: a descriptor for each tile, its offset
/// the running length of the cells before it, then the cells themselves as the heap,
/// with 64-bit `Q` descriptors once the heap or a tile's element count passes the
/// 32-bit `P` range (§10.1.3). The container's header holds its one column.
fn lay_out_cells(cells: Vec<Vec<u8>>, codec: TileCodec) -> Result<EncodedUnit> {
    let ntiles = cells.len();
    let heap_len = cells.iter().try_fold(0usize, |len, cell| {
        len.checked_add(cell.len())
            .ok_or(FitsError::DataUnitOverflow)
    })?;
    let kind = codec.stream_kind();
    let element = kind.elem_size();
    let maxnelem = cells
        .iter()
        .map(|cell| cell.len() / element)
        .max()
        .unwrap_or(0);
    let width = DescriptorWidth::addressing(heap_len, maxnelem);
    let descriptor_width = width.bytes();

    let descriptor_len = ntiles
        .checked_mul(descriptor_width)
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut rows = allocation::try_zeroed(0u8, descriptor_len)?;
    let mut offset = 0;
    for (slot, cell) in rows.chunks_exact_mut(descriptor_width).zip(&cells) {
        PqDescriptor {
            count: cell.len() / element,
            offset,
        }
        .write(width, slot)?;
        offset += cell.len();
    }

    let shape = TableShape {
        row_len: descriptor_width,
        row_count: ntiles,
        field_count: 1,
    };
    let mut header = container_header(shape, heap_len)?;
    header.set_internal(keywords::TTYPE.indexed(1)?, "COMPRESSED_DATA");
    header.set_internal(
        keywords::TFORM.indexed(1)?,
        format!("1{}{}({maxnelem})", width.letter(), kind.code()),
    );
    Ok(EncodedUnit {
        header,
        rows,
        heap: cells,
    })
}

/// The `BINTABLE` header of a compressed image's container of `shape`, a row a tile, and
/// a heap of `heap_len` bytes, up to its columns.
fn container_header(shape: TableShape, heap_len: usize) -> Result<Header> {
    let mut h = Prologue {
        opening: Opening::Extension(Xtension::BinTable),
        bitpix: Bitpix::U8,
        axes: &[shape.row_len, shape.row_count],
        pcount: heap_len,
    }
    .header()?;
    h.set_internal(keywords::TFIELDS, value::fits_i64(shape.field_count)?);
    Ok(h)
}

/// The `ZNAMEi`/`ZVALi` parameters of `codec` (§10.4.1 Table 37): the block size and
/// pixel width of `RICE_1`, and the `SCALE` of `HCOMPRESS_1`.
fn set_codec_parameters(h: &mut Header, codec: TileCodec) -> Result<()> {
    match codec {
        TileCodec::Rice(rice::Codec { blocksize, bytepix }) => {
            h.set_internal(keywords::ZNAME.indexed(1)?, "BLOCKSIZE")
                .set_internal(keywords::ZVAL.indexed(1)?, value::fits_i64(blocksize)?);
            h.set_internal(keywords::ZNAME.indexed(2)?, "BYTEPIX")
                .set_internal(
                    keywords::ZVAL.indexed(2)?,
                    value::fits_i64(bytepix.elem_size())?,
                );
        }
        TileCodec::Hcompress(hcompress::Codec { scale, .. }) => {
            h.set_internal(keywords::ZNAME.indexed(1)?, "SCALE")
                .set_internal(keywords::ZVAL.indexed(1)?, scale.keyword_value());
        }
        TileCodec::Gzip1 { .. }
        | TileCodec::Gzip2 { .. }
        | TileCodec::Plio(_)
        | TileCodec::NoCompress => {}
    }
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
fn compress_float_image(float: FloatImage<'_>, config: &CompressionConfig) -> Result<EncodedUnit> {
    match config.quantization.unwrap_or_default().spacing() {
        Spacing::None => compress_lossless_float_image(float, config),
        Spacing::Quantized(levels) => {
            let encoded = QuantizedFloatImage::encode(float, config, levels)?;
            let width = encoded.addressing()?;
            encoded.lay_out(width)
        }
    }
}

/// Encode a float image losslessly (§10.2): each tile's big-endian floats gzip'd in
/// `COMPRESSED_DATA`, byte-shuffled for `GZIP_2`, with `ZQUANTIZ = 'NONE'` and no
/// `ZSCALE`/`ZZERO` column. cfitsio takes no other codec for it (`imcomp_init_table`).
fn compress_lossless_float_image(
    FloatImage { image, samples }: FloatImage<'_>,
    config: &CompressionConfig,
) -> Result<EncodedUnit> {
    let zbitpix = samples.bitpix();
    let width = zbitpix.integer_width();
    let codec = config.compression.tile_codec(width, 0);
    if !codec.stores_lossless_floats() {
        return Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::LosslessFloatCodec {
                codec: codec.name(),
            },
        });
    }
    let dims = image.shape;
    let tiles = resolve_tile_shape(dims, &config.tiles, codec)?;
    let codec = codec.fitted(dims, &tiles)?;
    let geom = TileGeometry::new(dims, tiles.as_slice());
    let ntiles = tile_count(image.samples.len(), &geom);
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<Vec<u8>> {
        geom.tile_into(t, &mut s.tile);
        convert::gather_float_be(samples, &s.tile.row_bases, s.tile.row_len, &mut s.be);
        let tile = Tile {
            shape: &s.tile.tdims,
            width,
            blank: None,
        };
        Ok(codec
            .encode(tile, TileSamples::Bytes(&s.be), &mut s.codec)?
            .to_vec())
    })?;
    let mut unit = lay_out_cells(cells, codec)?;
    let h = &mut unit.header;
    set_zimage_axes(h, codec.name(), zbitpix.bitpix(), dims, &tiles)?;
    h.set_internal(keywords::ZQUANTIZ, "NONE");
    image.scaling.add_to_header(h, zbitpix.bitpix())?;
    Ok(unit)
}

/// The integer codec a quantized float image's tiles are stored with: any but
/// `PLIO_1`, which takes a quantized tile only within its 24-bit range, seldom met; and
/// `HCOMPRESS_1` not with `SUBTRACTIVE_DITHER_2`, whose exact zeros it cannot keep, as
/// fpack refuses it too (`-qz` with `-h`).
fn quantized_codec(compression: Compression, method: DitherMethod) -> Result<TileCodec> {
    let codec = compression.tile_codec(IntBitpix::I32, 0);
    match codec {
        TileCodec::Plio(_) => Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::QuantizedFloatCodec {
                codec: codec.name(),
            },
        }),
        TileCodec::Hcompress(_) if method == DitherMethod::Subtractive2 => {
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::HcompressDither2,
            })
        }
        _ => Ok(codec),
    }
}

/// A quantized float image's encoded tiles, with what its container header records.
#[derive(Debug)]
struct QuantizedFloatImage<'a> {
    tiles: Vec<FloatTile>,
    codec: TileCodec,
    zbitpix: FloatBitpix,
    shape: &'a [usize],
    tile_shape: Vec<usize>,
    method: DitherMethod,
    zdither0: Option<i64>,
    scaling: Scaling,
}

/// The longest `COMPRESSED_DATA` cell, of the quantized tiles, and the longest
/// `GZIP_COMPRESSED_DATA` cell, of the raw ones, which size their columns' `TFORMn`.
#[derive(Debug, Clone, Copy, Default)]
struct LongestCells {
    quantized: usize,
    raw: usize,
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
        let method = dither.method();
        let zbitpix = samples.bitpix();
        let shape = image.shape;
        let codec = quantized_codec(config.compression, method)?;
        let tile_shape = resolve_tile_shape(shape, &config.tiles, codec)?;
        let codec = codec.fitted(shape, &tile_shape)?;
        let geom = TileGeometry::new(shape, tile_shape.as_slice());
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
        // A tile that does not quantize keeps its floats, gzip'd at the codec's level.
        let raw_codec = TileCodec::Gzip1 {
            level: match codec {
                TileCodec::Gzip1 { level } | TileCodec::Gzip2 { level } => level,
                _ => gzip::DEFAULT_GZIP_LEVEL,
            },
        };
        let width = zbitpix.integer_width();
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
                    let tile = Tile {
                        shape: &s.tile.tdims,
                        width,
                        blank: None,
                    };
                    let bytes = raw_codec
                        .encode(tile, TileSamples::Bytes(&s.be), &mut s.codec)?
                        .to_vec();
                    return Ok(FloatTile::Raw { bytes });
                };
                s.ints.clear();
                s.ints
                    .extend(s.quantize.ints.iter().map(|&value| i64::from(value)));
                let tile = Tile {
                    shape: &s.tile.tdims,
                    width: IntBitpix::I32,
                    blank: q.has_null.then_some(i64::from(quantize::NULL_VALUE)),
                };
                let bytes = codec
                    .encode(tile, TileSamples::Ints(&s.ints), &mut s.codec)?
                    .to_vec();
                Ok(FloatTile::Quantized {
                    bytes,
                    zscale: q.bscale,
                    zzero: q.bzero,
                    has_null: q.has_null,
                })
            },
        )?;
        Ok(Self {
            tiles,
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

    /// The longest cell of the quantized tiles and of the raw ones.
    fn longest_cells(&self) -> LongestCells {
        self.tiles
            .iter()
            .fold(LongestCells::default(), |longest, tile| match tile {
                FloatTile::Quantized { .. } => LongestCells {
                    quantized: longest.quantized.max(tile.bytes().len()),
                    ..longest
                },
                FloatTile::Raw { .. } => LongestCells {
                    raw: longest.raw.max(tile.bytes().len()),
                    ..longest
                },
            })
    }

    /// The narrowest descriptors that address this heap (§10.1.3).
    fn addressing(&self) -> Result<DescriptorWidth> {
        let longest = self.longest_cells();
        let longest = longest.quantized.max(longest.raw);
        Ok(DescriptorWidth::addressing(self.heap_len()?, longest))
    }

    /// Lay out the container with `width` descriptors: for each tile the
    /// `COMPRESSED_DATA` and `GZIP_COMPRESSED_DATA` descriptors and the `ZSCALE`/`ZZERO`
    /// doubles, then the tiles as the heap.
    fn lay_out(self, width: DescriptorWidth) -> Result<EncodedUnit> {
        let heap_len = self.heap_len()?;
        let LongestCells {
            quantized: max_cd,
            raw: max_gz,
        } = self.longest_cells();
        let any_null = self
            .tiles
            .iter()
            .any(|tile| matches!(tile, FloatTile::Quantized { has_null: true, .. }));
        let descriptor = width.bytes();
        let row_len = 2 * descriptor + 16;
        let ntiles = self.tiles.len();
        let rows_len = ntiles
            .checked_mul(row_len)
            .ok_or(FitsError::DataUnitOverflow)?;
        let mut rows = allocation::try_zeroed(0u8, rows_len)?;
        let mut heap = Vec::with_capacity(ntiles);
        let mut offset = 0;
        for (row, tile) in rows.chunks_exact_mut(row_len).zip(self.tiles) {
            // A raw tile has no quantization to invert, so its `ZSCALE`/`ZZERO` are zero.
            let (bytes, zscale, zzero, quantized) = match tile {
                FloatTile::Quantized {
                    bytes,
                    zscale,
                    zzero,
                    ..
                } => (bytes, zscale, zzero, true),
                FloatTile::Raw { bytes } => (bytes, 0.0, 0.0, false),
            };
            let len = bytes.len();
            let (cd_count, gz_count) = if quantized { (len, 0) } else { (0, len) };
            let (descriptors, scales) = row.split_at_mut(2 * descriptor);
            let (cd, gz) = descriptors.split_at_mut(descriptor);
            PqDescriptor {
                count: cd_count,
                offset,
            }
            .write(width, cd)?;
            PqDescriptor {
                count: gz_count,
                offset,
            }
            .write(width, gz)?;
            scales[..8].copy_from_slice(&zscale.to_be_bytes());
            scales[8..].copy_from_slice(&zzero.to_be_bytes());
            offset += len;
            heap.push(bytes);
        }

        let letter = width.letter();
        let shape = TableShape {
            row_len,
            row_count: ntiles,
            field_count: 4,
        };
        let mut h = container_header(shape, heap_len)?;
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
            self.codec.name(),
            zbitpix,
            self.shape,
            &self.tile_shape,
        )?;
        set_codec_parameters(&mut h, self.codec)?;
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
        Ok(EncodedUnit {
            header: h,
            rows,
            heap,
        })
    }
}

/// Resolve the tile shape for an image: an empty `tile_shape` defaults to row-tiling
/// (the first axis whole, the rest 1), or for `HCOMPRESS_1` to cfitsio's default (see
/// [`hcompress_default_rows`]); a non-empty one is used as given (each axis clamped to
/// ≥1) and must match the image rank, else [`FitsError::RankMismatch`]. An
/// `HCOMPRESS_1` tiling with a side of its plane under 4 pixels, the last tile's
/// included, is refused rather than adjusted; an axis of extent 1 lies outside the plane.
fn resolve_tile_shape(
    dims: &[usize],
    tile_shape: &[usize],
    codec: TileCodec,
) -> Result<Vec<usize>> {
    let hcompress = matches!(codec, TileCodec::Hcompress(_));
    let tiles: Vec<usize> = if tile_shape.is_empty() {
        (0..dims.len())
            .map(|i| match i {
                0 => dims[i].max(1),
                1 if hcompress => hcompress_default_rows(dims[1]).max(1),
                _ => 1,
            })
            .collect()
    } else if tile_shape.len() == dims.len() {
        if let Some(axis) = tile_shape.iter().position(|&tile| tile == 0) {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::EmptyTile { axis },
            });
        }
        tile_shape.to_vec()
    } else {
        return Err(FitsError::RankMismatch {
            ranked: Ranked::TileShape,
            expected: dims.len(),
            got: tile_shape.len(),
        });
    };
    // An empty image has no tiles to refuse.
    if hcompress && !dims.contains(&0) {
        for (axis, (&length, &tile)) in dims.iter().zip(&tiles).enumerate() {
            let tile = tile.min(length);
            if tile == 1 {
                continue;
            }
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
