//! Tiled image compression (§10.1) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module orchestrates tile reassembly and (de)quantization; the per-codec
//! work lives in [`gzip`], [`rice`], [`plio`], and [`hcompress`].
//!
//! Decode and encode ([`encode_image`]) cover all five codecs: `GZIP_1`,
//! `GZIP_2`, `RICE_1`, `PLIO_1`, and `HCOMPRESS_1` (with `SMOOTH=1` decode). Float
//! images are quantized per-tile (`ZSCALE`/`ZZERO`) with `NO_DITHER`,
//! `SUBTRACTIVE_DITHER_1`, or `SUBTRACTIVE_DITHER_2`, with `ZBLANK`/NaN nulls and a
//! raw-gzip fallback for constant tiles. Tiled *table* compression (§10.3) lives in
//! [`table`] ([`compress_table`]/[`uncompress_table`]).

mod gzip;
mod hcompress;
mod plio;
mod quantize;
mod rice;
mod table;

pub(crate) use table::compress_table;
pub(crate) use table::uncompress_table;

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::endian::encode_be;
use crate::endian::push_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::ColumnData;

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
}

impl Default for CompressOptions {
    fn default() -> CompressOptions {
        CompressOptions {
            tile_shape: Vec::new(),
            gzip_level: gzip::DEFAULT_GZIP_LEVEL,
            hcompress_scale: 0,
            quantize_level: 0.0,
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

/// Decompress a tiled-image `BINTABLE` into the full [`Image`] it encodes.
pub(crate) fn decompress_image(header: &Header, table: &BinTable) -> Result<Image> {
    if header.get_logical("ZIMAGE") != Some(true) {
        return Err(FitsError::NotCompressedImage);
    }
    let zbitpix = Bitpix::from_code(
        header
            .get_integer("ZBITPIX")
            .ok_or(FitsError::MissingKeyword { name: "ZBITPIX" })?,
    )?;
    let is_float = zbitpix.is_float();
    let cmptype = header
        .get_text("ZCMPTYPE")
        .ok_or(FitsError::MissingKeyword { name: "ZCMPTYPE" })?
        .to_string();

    let znaxis = header
        .get_integer("ZNAXIS")
        .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })?;
    // `ZNAXIS` is untrusted; cap it like the uncompressed `NAXIS` path (§4.4.1) so a
    // negative value can't wrap through `as usize` and a huge one can't drive the
    // per-axis keyword loops below.
    if !(0..=999).contains(&znaxis) {
        return Err(FitsError::KeywordOutOfRange { name: "ZNAXIS" });
    }
    let znaxis = znaxis as usize;
    let dims = read_axes(header, "ZNAXIS", znaxis)?;
    // A `ZNAXIS = 0` ZIMAGE has no data array (as an uncompressed `NAXIS = 0` does).
    // Return empty before building the geometry, which would otherwise size `total`
    // as the empty product (1) and fabricate a phantom one-pixel tile.
    if dims.is_empty() {
        return Ok(Image {
            shape: dims,
            samples: zeroed_samples(zbitpix, 0),
            scaling: Scaling::from_header(header),
        });
    }
    // `ZNAXISn` are untrusted; guard the product up front — before reading any tile
    // — so a wrapped value can't mis-size the output buffer below (the un-wrapped
    // strides would then scatter out of bounds). Mirrors `hdu::data_extent`.
    let total = dims
        .iter()
        .try_fold(1usize, |acc, &n| acc.checked_mul(n))
        .ok_or(FitsError::DataUnitOverflow)?;
    let tiles: Vec<usize> = (1..=znaxis)
        .map(|i| {
            header
                .get_integer(key!("ZTILE{i}").as_str())
                .map(|v| v.max(1) as usize)
                .unwrap_or(if i == 1 { dims[0] } else { 1 })
        })
        .collect();

    let rice = rice::rice_params(header, zbitpix);
    // Float pixels are quantized to integers of `bytepix` bytes; decode the tile
    // as that integer type, then dequantize. Integer images decode as `zbitpix`.
    let int_bitpix = if is_float {
        bytepix_to_bitpix(rice.bytepix)
    } else {
        zbitpix
    };

    // Float quantization: NO_DITHER, SUBTRACTIVE_DITHER_1, and SUBTRACTIVE_DITHER_2.
    let zquantiz = header
        .get_text("ZQUANTIZ")
        .unwrap_or("NO_DITHER")
        .to_string();
    let method = match zquantiz.as_str() {
        "NO_DITHER" => quantize::DitherMethod::None,
        "SUBTRACTIVE_DITHER_1" => quantize::DitherMethod::Subtractive1,
        "SUBTRACTIVE_DITHER_2" => quantize::DitherMethod::Subtractive2,
        other => {
            if is_float {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("float quantization {other}"),
                });
            }
            quantize::DitherMethod::None
        }
    };
    let zdither0 = header.get_integer("ZDITHER0").unwrap_or(1);
    // ZBLANK may be a keyword (constant) or a per-tile column; §10.1.3 says the
    // column value wins where present.
    let zblank_keyword = header.get_integer("ZBLANK");
    let zblank_column = read_i64_column(table, "ZBLANK");
    let smooth = hcompress_smooth(header);
    let codec = CodecParams {
        blocksize: rice.blocksize,
        bytepix: rice.bytepix,
        smooth,
    };

    // Per-tile compressed data, with the conventional fallback columns.
    let primary = read_tiles(table, "COMPRESSED_DATA")?;
    let gzip_fallback = read_tiles(table, "GZIP_COMPRESSED_DATA")?;
    let uncompressed = read_tiles(table, "UNCOMPRESSED_DATA")?;
    // Per-tile linear dequantization parameters (float only).
    let zscale = read_f64_column(table, "ZSCALE");
    let zzero = read_f64_column(table, "ZZERO");

    let geom = TileGeometry::new(&dims, &tiles);
    let ntiles = geom.ntiles();
    let mut samples = zeroed_samples(zbitpix, total);

    // Decode and scatter each tile in one fused pass — parallel under the `parallel`
    // feature, where tiles write disjoint regions of `samples` concurrently (they
    // partition the image). Each value is narrowed to `ZBITPIX` as it lands, so there
    // is no whole-image `i64`/`f64` intermediate and no separate serial scatter tail.
    let ctx = DecodeCtx {
        cmptype: &cmptype,
        zbitpix,
        int_bitpix,
        codec,
    };
    if is_float {
        let decode = |t: usize, s: &TileScratch, out: &mut Vec<f64>, ints: &mut Vec<i64>| {
            let cols = TileColumns {
                primary: primary.get(t),
                gzip: gzip_fallback.get(t),
                uncompressed: uncompressed.get(t),
            };
            let dq = Dequant {
                scale: column_at(&zscale, t).unwrap_or(1.0),
                zero: column_at(&zzero, t).unwrap_or(0.0),
                method,
                irow: t as i64 + zdither0,
                zblank: column_at(&zblank_column, t).or(zblank_keyword),
            };
            decode_float_tile_into(&ctx, cols, s.nelem(), dq, out, ints)
        };
        match &mut samples {
            ImageData::F32(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v as f32)?,
            ImageData::F64(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v)?,
            _ => unreachable!("a float ZBITPIX yields a float sample buffer"),
        }
    } else {
        let decode = |t: usize, s: &TileScratch, out: &mut Vec<i64>, _ints: &mut Vec<i64>| {
            let cols = TileColumns {
                primary: primary.get(t),
                gzip: gzip_fallback.get(t),
                uncompressed: uncompressed.get(t),
            };
            decode_one_tile_into(&ctx, cols, s.nelem(), out)
        };
        match &mut samples {
            ImageData::U8(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v as u8)?,
            ImageData::I16(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v as i16)?,
            ImageData::I32(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v as i32)?,
            ImageData::I64(o) => run_decode_scatter(ntiles, &geom, o, decode, |v| v)?,
            _ => unreachable!("an integer ZBITPIX yields an integer sample buffer"),
        }
    }
    Ok(Image {
        shape: dims,
        samples,
        scaling: Scaling::from_header(header),
    })
}

