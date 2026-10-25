//! Tiled image+table compression (§10) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module holds the shared pieces — the write-time [`Compression`] and
//! [`CompressionConfig`], the [`tile_codec::TileCodec`] dispatch, and the
//! per-tile [`map_tiles`] fan-out — while the directions live in [`decode`]
//! ([`TiledImage::decode_into`]) and [`encode`] ([`encode::compress_image`], all five codecs:
//! `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` with `SMOOTH=1` decode).
//! Float images are quantized per-tile (`ZSCALE`/`ZZERO`) with `NO_DITHER`,
//! `SUBTRACTIVE_DITHER_1`, or `SUBTRACTIVE_DITHER_2`. The per-codec work lives in
//! [`gzip`], [`rice`], [`plio`], and [`hcompress`]; tiled *table* compression
//! (§10.3) lives in [`table`] ([`table::compress_table`] and [`table::TiledTable`]).
//!
//! [`TiledImage::decode_into`]: decode::tiled_image::TiledImage::decode_into

#[cfg(feature = "internals")]
pub(crate) mod bench;
mod convert;
pub(crate) mod decode;
pub(crate) mod encode;
pub(crate) mod encoded_unit;
#[cfg(feature = "parallel")]
mod first_failure;
pub(crate) mod gzip;
mod hcompress;
mod nocompress;
mod plane;
mod plio;
pub(crate) mod quantization;
mod quantize;
mod rice;
pub(crate) mod table;
mod tile_codec;
mod tile_geometry;

use crate::compression::hcompress::Smoothing;
use crate::compression::plane::IntBitpix;
use crate::compression::quantization::{Quantization, Spacing};
use crate::compression::tile_codec::TileCodec;
use crate::fits_error::CompressionParameter;
use crate::fits_error::FitsError;
use crate::fits_error::Ranked;
use crate::fits_error::Result;
use crate::header_model::keywords;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::scaling::Scaling;
use crate::keyword::Keyword;

/// Validated GZIP configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gzip {
    form: GzipForm,
    level: u32,
}

/// Whether a gzip codec shuffles its bytes first: `GZIP_1` or `GZIP_2`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GzipForm {
    Plain,
    Shuffled,
}

impl Gzip {
    /// Standard `GZIP_1` at a validated deflate level.
    pub fn new(level: u32) -> Result<Self> {
        Ok(Self {
            form: GzipForm::Plain,
            level: Self::checked_level(level)?,
        })
    }

    /// Byte-shuffled `GZIP_2` at a validated deflate level.
    pub fn shuffled(level: u32) -> Result<Self> {
        Ok(Self {
            form: GzipForm::Shuffled,
            level: Self::checked_level(level)?,
        })
    }

    const fn checked_level(level: u32) -> Result<u32> {
        if level > 9 {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::GzipLevel { level },
            });
        }
        Ok(level)
    }
}

impl Default for Gzip {
    fn default() -> Self {
        Self {
            form: GzipForm::Plain,
            level: gzip::DEFAULT_GZIP_LEVEL,
        }
    }
}

/// Validated HCOMPRESS configuration; the default is lossless.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hcompress {
    scale: HcompressScale,
}

/// The HCOMPRESS `SCALE` (§10.4.4): the step its coefficients are rounded to.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
enum HcompressScale {
    #[default]
    Lossless,
    /// A multiple of each tile's background-noise estimate.
    Relative(f64),
    /// One step for every tile.
    Absolute(f64),
}

impl Hcompress {
    /// Lossy HCOMPRESS whose scale is `scale` times each tile's background noise, a
    /// finite positive number, as fpack's `-s scale`.
    pub fn relative(scale: f64) -> Result<Self> {
        Ok(Self {
            scale: HcompressScale::Relative(Self::positive(scale)?),
        })
    }

    /// Lossy HCOMPRESS with one scale `scale` for every tile, a finite positive
    /// number, as fpack's `-s -scale`.
    pub fn absolute(scale: f64) -> Result<Self> {
        Ok(Self {
            scale: HcompressScale::Absolute(Self::positive(scale)?),
        })
    }

    fn positive(scale: f64) -> Result<f64> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::HcompressScale { scale },
            });
        }
        Ok(scale)
    }
}

impl HcompressScale {
    /// The `ZVAL1` cfitsio writes for `ZNAME1 = 'SCALE'`: an absolute scale negated.
    const fn keyword_value(self) -> f64 {
        match self {
            Self::Lossless => 0.0,
            Self::Relative(scale) => scale,
            Self::Absolute(scale) => -scale,
        }
    }
}

/// Typed tiled-compression choice. Codec-specific configuration lives only on the
/// variant that consumes it.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Compression {
    Gzip(Gzip),
    Rice,
    Plio,
    Hcompress(Hcompress),
    None,
}

