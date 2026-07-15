//! Tiled image+table compression (§10) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module holds the shared pieces — the write-time [`CompressOptions`], the
//! [`ImageCodec`] dispatch, the per-tile [`map_tiles`] fan-out, and the `P`-vs-`Q`
//! descriptor threshold — while the directions live in [`decode`]
//! ([`decompress_image`]) and [`encode`] ([`compress_image`], all five codecs:
//! `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` with `SMOOTH=1` decode).
//! Float images are quantized per-tile (`ZSCALE`/`ZZERO`) with `NO_DITHER`,
//! `SUBTRACTIVE_DITHER_1`, or `SUBTRACTIVE_DITHER_2`. The per-codec work lives in
//! [`gzip`], [`rice`], [`plio`], and [`hcompress`]; tiled *table* compression
//! (§10.3) lives in [`table`] ([`compress_table`]/[`uncompress_table`]).

mod convert;
mod decode;
mod encode;
mod geometry;
mod gzip;
mod hcompress;
mod plio;
mod quantize;
mod rice;
mod table;

pub(crate) use decode::decompress_image;
pub(crate) use decode::decompress_image_into_words;
pub(crate) use encode::compress_image;
pub(crate) use table::compress_table;
pub(crate) use table::uncompress_table;

use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;

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
    pub(crate) fn dithered(self) -> bool {
        !matches!(self, DitherMethod::None)
    }
}

/// Write-time tuning for [`crate::FitsWriter::write_compressed_image`]. Each field
/// applies only to the codecs that use it; the rest ignore it. Every field defaults
/// to conventional behavior, so `CompressOptions::default()` (row tiling) or
/// `CompressOptions::tiled(shape)` is the common case.
#[derive(Debug, Clone)]
pub struct CompressOptions {
    /// Tile shape, fastest axis first. Empty ⇒ one tile per row (the default).
    /// `HCOMPRESS_1` requires a 2-D shape.
    pub tile_shape: Vec<usize>,
    /// `flate2` deflate level (0–9) for `GZIP_1`/`GZIP_2`. Lossless — only the
    /// speed↔ratio tradeoff changes.
    pub gzip_level: u32,
    /// `HCOMPRESS_1` quantization scale: `0` = lossless, larger = more lossy / smaller.
    pub hcompress_scale: i32,
    /// Float quantization noise divisor (`qlevel`): `0` ⇒ cfitsio's default of
    /// noise/4; larger keeps more precision (and grows the output). Ignored by the
    /// integer codecs.
    pub quantize_level: f64,
    /// Dithering for float quantization (`ZQUANTIZ`). Defaults to
    /// `SUBTRACTIVE_DITHER_1`. Ignored by the integer codecs.
    pub dither: DitherMethod,
}

impl Default for CompressOptions {
    fn default() -> CompressOptions {
        CompressOptions {
            tile_shape: Vec::new(),
            gzip_level: gzip::DEFAULT_GZIP_LEVEL,
            hcompress_scale: 0,
            quantize_level: 0.0,
            dither: DitherMethod::Subtractive1,
        }
    }
}

impl CompressOptions {
    /// Default options with an explicit tile shape (fastest axis first; empty ⇒ row
    /// tiling). Tune further with struct-update syntax:
    /// `CompressOptions { gzip_level: 9, ..CompressOptions::tiled([256, 256]) }`.
    pub fn tiled(tile_shape: impl Into<Vec<usize>>) -> CompressOptions {
        CompressOptions {
            tile_shape: tile_shape.into(),
            ..CompressOptions::default()
        }
    }
}

/// A restored header and its decompressed data unit — the result of
/// [`uncompress_table`] (a named pair rather than a bare `(Header, Vec<u8>)`).
#[derive(Debug)]
pub(crate) struct HduParts {
    pub header: Header,
    pub data: Vec<u8>,
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
/// near-linear speedup. The caller then folds the results — scatter into the image,
/// or concatenate into the heap — and *that* step stays serial because tile order
/// and heap offsets are sequential.
#[cfg(feature = "parallel")]
pub(crate) fn map_tiles<S, T, I, F>(ntiles: usize, init: I, f: F) -> Result<Vec<T>>
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
pub(crate) fn map_tiles<S, T, I, F>(ntiles: usize, init: I, f: F) -> Result<Vec<T>>
where
    I: FnOnce() -> S,
    F: Fn(&mut S, usize) -> Result<T>,
{
    let mut scratch = init();
    (0..ntiles).map(|t| f(&mut scratch, t)).collect()
}

#[cfg(test)]
mod tests;