/// Decode every tile and scatter its values into `out` at the tile's positions,
/// narrowing each with `convert`. Under `parallel` the tiles run concurrently and
/// write disjoint regions of `out` directly (no collect, no serial scatter);
/// otherwise it is a plain fused loop.
fn run_decode_scatter<S, D>(
    ntiles: usize,
    geom: &TileGeometry,
    out: &mut [D],
    decode: impl Fn(usize, &TileScratch, &mut Vec<S>, &mut Vec<i64>) -> Result<()> + Sync + Send,
    convert: impl Fn(S) -> D + Sync + Send,
) -> Result<()>
where
    S: Copy + Send,
{
    // Per-worker decode buffers, reused across that worker's tiles (one set per rayon
    // worker via `map_init`, a single set serially): `vals` is the decoded tile (the
    // scatter source), `ints` the float path's quantized-int temp (unused otherwise).
    // Reusing them means steady-state decode allocates nothing per tile, and the
    // buffers stay cache-resident across tiles.
    #[cfg(feature = "parallel")]
    {
        let sink = DisjointOut::new(out);
        let init = || (TileScratch::default(), Vec::<S>::new(), Vec::<i64>::new());
        map_tiles(ntiles, init, |(scratch, vals, ints), t| -> Result<()> {
            geom.tile_into(t, scratch);
            decode(t, scratch, vals, ints)?;
            // SAFETY: the image tiles partition the pixel grid, so this tile's row
            // ranges are disjoint from every other tile's — concurrent writes through
            // `sink` never alias. `tile_into` clips rows to the image, which sized
            // `out`, so each row is in bounds.
            unsafe { sink.scatter_rows(&scratch.row_bases, scratch.row_len, vals, &convert) };
            Ok(())
        })?;
        Ok(())
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut scratch = TileScratch::default();
        let mut vals: Vec<S> = Vec::new();
        let mut ints: Vec<i64> = Vec::new();
        for t in 0..ntiles {
            geom.tile_into(t, &mut scratch);
            decode(t, &scratch, &mut vals, &mut ints)?;
            scatter_rows(out, &scratch.row_bases, scratch.row_len, &vals, &convert);
        }
        Ok(())
    }
}

/// Scatter `vals` (the tile's pixels in row-major order) into `out` one contiguous
/// row at a time: `row_len` values land at each `row_bases` offset, narrowed by
/// `convert`. A `vals` shorter than the tile fills only what it covers (matching the
/// old index-zip), so a malformed tile can't index out of bounds.
#[cfg(not(feature = "parallel"))]
fn scatter_rows<S: Copy, D>(
    out: &mut [D],
    row_bases: &[usize],
    row_len: usize,
    vals: &[S],
    convert: &impl Fn(S) -> D,
) {
    let mut off = 0;
    for &base in row_bases {
        if off >= vals.len() {
            break;
        }
        let rl = row_len.min(vals.len() - off);
        for (d, &v) in out[base..base + rl].iter_mut().zip(&vals[off..off + rl]) {
            *d = convert(v);
        }
        off += row_len;
    }
}

/// A raw pointer into the decode output, shared across rayon workers so each tile
/// scatters its decoded values in place. The `Sync` impl is sound *only* under the
/// contract that callers write disjoint index sets — which holds because the image
/// tiles partition the pixel grid (see [`run_decode_scatter`]).
#[cfg(feature = "parallel")]
struct DisjointOut<D> {
    ptr: *mut D,
    len: usize,
}

// SAFETY: see the type doc — concurrent use only writes disjoint, in-bounds indices.
#[cfg(feature = "parallel")]
unsafe impl<D> Sync for DisjointOut<D> {}

#[cfg(feature = "parallel")]
impl<D> DisjointOut<D> {
    fn new(out: &mut [D]) -> DisjointOut<D> {
        DisjointOut {
            ptr: out.as_mut_ptr(),
            len: out.len(),
        }
    }