impl Compression {
    pub const GZIP: Self = Self::Gzip(Gzip {
        form: GzipForm::Plain,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });
    pub const GZIP_SHUFFLED: Self = Self::Gzip(Gzip {
        form: GzipForm::Shuffled,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });

    /// The tile codec of this choice for samples whose integer type is `bytepix`, the
    /// one `RICE_1` writes, and whose `PLIO_1` stream holds each plus `plio_offset`.
    const fn tile_codec(self, bytepix: IntBitpix, plio_offset: i64) -> TileCodec {
        match self {
            Self::Gzip(Gzip {
                form: GzipForm::Plain,
                level,
            }) => TileCodec::Gzip1 { level },
            Self::Gzip(Gzip {
                form: GzipForm::Shuffled,
                level,
            }) => TileCodec::Gzip2 { level },
            Self::Rice => TileCodec::Rice(rice::Codec {
                blocksize: rice::BLOCKSIZE,
                bytepix,
            }),
            Self::Plio => TileCodec::Plio(plio::Codec {
                offset: plio_offset,
            }),
            Self::Hcompress(Hcompress { scale }) => TileCodec::Hcompress(hcompress::Codec {
                scale,
                smoothing: Smoothing::Off,
                plane: hcompress::Codec::PLANE_2D,
            }),
            Self::None => TileCodec::NoCompress,
        }
    }

    /// FITS `ZCMPTYPE` name written for this choice.
    pub const fn name(self) -> &'static str {
        self.tile_codec(IntBitpix::I32, 0).name()
    }
}

/// A tiled-compression write: the codec, the tiles, and a float image's quantization.
#[derive(Debug, Clone, PartialEq)]
pub struct CompressionConfig {
    compression: Compression,
    /// Fastest axis first for an image; one axis, the rows, for a table. Empty means
    /// the default: for an image one tile per row, or for `HCOMPRESS_1` cfitsio's
    /// default (full rows, one band up to 30 rows, else bands of 16 rows unless that
    /// leaves a last band of 1 to 3, which moves to cfitsio's next candidate height);
    /// for a table cfitsio's tiles of about [`DEFAULT_TABLE_TILE_BYTES`].
    tiles: Vec<usize>,
    /// `None` until set, which a float image reads as [`Quantization::default`]. An
    /// integer image and a table refuse a set level.
    quantization: Option<Quantization>,
}

/// cfitsio's `maxchunksize`, the bytes of rows in one tile of a table compressed with
/// no tile height set (`fits_compress_table`).
const DEFAULT_TABLE_TILE_BYTES: usize = 10_000_000;

impl CompressionConfig {
    pub const fn new(compression: Compression) -> Self {
        Self {
            compression,
            tiles: Vec::new(),
            quantization: None,
        }
    }

    /// The tile shape: one length per image axis, fastest first, or one row count for
    /// a table.
    #[must_use]
    pub fn with_tiles(mut self, tiles: impl Into<Vec<usize>>) -> Self {
        self.tiles = tiles.into();
        self
    }

    /// A float image's quantization; [`Quantization::default`] when it is not set. An
    /// integer image's and a table's write refuse a level, and take [`Quantization::NONE`],
    /// as they keep their values.
    #[must_use]
    pub const fn with_quantization(mut self, quantization: Quantization) -> Self {
        self.quantization = Some(quantization);
        self
    }

    /// Whether a quantization level is set, relative or absolute: what an integer image
    /// and a table refuse. Unset, the config quantizes a float image alone.
    fn quantizes(&self) -> bool {
        self.quantization
            .is_some_and(|quantization| matches!(quantization.spacing(), Spacing::Quantized(_)))
    }

    /// Rows per tile of a table of `row_count` rows of `row_len` bytes, in `1..=row_count`.
    /// Errors when the tiles have more than one axis or the config quantizes.
    fn table_tile_rows(&self, row_count: usize, row_len: usize) -> Result<usize> {
        if self.quantizes() {
            return Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::TableQuantization,
            });
        }
        let rows = match *self.tiles.as_slice() {
            [] => DEFAULT_TABLE_TILE_BYTES
                .checked_div(row_len)
                .unwrap_or(row_count),
            [0] => {
                return Err(FitsError::InvalidCompressionParameter {
                    parameter: CompressionParameter::EmptyTile { axis: 0 },
                });
            }
            [rows] => rows,
            ref tiles => {
                return Err(FitsError::RankMismatch {
                    ranked: Ranked::TileShape,
                    expected: 1,
                    got: tiles.len(),
                });
            }
        };
        Ok(rows.clamp(1, row_count.max(1)))
    }
}

