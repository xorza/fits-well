//! Tiled image+table compression (§10) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module holds the shared pieces — the write-time [`Compression`] and
//! [`CompressionOptions`], the
//! [`ImageCodec`] dispatch, the per-tile [`map_tiles`] fan-out, and the `P`-vs-`Q`
//! descriptor threshold — while the directions live in [`decode`]
//! ([`decode::decompress_image`]) and [`encode`] ([`encode::compress_image`], all five codecs:
//! `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` with `SMOOTH=1` decode).
//! Float images are quantized per-tile (`ZSCALE`/`ZZERO`) with `NO_DITHER`,
//! `SUBTRACTIVE_DITHER_1`, or `SUBTRACTIVE_DITHER_2`. The per-codec work lives in
//! [`gzip`], [`rice`], [`plio`], and [`hcompress`]; tiled *table* compression
//! (§10.3) lives in [`table`] ([`table::compress_table`]/[`table::uncompress_table`]).

mod convert;
pub(crate) mod decode;
pub(crate) mod encode;
mod gzip;
mod hcompress;
mod plio;
mod quantize;
mod rice;
pub(crate) mod table;
mod tile_geometry;

use crate::error::FitsError;
use crate::error::Result;
use crate::keyword;

/// Float-quantization dithering for [`crate::FitsWriter::write_compressed_image`]
/// (`ZQUANTIZ`, §10.2). Applies only when compressing a float image; the integer
/// codecs ignore it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DitherMethod {
    /// `NO_DITHER` — plain linear quantization.
    None,
    /// `SUBTRACTIVE_DITHER_1` — per-pixel dither from the shared random sequence
    /// (cfitsio's default).
    #[default]
    Subtractive1,
    /// `SUBTRACTIVE_DITHER_2` — like `Subtractive1`, but exact zeros are preserved
    /// (stored as a reserved integer rather than dithered).
    Subtractive2,
}

impl DitherMethod {
    /// Whether this method applies a per-pixel dither (everything but `None`).
    fn dithered(self) -> bool {
        !matches!(self, DitherMethod::None)
    }

    /// The `ZQUANTIZ` keyword spelling; `None` for a value this crate does not model.
    /// A float image must be able to reproduce its dither exactly, so the decoder
    /// rejects an unrecognized value there — an integer image ignores `ZQUANTIZ`.
    fn parse(s: &str) -> Option<DitherMethod> {
        match s {
            "NO_DITHER" => Some(DitherMethod::None),
            "SUBTRACTIVE_DITHER_1" => Some(DitherMethod::Subtractive1),
            "SUBTRACTIVE_DITHER_2" => Some(DitherMethod::Subtractive2),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            DitherMethod::None => "NO_DITHER",
            DitherMethod::Subtractive1 => "SUBTRACTIVE_DITHER_1",
            DitherMethod::Subtractive2 => "SUBTRACTIVE_DITHER_2",
        }
    }
}

/// Validated GZIP configuration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Gzip {
    shuffle: bool,
    level: u32,
}

impl Gzip {
    /// Standard `GZIP_1` at a validated deflate level.
    pub fn new(level: u32) -> Result<Gzip> {
        Gzip::configured(false, level)
    }

    /// Byte-shuffled `GZIP_2` at a validated deflate level.
    pub fn shuffled(level: u32) -> Result<Gzip> {
        Gzip::configured(true, level)
    }

    fn configured(shuffle: bool, level: u32) -> Result<Gzip> {
        if level > 9 {
            return Err(FitsError::InvalidValue {
                card: format!("gzip compression level {level} is outside 0..=9"),
            });
        }
        Ok(Gzip { shuffle, level })
    }
}

impl Default for Gzip {
    fn default() -> Gzip {
        Gzip {
            shuffle: false,
            level: gzip::DEFAULT_GZIP_LEVEL,
        }
    }
}

/// Validated HCOMPRESS configuration.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Hcompress {
    scale: f64,
}