    /// Write `vals` (row-major) into the tile's contiguous rows: `row_len` values at
    /// each `row_bases` offset, narrowed by `convert`. A short `vals` fills only what
    /// it covers (matching the serial [`scatter_rows`]).
    ///
    /// # Safety
    /// Each `[base, base + row_len)` range must be `<= self.len` and disjoint from
    /// those passed by any concurrent call, so no two writes alias.
    unsafe fn scatter_rows<S: Copy>(
        &self,
        row_bases: &[usize],
        row_len: usize,
        vals: &[S],
        convert: &impl Fn(S) -> D,
    ) {
        let mut off = 0;
        for &base in row_bases {
            if off >= vals.len() {
                break;
            }
            let rl = row_len.min(vals.len() - off);
            debug_assert!(base + rl <= self.len, "tile row out of bounds {}", self.len);
            // SAFETY: `[base, base + rl)` is in bounds (debug-asserted; guaranteed by
            // the tile geometry) and disjoint across tiles, so these are non-aliasing
            // in-bounds writes over one contiguous run.
            let dst = unsafe { std::slice::from_raw_parts_mut(self.ptr.add(base), rl) };
            for (d, &v) in dst.iter_mut().zip(&vals[off..off + rl]) {
                *d = convert(v);
            }
            off += row_len;
        }
    }
}

/// Per-worker tile-encode scratch, reused across the tiles one rayon worker
/// handles (via `map_tiles`'s `init`): the tile geometry plus the widened pixel
/// buffers the codec reads from. Reusing them means steady-state tile compression
/// allocates only each tile's compressed output, not the gather buffers.
#[derive(Debug, Default)]
struct EncodeScratch {
    tile: TileScratch,
    /// The tile's pixels widened to `i64` — the integer codec input (and the
    /// quantized int32 plane for float images).
    ints: Vec<i64>,
    /// The tile's pixels as `f64` (float images only; stays empty otherwise).
    floats: Vec<f64>,
    /// The tile's pixels packed to big-endian bytes — the gzip codecs' input,
    /// reused so each tile allocates only its compressed output.
    be: Vec<u8>,
}