/// The offset between a `PLIO_1` stream's values and an image's stored values: 32768
/// for a u16 image (`BITPIX = 16`, `BZERO = 32768`), whose stream holds the physical
/// values, as cfitsio writes and reads it (`imcompress.c`, `imcomp_convert_tile_tshort`
/// and `imcomp_decompress_tile`); 0 for any other image.
fn plio_offset(bitpix: Bitpix, scaling: Scaling) -> i64 {
    if bitpix == Bitpix::I16 && scaling.bzero == 32768.0 {
        32768
    } else {
        0
    }
}

fn parameter_index(keyword: Keyword) -> Option<usize> {
    keyword.index_after(keywords::ZNAME)
}

/// Map `f` over each tile index `0..ntiles`, collecting the per-tile results in
/// tile order and short-circuiting on the first error. `init` builds the reusable
/// per-worker scratch `f` writes through (one per thread under `parallel`, reused
/// across that worker's tiles; a single one serially).
///
/// Tiles (de)compress independently and the codecs are compute-bound, so with the
/// `parallel` feature this fans the per-tile work across the rayon pool for a
/// near-linear speedup. The caller then folds the ordered results through safe
/// slices; heap concatenation stays serial, while image scatter may partition the
/// destination into disjoint rows.
#[cfg(feature = "parallel")]
fn map_tiles<S, T, I, F>(ntiles: usize, init: I, f: F) -> Result<Vec<T>>
where
    S: Send,
    T: Send,
    I: Fn() -> S + Sync + Send,
    F: Fn(&mut S, usize) -> Result<T> + Sync + Send,
{
    first_failure::try_map_init(0..ntiles, init, f)
}

#[cfg(not(feature = "parallel"))]
fn map_tiles<S, T, I, F>(ntiles: usize, init: I, f: F) -> Result<Vec<T>>
where
    I: FnOnce() -> S,
    F: Fn(&mut S, usize) -> Result<T>,
{
    let mut scratch = init();
    (0..ntiles).map(|t| f(&mut scratch, t)).collect()
}

/// The synthetic planes the codec fixtures in `test_resources/comp_*` encode.
/// The check of a quantized read that tests and benches share.
#[cfg(feature = "internals")]
pub(crate) mod half_step {
    use crate::image_model::bitpix::Bitpix;
    use crate::internals::tolerance::{Tolerance, assert_within};

    /// Assert that `back` is within half the quantization step `zscale` of `original`,
    /// plus the roundings of the dither arithmetic and of storing a sample of `bitpix`: an
    /// ulp of an f32, or four roundings of f64 arithmetic, relative to the sample.
    #[track_caller]
    pub(crate) fn assert_within_half_step(
        original: f64,
        back: f64,
        zscale: f64,
        bitpix: Bitpix,
        context: &str,
    ) {
        let rounding = match bitpix {
            Bitpix::F32 => f64::from(f32::EPSILON),
            _ => 4.0 * f64::EPSILON,
        };
        assert_within(
            back,
            original,
            Tolerance::abs(
                0.5 * zscale + rounding * original.abs(),
                "half a quantization step, and the roundings of the dither and the stored sample",
            ),
            context,
        );
    }
}

#[cfg(test)]
pub(crate) mod internals {
    use crate::bintable::Table;
    use crate::compression::decode::tiled_image::TiledImage;
    use crate::fits_error::Result;
    use crate::hdu::hdu_kind::{HduKind, HduPosition};
    use crate::hdu::image_geometry::ImageGeometry;
    use crate::header_model::Header;
    use crate::image_model::Image;

    /// Decode the tiled image `header` describes from its container `table`, resolving
    /// the kind and geometry the way the reader's scan does.
    pub(crate) fn decompress_image(header: &Header, table: &Table) -> Result<Image> {
        let kind = HduKind::from_header(header, HduPosition::Extension)?;
        let image = ImageGeometry::from_header(header, kind)?;
        let samples = TiledImage::new(header, &image)?.decode_table(table)?;
        Image::new(image.shape.clone(), samples, image.scaling)
    }

    /// value(x, y) = 7x − 5y over 24×16, row-major: the integer codec fixtures.
    pub(crate) fn fixture_plane() -> Vec<i16> {
        (0..24 * 16)
            .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
            .collect()
    }

    /// value(x, y) = (x + y) mod 7 over 24×16: the PLIO fixture, a non-negative mask.
    pub(crate) fn fixture_mask() -> Vec<i32> {
        (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect()
    }

    /// `count` values of `(7919·i + 13) mod 2001 − 1000`: rough data whose HCOMPRESS
    /// transform has odd halves at every level of a 17×17 tile.
    pub(crate) fn odd_plane(count: usize) -> Vec<i16> {
        (0..count)
            .map(|i| i16::try_from((i * 7919 + 13) % 2001).unwrap() - 1000)
            .collect()
    }
}

#[cfg(test)]
mod tests;