impl Hcompress {
    /// Configure lossy HCOMPRESS with a positive finite multiplier of the
    /// per-tile background-noise estimate (§10.4.4).
    pub fn lossy(scale: f64) -> Result<Hcompress> {
        if !scale.is_finite() || scale <= 0.0 {
            return Err(FitsError::InvalidValue {
                card: format!("lossy HCOMPRESS scale {scale} must be positive"),
            });
        }
        Ok(Hcompress { scale })
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
    pub const GZIP: Compression = Compression::Gzip(Gzip {
        shuffle: false,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });
    pub const GZIP_SHUFFLED: Compression = Compression::Gzip(Gzip {
        shuffle: true,
        level: gzip::DEFAULT_GZIP_LEVEL,
    });
}

/// Float quantization shared by the lossless integer codecs.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Quantization {
    level: f64,
    dither: DitherMethod,
}

impl Default for Quantization {
    fn default() -> Quantization {
        Quantization {
            level: 0.0,
            dither: DitherMethod::Subtractive1,
        }
    }
}

/// Shared tiled-image options. Codec-specific knobs are part of [`Compression`].
#[derive(Debug, Clone, Default)]
pub struct CompressionOptions {
    /// Tile shape, fastest axis first. Empty means one tile per row.
    tile_shape: Vec<usize>,
    quantization: Quantization,
}

impl CompressionOptions {
    pub fn tiled(tile_shape: impl Into<Vec<usize>>) -> CompressionOptions {
        CompressionOptions {
            tile_shape: tile_shape.into(),
            ..CompressionOptions::default()
        }
    }

    pub fn with_quantization(
        mut self,
        level: f64,
        dither: DitherMethod,
    ) -> Result<CompressionOptions> {
        if !level.is_finite() || level < 0.0 {
            return Err(FitsError::InvalidValue {
                card: format!("float quantization level {level} must be finite and nonnegative"),
            });
        }
        self.quantization = Quantization { level, dither };
        Ok(self)
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

fn parameter_index(keyword: &str) -> Option<usize> {
    keyword::index(keyword, "ZNAME").filter(|index| (1..=999).contains(index))
}

impl ImageCodec {
    fn parse(s: &str) -> Result<ImageCodec> {
        Ok(match s {
            "GZIP_1" => ImageCodec::Gzip1,
            "GZIP_2" => ImageCodec::Gzip2,
            "RICE_1" => ImageCodec::Rice1,
            "PLIO_1" => ImageCodec::Plio1,
            "HCOMPRESS_1" => ImageCodec::Hcompress1,
            "NOCOMPRESS" => ImageCodec::NoCompress,
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
    fn name(self) -> &'static str {
        match self {
            ImageCodec::Gzip1 => "GZIP_1",
            ImageCodec::Gzip2 => "GZIP_2",
            ImageCodec::Rice1 => "RICE_1",
            ImageCodec::Plio1 => "PLIO_1",
            ImageCodec::Hcompress1 => "HCOMPRESS_1",
            ImageCodec::NoCompress => "NOCOMPRESS",
        }
    }
}

impl Compression {
    fn image_codec(self) -> ImageCodec {
        match self {
            Compression::Gzip(config) if config.shuffle => ImageCodec::Gzip2,
            Compression::Gzip(_) => ImageCodec::Gzip1,
            Compression::Rice => ImageCodec::Rice1,
            Compression::Plio => ImageCodec::Plio1,
            Compression::Hcompress(_) => ImageCodec::Hcompress1,
            Compression::None => ImageCodec::NoCompress,
        }
    }

    /// FITS `ZCMPTYPE` name written for this choice.
    pub fn name(self) -> &'static str {
        self.image_codec().name()
    }

    fn gzip_level(self) -> u32 {
        match self {
            Compression::Gzip(config) => config.level,
            _ => gzip::DEFAULT_GZIP_LEVEL,
        }
    }

    fn hcompress_scale(self) -> f64 {
        match self {
            Compression::Hcompress(config) => config.scale,
            _ => 0.0,
        }
    }
}

/// Whether a tiled-image data unit needs 64-bit `Q` descriptors rather than 32-bit
/// `P`: any heap offset or tile element count past the signed-32-bit range (§10.1.3).
/// The integer encoder promotes to `Q`; the fixed-layout float container instead
/// rejects (it can't widen its row), so both share this one threshold.
fn needs_wide(heap_len: usize, max_nelem: usize) -> bool {
    heap_len > i32::MAX as usize || max_nelem > i32::MAX as usize
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

#[cfg(test)]
mod tests;