/// Encode an integer [`Image`] as a tiled-compressed `BINTABLE`: returns the
/// `ZIMAGE` header and the data unit (per-tile `P` descriptors + the heap of
/// compressed tile bytes). A tile shape of the wrong length falls back to
/// row-tiling. Float images and codecs without an encoder are rejected.
pub(crate) fn encode_image(
    image: &Image,
    cmptype: &str,
    options: &CompressOptions,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let bitpix = image.samples.bitpix();
    if bitpix.is_float() {
        return encode_float_image(image, cmptype, options, out);
    }
    // RICE handles only 1/2/4-byte pixels (cfitsio parity); refuse the 64-bit path
    // rather than silently corrupting. Table 37 lists BYTEPIX 8 as permitted, but
    // neither this encoder nor the decoder implements the 64-bit bitstream.
    if cmptype == "RICE_1" && bitpix.elem_size() > 4 {
        return Err(FitsError::UnsupportedCompression {
            name: "RICE_1 with BYTEPIX > 4 (64-bit pixels)".to_string(),
        });
    }
    // HCOMPRESS is a 32-bit transform; an I64 image would be silently truncated to
    // i32 by the encoder. Refuse it rather than corrupt (I32 stays supported, with
    // the documented "moderate values" caveat against H-transform overflow).
    if cmptype == "HCOMPRESS_1" && bitpix.elem_size() > 4 {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1 with 64-bit pixels".to_string(),
        });
    }
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, &options.tile_shape);

    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = geom.ntiles();
    let bytepix = bitpix.elem_size();
    let (gzip_level, scale) = (options.gzip_level, options.hcompress_scale);

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
        Ok(match cmptype {
            "GZIP_1" => {
                i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip_encode(&s.be, gzip_level))
            }
            "GZIP_2" => {
                i64_to_be_into(vals, bitpix, &mut s.be);
                TileCell::Bytes(gzip::gzip2_encode(&s.be, bytepix, gzip_level))
            }
            "RICE_1" => TileCell::Bytes(rice::rice_encode(vals, bytepix, 32)),
            "PLIO_1" => TileCell::I16(plio::plio_encode(vals, vals.len())),
            "HCOMPRESS_1" => TileCell::Bytes(hcompress::hcompress_tile_encode(
                vals,
                &s.tile.tdims,
                scale,
            )?),
            // §10.4: store the tile's raw big-endian pixels, uncompressed.
            "NOCOMPRESS" => TileCell::Bytes(i64_to_be(vals, bitpix)),
            other => {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("{other} (write)"),
                });
            }
        })
    })?;

    let mut descriptors: Vec<(usize, usize)> = Vec::with_capacity(ntiles);
    let mut heap: Vec<u8> = Vec::new();
    for cell in &cells {
        descriptors.push((cell.nelem(), heap.len()));
        cell.extend_heap(&mut heap);
    }

    let maxnelem = descriptors.iter().map(|&(n, _)| n).max().unwrap_or(0);
    // §10.1.3: use 64-bit `Q` descriptors once the heap or a tile's element count
    // exceeds the 32-bit `P` range; otherwise the compact `P` form.
    let wide = heap.len() > i32::MAX as usize || maxnelem > i32::MAX as usize;

    // Data unit: an array descriptor (count, heap offset) per tile, then the heap.
    // Built into the caller's reused buffer (the writer's scratch).
    out.clear();
    out.reserve(ntiles * if wide { 16 } else { 8 } + heap.len());
    for &(nelem, offset) in &descriptors {
        push_pq_descriptor(out, wide, nelem as u64, offset as u64);
    }
    out.extend_from_slice(&heap);

    let tform_letter = if cmptype == "PLIO_1" { 'I' } else { 'B' };
    let desc = if wide { 'Q' } else { 'P' };
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .comment("XTENSION", "binary table extension");
    h.set("BITPIX", 8).set("NAXIS", 2);
    h.set("NAXIS1", if wide { 16 } else { 8 })
        .set("NAXIS2", ntiles as i64);
    h.set("PCOUNT", heap.len() as i64).set("GCOUNT", 1);
    h.set("TFIELDS", 1);
    h.set("TTYPE1", "COMPRESSED_DATA");
    h.set("TFORM1", format!("1{desc}{tform_letter}({maxnelem})"));
    set_zimage_axes(&mut h, cmptype, bitpix, dims, &tiles);
    match cmptype {
        "RICE_1" => {
            h.set("ZNAME1", "BLOCKSIZE").set("ZVAL1", 32);
            h.set("ZNAME2", "BYTEPIX").set("ZVAL2", bytepix as i64);
        }
        "HCOMPRESS_1" => {
            h.set("ZNAME1", "SCALE").set("ZVAL1", scale as i64);
            h.set("ZNAME2", "SMOOTH").set("ZVAL2", 0);
        }
        _ => {}
    }
    // §10.2: tiles store the *raw* stored integers, so the original image's
    // physical scaling and undefined-pixel sentinel must travel in the header to
    // be reconstructed on decode (`bitpix` is integer here — floats diverted above).
    if !image.scaling.is_identity() {
        h.set("BZERO", image.scaling.bzero);
        h.set("BSCALE", image.scaling.bscale);
    }
    if let Some(blank) = image.scaling.blank {
        h.set("BLANK", blank);
    }
    Ok(h)
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
/// `ZSCALE`/`ZZERO`, then compressed with `cmptype` (`GZIP_1`/`GZIP_2`/`RICE_1`);
/// a tile that can't be quantized (constant data) is stored as raw gzip'd floats
/// in `GZIP_COMPRESSED_DATA`. The table has four columns: `COMPRESSED_DATA`,
/// `GZIP_COMPRESSED_DATA`, `ZSCALE`, `ZZERO`.
fn encode_float_image(
    image: &Image,
    cmptype: &str,
    options: &CompressOptions,
    out: &mut Vec<u8>,
) -> Result<Header> {
    if !matches!(cmptype, "GZIP_1" | "GZIP_2" | "RICE_1") {
        return Err(FitsError::UnsupportedCompression {
            name: format!("{cmptype} for float images (write)"),
        });
    }
    let zbitpix = image.samples.bitpix();
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, &options.tile_shape);

    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = geom.ntiles();

    let zdither0 = 1i64; // deterministic dither seed (any 1..=10000 is valid)
    let int_bitpix = Bitpix::I32; // quantized planes are always int32
    let method = quantize::DitherMethod::Subtractive1; // cfitsio's default
    let (gzip_level, qlevel) = (options.gzip_level, options.quantize_level);

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
                match quantize::quantize_tile(&s.floats, nx, ny, qlevel, method, irow) {
                    Some(q) => {
                        s.ints.clear();
                        s.ints.extend(q.idata.iter().map(|&v| v as i64));
                        let bytes = match cmptype {
                            "GZIP_1" => {
                                i64_to_be_into(&s.ints, int_bitpix, &mut s.be);
                                gzip::gzip_encode(&s.be, gzip_level)
                            }
                            "GZIP_2" => {
                                i64_to_be_into(&s.ints, int_bitpix, &mut s.be);
                                gzip::gzip2_encode(&s.be, 4, gzip_level)
                            }
                            "RICE_1" => rice::rice_encode(&s.ints, 4, 32),
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
                    None => FloatTile {
                        bytes: gzip::gzip_encode(&float_to_be(&s.floats, zbitpix), gzip_level),
                        zscale: 0.0,
                        zzero: 0.0,
                        quantized: false,
                        has_null: false,
                    },
                },
            )
        },
    )?;

    let mut cd_desc: Vec<(usize, usize)> = Vec::with_capacity(ntiles);
    let mut gz_desc: Vec<(usize, usize)> = Vec::with_capacity(ntiles);
    let mut zscale = vec![0f64; ntiles];
    let mut zzero = vec![0f64; ntiles];
    let mut heap: Vec<u8> = Vec::new();
    let mut any_null = false;
    for (t, tile) in tiles_out.iter().enumerate() {
        zscale[t] = tile.zscale;
        zzero[t] = tile.zzero;
        any_null |= tile.has_null;
        let (cd, gz) = if tile.quantized {
            ((tile.bytes.len(), heap.len()), (0, heap.len()))
        } else {
            ((0, heap.len()), (tile.bytes.len(), heap.len()))
        };
        cd_desc.push(cd);
        gz_desc.push(gz);
        heap.extend_from_slice(&tile.bytes);
    }

    // Fixed table: per tile, the two `P` descriptors then the `ZSCALE`/`ZZERO`
    // doubles (row width 32), followed by the heap.
    out.clear();
    out.reserve(ntiles * 32 + heap.len());
    for t in 0..ntiles {
        // Two 32-bit `P` descriptors (COMPRESSED_DATA, GZIP_COMPRESSED_DATA) then
        // the ZSCALE/ZZERO doubles — the §10 quantized-float row layout.
        push_pq_descriptor(out, false, cd_desc[t].0 as u64, cd_desc[t].1 as u64);
        push_pq_descriptor(out, false, gz_desc[t].0 as u64, gz_desc[t].1 as u64);
        out.extend_from_slice(&zscale[t].to_be_bytes());
        out.extend_from_slice(&zzero[t].to_be_bytes());
    }
    out.extend_from_slice(&heap);

    let max_cd = cd_desc.iter().map(|&(n, _)| n).max().unwrap_or(0);
    let max_gz = gz_desc.iter().map(|&(n, _)| n).max().unwrap_or(0);
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .comment("XTENSION", "binary table extension");
    h.set("BITPIX", 8).set("NAXIS", 2);
    h.set("NAXIS1", 32).set("NAXIS2", ntiles as i64);
    h.set("PCOUNT", heap.len() as i64).set("GCOUNT", 1);
    h.set("TFIELDS", 4);
    h.set("TTYPE1", "COMPRESSED_DATA")
        .set("TFORM1", format!("1PB({max_cd})"));
    h.set("TTYPE2", "GZIP_COMPRESSED_DATA")
        .set("TFORM2", format!("1PB({max_gz})"));
    h.set("TTYPE3", "ZSCALE").set("TFORM3", "1D");
    h.set("TTYPE4", "ZZERO").set("TFORM4", "1D");
    set_zimage_axes(&mut h, cmptype, zbitpix, dims, &tiles);
    if cmptype == "RICE_1" {
        h.set("ZNAME1", "BLOCKSIZE").set("ZVAL1", 32);
        h.set("ZNAME2", "BYTEPIX").set("ZVAL2", 4);
    } else {
        // Tell the decoder the quantized integers are 4 bytes wide.
        h.set("ZNAME1", "BYTEPIX").set("ZVAL1", 4);
    }
    h.set("ZQUANTIZ", dither_name(method));
    h.set("ZDITHER0", zdither0);
    if any_null {
        // Quantized nulls are stored as this reserved integer; ZBLANK tells the
        // decoder which value maps back to a blank (NaN) pixel.
        h.set("ZBLANK", quantize::NULL_VALUE as i64);
    }
    Ok(h)
}

