//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`](super::gzip)/[`rice`](super::rice)/[`plio`](super::plio)/
//! [`hcompress`](super::hcompress) modules; this drives the tile geometry, the
//! fallback-column resolution, and the narrow-and-scatter into the output plane.

use super::convert::as_bytes;
use super::convert::as_i16;
use super::convert::be_floats_into;
use super::convert::be_to_i64_into;
use super::convert::bytepix_to_bitpix;
use super::convert::cell_len;
use super::convert::cell_to_f64_into;
use super::convert::cell_to_i64_into;
use super::convert::zeroed_samples;
use super::geometry::TileGeometry;
use super::geometry::TileScratch;
#[cfg(feature = "parallel")]
use super::map_tiles;
use super::{DitherMethod, ImageCodec};
use super::{gzip, hcompress, plio, quantize, rice};

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::ColumnData;

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
    let dims = read_axes(header, znaxis)?;
    // A `ZNAXIS = 0` ZIMAGE has no data array (as an uncompressed `NAXIS = 0` does).
    // Return empty before building the geometry, which would otherwise size `total`
    // as the empty product (1) and fabricate a phantom one-pixel tile.
    if dims.is_empty() {
        return Ok(Image {
            shape: dims,
            samples: zeroed_samples(zbitpix, 0)?,
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
    if total == 0 {
        return Ok(Image {
            shape: dims,
            samples: zeroed_samples(zbitpix, 0)?,
            scaling: Scaling::from_header(header),
        });
    }
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
        "NO_DITHER" => DitherMethod::None,
        "SUBTRACTIVE_DITHER_1" => DitherMethod::Subtractive1,
        "SUBTRACTIVE_DITHER_2" => DitherMethod::Subtractive2,
        other => {
            if is_float {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("float quantization {other}"),
                });
            }
            DitherMethod::None
        }
    };
    let zdither0 = header.get_integer("ZDITHER0").unwrap_or(1);
    // ZBLANK may be a keyword (constant) or a per-tile column; §10.1.3 says the
    // column value wins where present.
    let zblank_keyword = header.get_integer("ZBLANK");
    let zblank_column = read_i64_column(table, "ZBLANK");
    let smooth = hcompress_smooth(header);
    let params = CodecParams {
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
    let mut samples = zeroed_samples(zbitpix, total)?;

    // Decode and scatter each tile in one fused pass — parallel under the `parallel`
    // feature, where tiles write disjoint regions of `samples` concurrently (they
    // partition the image). Each value is narrowed to `ZBITPIX` as it lands, so there
    // is no whole-image `i64`/`f64` intermediate and no separate serial scatter tail.
    let ctx = DecodeCtx {
        codec: ImageCodec::parse(&cmptype)?,
        zbitpix,
        int_bitpix,
        params,
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
            ensure_tile_size(scratch.nelem(), vals.len())?;
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
            ensure_tile_size(scratch.nelem(), vals.len())?;
            scatter_rows(out, &scratch.row_bases, scratch.row_len, &vals, &convert);
        }
        Ok(())
    }
}

/// Scatter `vals` (the tile's pixels in row-major order) into `out` one contiguous
/// row at a time: `row_len` values land at each `row_bases` offset, narrowed by
/// `convert`.
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
        for (d, &v) in out[base..base + row_len]
            .iter_mut()
            .zip(&vals[off..off + row_len])
        {
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
#[derive(Debug)]
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
    /// each `row_bases` offset, narrowed by `convert`.
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
            assert!(
                base + row_len <= self.len,
                "tile row out of bounds {}",
                self.len
            );
            // SAFETY: `[base, base + row_len)` is in bounds (asserted; guaranteed by
            // the tile geometry) and disjoint across tiles, so these are non-aliasing
            // in-bounds writes over one contiguous run.
            let dst = unsafe { std::slice::from_raw_parts_mut(self.ptr.add(base), row_len) };
            for (d, &v) in dst.iter_mut().zip(&vals[off..off + row_len]) {
                *d = convert(v);
            }
            off += row_len;
        }
    }
}

