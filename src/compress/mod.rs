//! Tiled image+table compression (§10) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module holds the shared pieces — the write-time [`Compression`] and
//! [`CompressionConfig`], the
//! [`ImageCodec`] dispatch, the per-tile [`map_tiles`] fan-out, and the `P`-vs-`Q`
//! descriptor threshold — while the directions live in [`decode`]
//! ([`TiledImage::decode`]) and [`encode`] ([`encode::compress_image`], all five codecs:
//! `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` with `SMOOTH=1` decode).
//! Float images are quantized per-tile (`ZSCALE`/`ZZERO`) with `NO_DITHER`,
//! `SUBTRACTIVE_DITHER_1`, or `SUBTRACTIVE_DITHER_2`. The per-codec work lives in
//! [`gzip`], [`rice`], [`plio`], and [`hcompress`]; tiled *table* compression
//! (§10.3) lives in [`table`] ([`table::compress_table`] and [`table::TiledTable`]).
//!
//! [`TiledImage::decode`]: decode::tiled_image::TiledImage::decode

#[cfg(feature = "internals")]
pub(crate) mod bench;
mod convert;
pub(crate) mod decode;
pub(crate) mod encode;
mod gzip;
mod hcompress;
mod plane;
mod plio;
pub(crate) mod quantization;
mod quantize;
mod rice;
pub(crate) mod table;
mod tile_geometry;

use crate::bitpix::Bitpix;
use crate::compress::quantization::Quantization;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::error::Result;
use crate::image_model::scaling::Scaling;
use crate::keyword;

/// Validated GZIP configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gzip {
    shuffle: bool,
    level: u32,
}

impl Gzip {
    /// Standard `GZIP_1` at a validated deflate level.
    pub fn new(level: u32) -> Result<Self> {
        Ok(Self {
            shuffle: false,
            level: Self::checked_level(level)?,
        })
    }

    /// Byte-shuffled `GZIP_2` at a validated deflate level.
    pub fn shuffled(level: u32) -> Result<Self> {
        Ok(Self {
            shuffle: true,
            level: Self::checked_level(level)?,
        })
    }

    fn checked_level(level: u32) -> Result<u32> {
        if level > 9 {
            return Err(FitsError::InvalidCompressionParameter {
                detail: format!("gzip compression level {level} is outside 0..=9"),
            });
        }
        Ok(level)
    }
}

impl Default for Gzip {
    fn default() -> Self {
        Self {
            shuffle: false,
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
                detail: format!("HCOMPRESS scale {scale} must be finite and positive"),
            });
        }
        Ok(scale)
    }
}