/// The `ZQUANTIZ` keyword string for a dither method.
fn dither_name(method: quantize::DitherMethod) -> &'static str {
    match method {
        quantize::DitherMethod::None => "NO_DITHER",
        quantize::DitherMethod::Subtractive1 => "SUBTRACTIVE_DITHER_1",
        quantize::DitherMethod::Subtractive2 => "SUBTRACTIVE_DITHER_2",
    }
}

/// Gather a tile's float pixels straight from the typed source into `out`,
/// widening to `f64` — so float encoding never materializes a whole-image `f64`
/// buffer. Integer sources yield nothing (they take the integer path).
fn gather_f64(samples: &ImageData, row_bases: &[usize], row_len: usize, out: &mut Vec<f64>) {
    out.clear();
    match samples {
        ImageData::F32(v) => {
            for &b in row_bases {
                out.extend(v[b..b + row_len].iter().map(|&x| x as f64));
            }
        }
        ImageData::F64(v) => {
            for &b in row_bases {
                out.extend_from_slice(&v[b..b + row_len]);
            }
        }
        _ => {}
    }
}

/// Encode `f64` values as a big-endian buffer of `bitpix`-width floats.
fn float_to_be(vals: &[f64], bitpix: Bitpix) -> Vec<u8> {
    match bitpix {
        Bitpix::F32 => encode_be(
            &vals.iter().map(|&v| v as f32).collect::<Vec<_>>(),
            f32::to_be_bytes,
        ),
        _ => encode_be(vals, f64::to_be_bytes),
    }
}

/// One tile's compressed payload: a byte stream (`1PB`) for most codecs, or an
/// i16 instruction list (`1PI`) for `PLIO_1`.
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

    /// Append the cell to the heap as big-endian bytes.
    fn extend_heap(&self, heap: &mut Vec<u8>) {
        match self {
            TileCell::Bytes(b) => heap.extend_from_slice(b),
            TileCell::I16(v) => {
                for &x in v {
                    heap.extend_from_slice(&x.to_be_bytes());
                }
            }
        }
    }
}