/// Read a compressed-data column's per-tile cells, or empty if the column is absent.
fn read_tiles(table: &BinTable, name: &str) -> Result<Vec<ColumnData>> {
    match table.column_index(name) {
        Some(c) => table.column_by_idx(c)?.vla(),
        None => Ok(Vec::new()),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: &BinTable, name: &str) -> Option<Vec<f64>> {
    let c = table.column_index(name)?;
    match table.column_by_idx(c).and_then(|col| col.raw()) {
        Ok(ColumnData::F64(v)) => Some(v),
        _ => None,
    }
}

/// Read a per-tile integer column (e.g. a `ZBLANK` column), widening any integer
/// `TFORM` to `i64`, or `None` if absent.
fn read_i64_column(table: &BinTable, name: &str) -> Option<Vec<i64>> {
    let c = table.column_index(name)?;
    match table.column_by_idx(c).and_then(|col| col.raw()) {
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
#[derive(Debug)]
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
    method: DitherMethod,
    irow: i64,
    zblank: Option<i64>,
}

/// The decode parameters constant across all of a tiled image's tiles: the codec,
/// the stored/quantized integer bitpix (and float `ZBITPIX`), and the codec knobs.
/// Bundled so the per-tile decode helpers take one context rather than a long
/// parameter list.
#[derive(Debug)]
struct DecodeCtx {
    codec: ImageCodec,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    params: CodecParams,
}

fn decode_one_tile_into(
    ctx: &DecodeCtx,
    cols: TileColumns,
    tile_elems: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => decode_tile_cell_into(ctx, cell, tile_elems, out),
        TileSource::Gzip(cell) => {
            gzip::gzip_tile_into(as_bytes(cell)?, ctx.int_bitpix, tile_elems, out)
        }
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
            // Raw floats, bounded at the tile's known byte size (`tile_elems` floats).
            let max = tile_elems.saturating_mul(ctx.zbitpix.elem_size());
            be_floats_into(&gzip::gunzip(as_bytes(cell)?, max)?, ctx.zbitpix, out);
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
    let params = ctx.params;
    match ctx.codec {
        ImageCodec::Gzip1 => gzip::gzip_tile_into(as_bytes(cell)?, ctx.int_bitpix, tile_elems, out),
        ImageCodec::Gzip2 => {
            gzip::gzip2_tile_into(as_bytes(cell)?, ctx.int_bitpix, tile_elems, out)
        }
        ImageCodec::Rice1 => {
            // Only 1/2/4-byte pixels are defined (cfitsio parity). A `BYTEPIX` of
            // 3/5/6/7 from an untrusted header would otherwise decode with mismatched
            // `fsbits`/mask and emit garbage instead of erroring.
            if !matches!(params.bytepix, 1 | 2 | 4) {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("RICE_1 with BYTEPIX = {} (only 1/2/4)", params.bytepix),
                });
            }
            rice::rice_decode_into(
                as_bytes(cell)?,
                tile_elems,
                params.bytepix,
                params.blocksize,
                out,
            )
        }
        ImageCodec::Plio1 => plio::plio_decode_into(as_i16(cell)?, tile_elems, out),
        ImageCodec::Hcompress1 => {
            hcompress::hcompress_tile_into(as_bytes(cell)?, params.smooth, tile_elems, out)
        }
        // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
        ImageCodec::NoCompress => {
            let bytes = as_bytes(cell)?;
            let expected = tile_elems
                .checked_mul(ctx.int_bitpix.elem_size())
                .ok_or(FitsError::DataUnitOverflow)?;
            if bytes.len() != expected {
                return Err(FitsError::DataSizeMismatch {
                    expected,
                    got: bytes.len(),
                });
            }
            be_to_i64_into(bytes, ctx.int_bitpix, out);
            Ok(())
        }
    }
}

fn ensure_tile_size(expected: usize, got: usize) -> Result<()> {
    if got != expected {
        return Err(FitsError::DataSizeMismatch { expected, got });
    }
    Ok(())
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

/// Read the `ZNAXIS1..ZNAXISn` integer axis lengths.
fn read_axes(header: &Header, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| match header.get_integer(key!("ZNAXIS{i}").as_str()) {
            Some(v) if v >= 0 => Ok(v as usize),
            Some(_) => Err(FitsError::KeywordOutOfRange { name: "ZNAXISn" }),
            None => Err(FitsError::MissingKeyword { name: "ZNAXISn" }),
        })
        .collect()
}