impl HcompressScale {
    /// The integer scale one tile of `values`, of `dimensions`, is compressed at:
    /// cfitsio rounds a relative scale times the tile's noise, or an absolute one
    /// (`imcomp_compress_tile`).
    fn tile_scale(
        self,
        values: &[i64],
        dimensions: &[usize],
        scratch: &mut quantize::QuantizeScratch,
    ) -> Result<i32> {
        let scale = match self {
            Self::Lossless => return Ok(0),
            Self::Absolute(scale) => scale.round(),
            Self::Relative(factor) => {
                let noise = scratch.integer_noise(values, dimensions[0]);
                (factor * noise).round()
            }
        };
        if scale > f64::from(i32::MAX) {
            return Err(FitsError::UnsupportedCompression {
                name: "HCOMPRESS_1 tile scale exceeds the 32-bit stream range".to_string(),
            });
        }
        Ok(scale as i32)
    }

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
        shuffle: false,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });
    pub const GZIP_SHUFFLED: Self = Self::Gzip(Gzip {
        shuffle: true,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });
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
    /// `None` until set, which a float image reads as the default and a table refuses.
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

    /// A float image's quantization; [`Quantization::default`] when it is not set. A
    /// table write refuses it.
    #[must_use]
    pub const fn with_quantization(mut self, quantization: Quantization) -> Self {
        self.quantization = Some(quantization);
        self
    }

    /// Rows per tile of a table of `row_count` rows of `row_len` bytes, in `1..=row_count`.
    /// Errors when the tiles have more than one axis or quantization is set.
    fn table_tile_rows(&self, row_count: usize, row_len: usize) -> Result<usize> {
        if self.quantization.is_some() {
            return Err(FitsError::InvalidCompressionParameter {
                detail: "a compressed table takes no float quantization".to_string(),
            });
        }
        let rows = match *self.tiles.as_slice() {
            [] => DEFAULT_TABLE_TILE_BYTES
                .checked_div(row_len)
                .unwrap_or(row_count),
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

/// The tiled-image codec selected by `ZCMPTYPE`, parsed once from the keyword string
/// then matched exhaustively — the image-path counterpart to the table path's `Algo`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImageCodec {
    Gzip1,
    Gzip2,
    Rice1,
    Plio1,
    Hcompress1,
    NoCompress,
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

fn parameter_index(keyword: &str) -> Option<usize> {
    keyword::index(keyword, "ZNAME").filter(|index| (1..=999).contains(index))
}

impl ImageCodec {
    fn parse(s: &str) -> Result<Self> {
        Ok(match s {
            "GZIP_1" => Self::Gzip1,
            "GZIP_2" => Self::Gzip2,
            "RICE_1" => Self::Rice1,
            "PLIO_1" => Self::Plio1,
            "HCOMPRESS_1" => Self::Hcompress1,
            "NOCOMPRESS" => Self::NoCompress,
            other => {
                return Err(FitsError::UnsupportedCompression {
                    name: other.to_string(),
                });
            }
        })
    }

    /// The `ZCMPTYPE` (image) / `ZCTYPn` (table column) keyword spelling — the
    /// inverse of [`ImageCodec::parse`], and the only place these names are written.
    /// The table path's narrower `Algo` and the public [`Compression`] both read
    /// them from here.
    const fn name(self) -> &'static str {
        match self {
            Self::Gzip1 => "GZIP_1",
            Self::Gzip2 => "GZIP_2",
            Self::Rice1 => "RICE_1",
            Self::Plio1 => "PLIO_1",
            Self::Hcompress1 => "HCOMPRESS_1",
            Self::NoCompress => "NOCOMPRESS",
        }
    }
}

impl Compression {
    const fn image_codec(self) -> ImageCodec {
        match self {
            Self::Gzip(config) if config.shuffle => ImageCodec::Gzip2,
            Self::Gzip(_) => ImageCodec::Gzip1,
            Self::Rice => ImageCodec::Rice1,
            Self::Plio => ImageCodec::Plio1,
            Self::Hcompress(_) => ImageCodec::Hcompress1,
            Self::None => ImageCodec::NoCompress,
        }
    }

    /// FITS `ZCMPTYPE` name written for this choice.
    pub const fn name(self) -> &'static str {
        self.image_codec().name()
    }

    const fn gzip_level(self) -> u32 {
        match self {
            Self::Gzip(config) => config.level,
            _ => gzip::DEFAULT_GZIP_LEVEL,
        }
    }

    const fn hcompress_scale(self) -> HcompressScale {
        match self {
            Self::Hcompress(config) => config.scale,
            _ => HcompressScale::Lossless,
        }
    }
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
    use rayon::prelude::*;
    (0..ntiles)
        .into_par_iter()
        .map_init(init, |scratch, t| f(scratch, t))
        .collect()
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
#[cfg(test)]
pub(crate) mod internals {
    /// value(x, y) = 7x − 5y over 24×16, row-major: the integer codec fixtures.
    pub(crate) fn ramp() -> Vec<i16> {
        (0..24 * 16)
            .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
            .collect()
    }

    /// value(x, y) = (x + y) mod 7 over 24×16: the PLIO fixture, a non-negative mask.
    pub(crate) fn mask() -> Vec<i32> {
        (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect()
    }
}

#[cfg(test)]
mod tests;