/// Resolve the tile shape for an image: the caller's `tile_shape` (each axis
/// clamped to ≥1) when it has the right rank, else row-tiling — the first axis
/// whole, the rest 1.
fn resolve_tile_shape(dims: &[usize], tile_shape: &[usize]) -> Vec<usize> {
    if tile_shape.len() == dims.len() {
        tile_shape.iter().map(|&t| t.max(1)).collect()
    } else {
        (0..dims.len())
            .map(|i| if i == 0 { dims[i].max(1) } else { 1 })
            .collect()
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
) {
    h.set("ZIMAGE", true)
        .comment("ZIMAGE", "this is a tiled-compressed image");
    h.set("ZCMPTYPE", cmptype);
    h.set("ZBITPIX", zbitpix.code());
    h.set("ZNAXIS", dims.len() as i64);
    for (i, &n) in dims.iter().enumerate() {
        h.set(key!("ZNAXIS{}", i + 1).as_str(), n as i64);
    }
    for (i, &t) in tiles.iter().enumerate() {
        h.set(key!("ZTILE{}", i + 1).as_str(), t as i64);
    }
}

/// Gather a tile's integer pixels straight from the typed source into `out`,
/// widening to `i64` — so integer encoding never materializes a whole-image `i64`
/// buffer. Float sources yield nothing (they take the quantized float path).
fn gather_i64(samples: &ImageData, row_bases: &[usize], row_len: usize, out: &mut Vec<i64>) {
    out.clear();
    match samples {
        ImageData::U8(v) => {
            for &b in row_bases {
                out.extend(v[b..b + row_len].iter().map(|&x| x as i64));
            }
        }
        ImageData::I16(v) => {
            for &b in row_bases {
                out.extend(v[b..b + row_len].iter().map(|&x| x as i64));
            }
        }
        ImageData::I32(v) => {
            for &b in row_bases {
                out.extend(v[b..b + row_len].iter().map(|&x| x as i64));
            }
        }
        ImageData::I64(v) => {
            for &b in row_bases {
                out.extend_from_slice(&v[b..b + row_len]);
            }
        }
        _ => {}
    }
}

/// Narrow + pack `i64` values to big-endian `bitpix`-width integers in `out`, in a
/// single pass (no intermediate narrowed `Vec`). `out` is cleared first, so it can
/// be a reused scratch buffer. Grows once then writes each `N`-byte slot, the
/// vectorizable shape [`extend_be`] uses.
fn i64_to_be_into(vals: &[i64], bitpix: Bitpix, out: &mut Vec<u8>) {
    out.clear();
    out.resize(vals.len() * bitpix.elem_size(), 0);
    match bitpix {
        Bitpix::U8 => {
            for (slot, &v) in out.iter_mut().zip(vals) {
                *slot = v as u8;
            }
        }
        Bitpix::I16 => {
            for (slot, &v) in out.chunks_exact_mut(2).zip(vals) {
                slot.copy_from_slice(&(v as i16).to_be_bytes());
            }
        }
        Bitpix::I32 => {
            for (slot, &v) in out.chunks_exact_mut(4).zip(vals) {
                slot.copy_from_slice(&(v as i32).to_be_bytes());
            }
        }
        Bitpix::I64 => {
            for (slot, &v) in out.chunks_exact_mut(8).zip(vals) {
                slot.copy_from_slice(&v.to_be_bytes());
            }
        }
        _ => {}
    }
}

/// Owning form of [`i64_to_be_into`], for the few sites that keep the bytes (the
/// `NOCOMPRESS` cell is stored verbatim, so it can't share the reused scratch).
fn i64_to_be(vals: &[i64], bitpix: Bitpix) -> Vec<u8> {
    let mut out = Vec::new();
    i64_to_be_into(vals, bitpix, &mut out);
    out
}

/// Read a compressed-data column's per-tile cells, or empty if the column is absent.
fn read_tiles(table: &BinTable, name: &str) -> Result<Vec<ColumnData>> {
    match table.column_index(name) {
        Some(c) => table.read_vla_column(c),
        None => Ok(Vec::new()),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: &BinTable, name: &str) -> Option<Vec<f64>> {
    let c = table.column_index(name)?;
    match table.read_column(c) {
        Ok(ColumnData::F64(v)) => Some(v),
        _ => None,
    }
}

/// Read a per-tile integer column (e.g. a `ZBLANK` column), widening any integer
/// `TFORM` to `i64`, or `None` if absent.
fn read_i64_column(table: &BinTable, name: &str) -> Option<Vec<i64>> {
    let c = table.column_index(name)?;
    match table.read_column(c) {
        Ok(ColumnData::Bytes(v)) => Some(v.iter().map(|&x| x as i64).collect()),
        Ok(ColumnData::I16(v)) => Some(v.iter().map(|&x| x as i64).collect()),
        Ok(ColumnData::I32(v)) => Some(v.iter().map(|&x| x as i64).collect()),
        Ok(ColumnData::I64(v)) => Some(v),
        _ => None,
    }
}

fn column_at<T: Copy>(col: &Option<Vec<T>>, t: usize) -> Option<T> {
    col.as_ref().and_then(|v| v.get(t).copied())
}

/// Decode one tile, honoring the fallback columns: the primary `COMPRESSED_DATA`
/// (via `ZCMPTYPE`), else gzip'd `GZIP_COMPRESSED_DATA`, else raw `UNCOMPRESSED_DATA`.
/// The three per-tile source columns (§10.1.3): the primary `COMPRESSED_DATA` and
/// the `GZIP_COMPRESSED_DATA` / `UNCOMPRESSED_DATA` fallbacks.
#[derive(Debug, Clone, Copy)]
struct TileColumns<'a> {
    primary: Option<&'a ColumnData>,
    gzip: Option<&'a ColumnData>,
    uncompressed: Option<&'a ColumnData>,
}

/// The resolved source for one tile — which non-empty column holds its bytes.
enum TileSource<'a> {
    Compressed(&'a ColumnData),
    Gzip(&'a ColumnData),
    Uncompressed(&'a ColumnData),
}

impl<'a> TileColumns<'a> {
    /// Pick the first non-empty source: primary `COMPRESSED_DATA`, then the
    /// gzip and uncompressed fallbacks; error if every column's cell is empty.
    fn resolve(&self) -> Result<TileSource<'a>> {
        if let Some(c) = self.primary.filter(|c| cell_len(c) > 0) {
            Ok(TileSource::Compressed(c))
        } else if let Some(c) = self.gzip.filter(|c| cell_len(c) > 0) {
            Ok(TileSource::Gzip(c))
        } else if let Some(c) = self.uncompressed.filter(|c| cell_len(c) > 0) {
            Ok(TileSource::Uncompressed(c))
        } else {
            Err(FitsError::UnsupportedCompression {
                name: "empty tile (no compressed or uncompressed data)".to_string(),
            })
        }
    }
}

/// The codec knobs from `ZNAMEi`/`ZVALi`: Rice block size & pixel width, and the
/// HCOMPRESS `SMOOTH` flag.
#[derive(Debug, Clone, Copy)]
struct CodecParams {
    blocksize: usize,
    bytepix: usize,
    smooth: bool,
}

/// Per-tile float dequantization parameters (§10.2): `physical = zero + scale·I`,
/// the dither method/seed, and the integer null sentinel.
#[derive(Debug)]
struct Dequant {
    scale: f64,
    zero: f64,
    method: quantize::DitherMethod,
    irow: i64,
    zblank: Option<i64>,
}

/// The decode parameters constant across all of a tiled image's tiles: the codec
/// name, the stored/quantized integer bitpix (and float `ZBITPIX`), and the codec
/// knobs. Bundled so the per-tile decode helpers take one context rather than a long
/// parameter list.
struct DecodeCtx<'a> {
    cmptype: &'a str,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    codec: CodecParams,
}

fn decode_one_tile_into(
    ctx: &DecodeCtx,
    cols: TileColumns,
    tile_elems: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => decode_tile_cell_into(ctx, cell, tile_elems, out),
        TileSource::Gzip(cell) => gzip::gzip_tile_into(as_bytes(cell)?, ctx.int_bitpix, out),
        TileSource::Uncompressed(cell) => {
            cell_to_i64_into(cell, out);
            Ok(())
        }
    }
}

/// Decode one tile of a *float* image into `out`. A primary `COMPRESSED_DATA` cell
/// holds quantized integers (decoded into the reused `ints` buffer, then dequantized
/// as `scale·int + zero`); otherwise the `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA`
/// fallbacks hold the raw float values.
fn decode_float_tile_into(
    ctx: &DecodeCtx,
    cols: TileColumns,
    tile_elems: usize,
    dq: Dequant,
    out: &mut Vec<f64>,
    ints: &mut Vec<i64>,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => {
            // Quantized integers (float images never use HCOMPRESS).
            decode_tile_cell_into(ctx, cell, tile_elems, ints)?;
            quantize::dequantize_into(ints, dq.scale, dq.zero, dq.method, dq.irow, dq.zblank, out);
            Ok(())
        }
        TileSource::Gzip(cell) => {
            be_floats_into(&gzip::gunzip(as_bytes(cell)?)?, ctx.zbitpix, out);
            Ok(())
        }
        TileSource::Uncompressed(cell) => {
            cell_to_f64_into(cell, ctx.zbitpix, out);
            Ok(())
        }
    }
}

/// Decode one tile's primary `COMPRESSED_DATA` cell into `tile_elems` integer values
/// in `out`, per `ZCMPTYPE`. The cell is a byte array except for `PLIO_1` (i16).
fn decode_tile_cell_into(
    ctx: &DecodeCtx,
    cell: &ColumnData,
    tile_elems: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    let codec = ctx.codec;
    match ctx.cmptype {
        "GZIP_1" => gzip::gzip_tile_into(as_bytes(cell)?, ctx.int_bitpix, out),
        "GZIP_2" => gzip::gzip2_tile_into(as_bytes(cell)?, ctx.int_bitpix, out),
        "RICE_1" => {
            // Only 1/2/4-byte pixels are defined (cfitsio parity). A `BYTEPIX` of
            // 3/5/6/7 from an untrusted header would otherwise decode with mismatched
            // `fsbits`/mask and emit garbage instead of erroring.
            if !matches!(codec.bytepix, 1 | 2 | 4) {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("RICE_1 with BYTEPIX = {} (only 1/2/4)", codec.bytepix),
                });
            }
            rice::rice_decode_into(
                as_bytes(cell)?,
                tile_elems,
                codec.bytepix,
                codec.blocksize,
                out,
            );
            Ok(())
        }
        "PLIO_1" => {
            plio::plio_decode_into(as_i16(cell)?, tile_elems, out);
            Ok(())
        }
        "HCOMPRESS_1" => {
            hcompress::hcompress_tile_into(as_bytes(cell)?, codec.smooth, tile_elems, out)
        }
        // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
        "NOCOMPRESS" => {
            be_to_i64_into(as_bytes(cell)?, ctx.int_bitpix, out);
            Ok(())
        }
        other => Err(FitsError::UnsupportedCompression {
            name: other.to_string(),
        }),
    }
}

/// HCOMPRESS smoothing flag: the `SMOOTH` `ZVALn` is non-zero (cfitsio applies
/// inverse-transform smoothing to suppress blocking in lossy images).
fn hcompress_smooth(header: &Header) -> bool {
    let mut i = 1;
    while let Some(name) = header.get_text(key!("ZNAME{i}").as_str()) {
        if name == "SMOOTH" {
            return header.get_integer(key!("ZVAL{i}").as_str()).unwrap_or(0) != 0;
        }
        i += 1;
    }
    false
}

/// The tiling of an N-d image: axis lengths, per-axis tile sizes, and the derived
/// strides and per-axis tile counts. Iterating `0..ntiles()` and calling `tile(t)`
/// yields each tile's clipped dimensions and flat pixel indices — the geometry
/// shared by image decompress and both encoders.
#[derive(Debug)]
struct TileGeometry {
    dims: Vec<usize>,
    tiles: Vec<usize>,
    stride: Vec<usize>,
    ntiles_axis: Vec<usize>,
}

/// Reusable per-tile scratch, filled by [`TileGeometry::tile_into`] each
/// iteration so a tile loop allocates these buffers once instead of per tile
/// (the `indices` buffer is `tile_elems` long — the dominant per-tile cost).
#[derive(Debug, Default)]
struct TileScratch {
    /// Per-axis origin of the current tile (scratch for the index computation).
    origin: Vec<usize>,
    /// Edge-clipped per-axis extent of the tile (`ny` fastest); used by HCOMPRESS.
    tdims: Vec<usize>,
    /// Flat start of each contiguous tile row in the full image (length = the product
    /// of `tdims[1..]`). Axis 0 has stride 1, so a row is `row_len` contiguous
    /// elements — gather/scatter copy it as a slice instead of per-pixel indexing.
    row_bases: Vec<usize>,
    /// Elements per row (`tdims[0]`): the fastest-axis extent.
    row_len: usize,
    /// Per-axis local coordinate, the odometer state [`TileGeometry::tile_into`]
    /// walks (over the higher axes) to emit `row_bases` without per-pixel division.
    coord: Vec<usize>,
}

impl TileScratch {
    /// Total pixels in the current tile (`row_len × nrows`).
    fn nelem(&self) -> usize {
        self.row_len * self.row_bases.len()
    }
}

impl TileGeometry {
    fn new(dims: &[usize], tiles: &[usize]) -> TileGeometry {
        let n = dims.len();
        let ntiles_axis = dims
            .iter()
            .zip(tiles)
            .map(|(&d, &t)| d.div_ceil(t))
            .collect();
        let mut stride = vec![1usize; n];
        for i in 1..n {
            stride[i] = stride[i - 1] * dims[i - 1];
        }
        TileGeometry {
            dims: dims.to_vec(),
            tiles: tiles.to_vec(),
            stride,
            ntiles_axis,
        }
    }

    fn ntiles(&self) -> usize {
        self.ntiles_axis.iter().product()
    }

    /// Fill `s` (reusing its buffers) with tile `t`'s edge-clipped extent and the
    /// flat indices of its pixels in the full image.
    fn tile_into(&self, t: usize, s: &mut TileScratch) {
        let n = self.dims.len();
        s.origin.clear();
        s.tdims.clear();
        let mut rem = t;
        for i in 0..n {
            let ti = rem % self.ntiles_axis[i];
            rem /= self.ntiles_axis[i];
            let origin = ti * self.tiles[i];
            s.origin.push(origin);
            s.tdims.push(self.tiles[i].min(self.dims[i] - origin));
        }
        // Axis 0 has stride 1, so each tile row is `row_len` contiguous elements and the
        // tile is `nrows` such rows (the product of the higher-axis extents). Emit only
        // the row starts — walking the *higher* axes as an odometer, `flat` maintained
        // by stride adds with no per-pixel division — and let gather/scatter copy each
        // row as a contiguous slice. The row order matches a row-major pixel walk, so
        // decoded values still land in the right pixels.
        s.row_len = if n == 0 { 1 } else { s.tdims[0] };
        let nrows: usize = if n <= 1 {
            1
        } else {
            s.tdims[1..].iter().product()
        };
        let mut flat: usize = (0..n).map(|i| s.origin[i] * self.stride[i]).sum();
        s.row_bases.clear();
        s.row_bases.reserve(nrows);
        s.coord.clear();
        s.coord.resize(n, 0);
        for _ in 0..nrows {
            s.row_bases.push(flat);
            for i in 1..n {
                s.coord[i] += 1;
                flat += self.stride[i];
                if s.coord[i] < s.tdims[i] {
                    break;
                }
                s.coord[i] = 0;
                flat -= s.tdims[i] * self.stride[i];
            }
        }
    }
}

/// Read `PREFIX1..PREFIXn` integer axis lengths.
fn read_axes(header: &Header, prefix: &str, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| match header.get_integer(key!("{prefix}{i}").as_str()) {
            Some(v) if v >= 0 => Ok(v as usize),
            Some(_) => Err(FitsError::KeywordOutOfRange { name: "ZNAXISn" }),
            None => Err(FitsError::MissingKeyword { name: "ZNAXISn" }),
        })
        .collect()
}

fn cell_len(cell: &ColumnData) -> usize {
    match cell {
        ColumnData::Bytes(v) => v.len(),
        ColumnData::I16(v) => v.len(),
        ColumnData::I32(v) => v.len(),
        ColumnData::I64(v) => v.len(),
        _ => 0,
    }
}

fn as_bytes(cell: &ColumnData) -> Result<&[u8]> {
    match cell {
        ColumnData::Bytes(b) => Ok(b),
        _ => Err(FitsError::UnsupportedCompression {
            name: "compressed cell is not a byte array".to_string(),
        }),
    }
}

fn as_i16(cell: &ColumnData) -> Result<&[i16]> {
    match cell {
        ColumnData::I16(v) => Ok(v),
        _ => Err(FitsError::UnsupportedCompression {
            name: "PLIO_1 data is not an i16 list".to_string(),
        }),
    }
}

/// Widen a raw (`UNCOMPRESSED_DATA`) tile cell to `i64` values.
fn cell_to_i64_into(cell: &ColumnData, out: &mut Vec<i64>) {
    out.clear();
    match cell {
        ColumnData::Bytes(v) => out.extend(v.iter().map(|&b| b as i64)),
        ColumnData::I16(v) => out.extend(v.iter().map(|&x| x as i64)),
        ColumnData::I32(v) => out.extend(v.iter().map(|&x| x as i64)),
        ColumnData::I64(v) => out.extend_from_slice(v),
        _ => {}
    }
}

/// Widen a raw (`UNCOMPRESSED_DATA`) float tile cell to `f64` in `out`.
fn cell_to_f64_into(cell: &ColumnData, zbitpix: Bitpix, out: &mut Vec<f64>) {
    out.clear();
    match cell {
        ColumnData::F32(v) => out.extend(v.iter().map(|&x| x as f64)),
        ColumnData::F64(v) => out.extend_from_slice(v),
        ColumnData::Bytes(b) => be_floats_into(b, zbitpix, out),
        _ => {}
    }
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values in `out`
/// (cleared first). Single pass — no intermediate narrowed `Vec`; the
/// `from_be_bytes` + `as i64` closure inlines and vectorizes like `decode_be`.
fn be_to_i64_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<i64>) {
    out.clear();
    match bitpix {
        Bitpix::U8 => out.extend(bytes.iter().map(|&b| b as i64)),
        Bitpix::I16 => out.extend(
            bytes
                .chunks_exact(2)
                .map(|c| i16::from_be_bytes(c.try_into().unwrap()) as i64),
        ),
        Bitpix::I32 => out.extend(
            bytes
                .chunks_exact(4)
                .map(|c| i32::from_be_bytes(c.try_into().unwrap()) as i64),
        ),
        Bitpix::I64 => out.extend(
            bytes
                .chunks_exact(8)
                .map(|c| i64::from_be_bytes(c.try_into().unwrap())),
        ),
        Bitpix::F32 | Bitpix::F64 => {} // excluded before this point
    }
}

/// Decode a big-endian buffer of `bitpix` floats into `f64` in `out`, widening in one
/// pass.
fn be_floats_into(bytes: &[u8], bitpix: Bitpix, out: &mut Vec<f64>) {
    out.clear();
    match bitpix {
        Bitpix::F32 => out.extend(
            bytes
                .chunks_exact(4)
                .map(|c| f32::from_be_bytes(c.try_into().unwrap()) as f64),
        ),
        Bitpix::F64 => out.extend(
            bytes
                .chunks_exact(8)
                .map(|c| f64::from_be_bytes(c.try_into().unwrap())),
        ),
        _ => {}
    }
}

fn bytepix_to_bitpix(bytepix: usize) -> Bitpix {
    match bytepix {
        1 => Bitpix::U8,
        2 => Bitpix::I16,
        8 => Bitpix::I64,
        _ => Bitpix::I32,
    }
}

/// A zeroed typed sample buffer of `len` elements — the decompression output the
/// tiles scatter into (narrowing as they land), so there is no whole-image `i64`
/// or `f64` intermediate to narrow afterwards.
fn zeroed_samples(bitpix: Bitpix, len: usize) -> ImageData {
    match bitpix {
        Bitpix::U8 => ImageData::U8(vec![0; len]),
        Bitpix::I16 => ImageData::I16(vec![0; len]),
        Bitpix::I32 => ImageData::I32(vec![0; len]),
        Bitpix::I64 => ImageData::I64(vec![0; len]),
        Bitpix::F32 => ImageData::F32(vec![0.0; len]),
        Bitpix::F64 => ImageData::F64(vec![0.0; len]),
    }
}

#[cfg(test)]
mod tests;
