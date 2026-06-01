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
use crate::endian::decode_be;
use crate::endian::encode_be;
use crate::endian::push_pq_descriptor;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::ColumnData;

/// A header and its data unit — what the (de)compression entry points return
/// (a named result instead of a bare `(Header, Vec<u8>)` tuple).
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
        .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })? as usize;
    let dims = read_axes(header, "ZNAXIS", znaxis)?;
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

    // Decode every tile (the compute-bound, independent step — parallel under the
    // `parallel` feature), collecting the per-tile values in tile order. Scatter them
    // into the typed output afterwards, narrowing each value to `ZBITPIX` as it lands
    // — there is no whole-image `i64`/`f64` intermediate. Tiles partition the image,
    // so a tile's positions are recomputed from the geometry and never overlap.
    let mut samples = zeroed_samples(zbitpix, total);
    if is_float {
        let tile_vals = map_tiles(ntiles, TileScratch::default, |scratch, t| {
            geom.tile_into(t, scratch);
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
            decode_float_tile(
                &cmptype,
                cols,
                scratch.indices.len(),
                zbitpix,
                int_bitpix,
                codec,
                dq,
            )
        })?;
        let mut scratch = TileScratch::default();
        for (t, vals) in tile_vals.iter().enumerate() {
            geom.tile_into(t, &mut scratch);
            scatter_f64(&mut samples, &scratch.indices, vals);
        }
    } else {
        let tile_vals = map_tiles(ntiles, TileScratch::default, |scratch, t| {
            geom.tile_into(t, scratch);
            let cols = TileColumns {
                primary: primary.get(t),
                gzip: gzip_fallback.get(t),
                uncompressed: uncompressed.get(t),
            };
            decode_one_tile(&cmptype, cols, scratch.indices.len(), int_bitpix, codec)
        })?;
        let mut scratch = TileScratch::default();
        for (t, vals) in tile_vals.iter().enumerate() {
            geom.tile_into(t, &mut scratch);
            scatter_i64(&mut samples, &scratch.indices, vals);
        }
    }
    Ok(Image {
        shape: dims,
        samples,
        scaling: Scaling::from_header(header),
    })
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
}

/// Encode an integer [`Image`] as a tiled-compressed `BINTABLE`: returns the
/// `ZIMAGE` header and the data unit (per-tile `P` descriptors + the heap of
/// compressed tile bytes). `tile_shape` of the wrong length falls back to
/// row-tiling. Float images and codecs without an encoder are rejected.
pub(crate) fn encode_image(
    image: &Image,
    cmptype: &str,
    tile_shape: &[usize],
    scale: i32,
    out: &mut Vec<u8>,
) -> Result<Header> {
    let bitpix = image.samples.bitpix();
    if bitpix.is_float() {
        return encode_float_image(image, cmptype, tile_shape, out);
    }
    // RICE handles only 1/2/4-byte pixels (cfitsio parity); refuse the 64-bit path
    // rather than silently corrupting. Table 37 lists BYTEPIX 8 as permitted, but
    // neither this encoder nor the decoder implements the 64-bit bitstream.
    if cmptype == "RICE_1" && bitpix.elem_size() > 4 {
        return Err(FitsError::UnsupportedCompression {
            name: "RICE_1 with BYTEPIX > 4 (64-bit pixels)".to_string(),
        });
    }
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, tile_shape);

    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = geom.ntiles();
    let bytepix = bitpix.elem_size();

    // Compress every tile independently (the compute-bound step — parallel under
    // the `parallel` feature). The heap layout is sequential (each descriptor's
    // offset is the running heap length), so concatenate the cells serially after.
    let cells = map_tiles(ntiles, EncodeScratch::default, |s, t| -> Result<TileCell> {
        geom.tile_into(t, &mut s.tile);
        // Gather + widen this tile's pixels straight from the typed source — no
        // whole-image `i64` buffer.
        gather_i64(&image.samples, &s.tile.indices, &mut s.ints);
        let vals = &s.ints;
        Ok(match cmptype {
            "GZIP_1" => TileCell::Bytes(gzip::gzip_encode(&i64_to_be(vals, bitpix))),
            "GZIP_2" => TileCell::Bytes(gzip::gzip2_encode(&i64_to_be(vals, bitpix), bytepix)),
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
    tile_shape: &[usize],
    out: &mut Vec<u8>,
) -> Result<Header> {
    if !matches!(cmptype, "GZIP_1" | "GZIP_2" | "RICE_1") {
        return Err(FitsError::UnsupportedCompression {
            name: format!("{cmptype} for float images (write)"),
        });
    }
    let zbitpix = image.samples.bitpix();
    let dims = &image.shape;
    let tiles = resolve_tile_shape(dims, tile_shape);

    let geom = TileGeometry::new(dims, &tiles);
    let ntiles = geom.ntiles();

    let zdither0 = 1i64; // deterministic dither seed (any 1..=10000 is valid)
    let int_bitpix = Bitpix::I32; // quantized planes are always int32
    let method = quantize::DitherMethod::Subtractive1; // cfitsio's default

    // Quantize + compress each tile independently (the compute-bound step —
    // parallel under the `parallel` feature); the §10 row layout and heap offsets
    // are assembled serially after, since they are sequential.
    let tiles_out = map_tiles(
        ntiles,
        EncodeScratch::default,
        |s, t| -> Result<FloatTile> {
            geom.tile_into(t, &mut s.tile);
            let nx = s.tile.tdims[0];
            let ny = s.tile.indices.len() / nx;
            // Gather + widen this tile's pixels straight from the typed source.
            gather_f64(&image.samples, &s.tile.indices, &mut s.floats);
            let irow = t as i64 + zdither0; // = (1-based tile row) + ZDITHER0 - 1
            Ok(
                match quantize::quantize_tile(&s.floats, nx, ny, 0.0, method, irow) {
                    Some(q) => {
                        s.ints.clear();
                        s.ints.extend(q.idata.iter().map(|&v| v as i64));
                        let bytes = match cmptype {
                            "GZIP_1" => gzip::gzip_encode(&i64_to_be(&s.ints, int_bitpix)),
                            "GZIP_2" => gzip::gzip2_encode(&i64_to_be(&s.ints, int_bitpix), 4),
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
                        bytes: gzip::gzip_encode(&float_to_be(&s.floats, zbitpix)),
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
fn gather_f64(samples: &ImageData, indices: &[usize], out: &mut Vec<f64>) {
    out.clear();
    match samples {
        ImageData::F32(v) => out.extend(indices.iter().map(|&i| v[i] as f64)),
        ImageData::F64(v) => out.extend(indices.iter().map(|&i| v[i])),
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
fn gather_i64(samples: &ImageData, indices: &[usize], out: &mut Vec<i64>) {
    out.clear();
    match samples {
        ImageData::U8(v) => out.extend(indices.iter().map(|&i| v[i] as i64)),
        ImageData::I16(v) => out.extend(indices.iter().map(|&i| v[i] as i64)),
        ImageData::I32(v) => out.extend(indices.iter().map(|&i| v[i] as i64)),
        ImageData::I64(v) => out.extend(indices.iter().map(|&i| v[i])),
        _ => {}
    }
}

/// Encode `i64` values as a big-endian buffer of `bitpix`-width integers.
fn i64_to_be(vals: &[i64], bitpix: Bitpix) -> Vec<u8> {
    match bitpix {
        Bitpix::U8 => vals.iter().map(|&v| v as u8).collect(),
        Bitpix::I16 => encode_be(
            &vals.iter().map(|&v| v as i16).collect::<Vec<_>>(),
            i16::to_be_bytes,
        ),
        Bitpix::I32 => encode_be(
            &vals.iter().map(|&v| v as i32).collect::<Vec<_>>(),
            i32::to_be_bytes,
        ),
        Bitpix::I64 => encode_be(vals, i64::to_be_bytes),
        _ => Vec::new(),
    }
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

fn decode_one_tile(
    cmptype: &str,
    cols: TileColumns,
    tile_elems: usize,
    int_bitpix: Bitpix,
    codec: CodecParams,
) -> Result<Vec<i64>> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => {
            decode_tile_cell(cmptype, cell, tile_elems, int_bitpix, codec)
        }
        TileSource::Gzip(cell) => gzip::gzip_tile(as_bytes(cell)?, int_bitpix),
        TileSource::Uncompressed(cell) => Ok(cell_to_i64(cell)),
    }
}

/// Decode one tile of a *float* image. A primary `COMPRESSED_DATA` cell holds
/// quantized integers (dequantized as `scale·int + zero`); otherwise the
/// `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks hold the raw float values.
fn decode_float_tile(
    cmptype: &str,
    cols: TileColumns,
    tile_elems: usize,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    codec: CodecParams,
    dq: Dequant,
) -> Result<Vec<f64>> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => {
            // Quantized integers (float images never use HCOMPRESS).
            let ints = decode_tile_cell(cmptype, cell, tile_elems, int_bitpix, codec)?;
            Ok(quantize::dequantize(
                &ints, dq.scale, dq.zero, dq.method, dq.irow, dq.zblank,
            ))
        }
        TileSource::Gzip(cell) => Ok(be_floats(&gzip::gunzip(as_bytes(cell)?)?, zbitpix)),
        TileSource::Uncompressed(cell) => Ok(cell_to_f64(cell, zbitpix)),
    }
}

/// Decode one tile's primary `COMPRESSED_DATA` cell into `tile_elems` integer
/// values, per `ZCMPTYPE`. The cell is a byte array except for `PLIO_1` (i16).
fn decode_tile_cell(
    cmptype: &str,
    cell: &ColumnData,
    tile_elems: usize,
    int_bitpix: Bitpix,
    codec: CodecParams,
) -> Result<Vec<i64>> {
    match cmptype {
        "GZIP_1" => gzip::gzip_tile(as_bytes(cell)?, int_bitpix),
        "GZIP_2" => gzip::gzip2_tile(as_bytes(cell)?, int_bitpix),
        "RICE_1" => {
            if codec.bytepix > 4 {
                return Err(FitsError::UnsupportedCompression {
                    name: "RICE_1 with BYTEPIX > 4 (64-bit pixels)".to_string(),
                });
            }
            Ok(rice::rice_decode(
                as_bytes(cell)?,
                tile_elems,
                codec.bytepix,
                codec.blocksize,
            ))
        }
        "PLIO_1" => Ok(plio::plio_decode(as_i16(cell)?, tile_elems)),
        "HCOMPRESS_1" => hcompress::hcompress_tile(as_bytes(cell)?, codec.smooth, tile_elems),
        // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
        "NOCOMPRESS" => Ok(be_to_i64(as_bytes(cell)?, int_bitpix)),
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
    /// Flat (row-major) indices of the tile's pixels in the full image.
    indices: Vec<usize>,
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
        let tile_elems: usize = s.tdims.iter().product();
        s.indices.clear();
        s.indices.reserve(tile_elems);
        for local in 0..tile_elems {
            let mut rem = local;
            let mut flat = 0;
            for i in 0..n {
                let c = rem % s.tdims[i];
                rem /= s.tdims[i];
                flat += (s.origin[i] + c) * self.stride[i];
            }
            s.indices.push(flat);
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
fn cell_to_i64(cell: &ColumnData) -> Vec<i64> {
    match cell {
        ColumnData::Bytes(v) => v.iter().map(|&b| b as i64).collect(),
        ColumnData::I16(v) => v.iter().map(|&x| x as i64).collect(),
        ColumnData::I32(v) => v.iter().map(|&x| x as i64).collect(),
        ColumnData::I64(v) => v.clone(),
        _ => Vec::new(),
    }
}

/// Widen a raw (`UNCOMPRESSED_DATA`) float tile cell to `f64`.
fn cell_to_f64(cell: &ColumnData, zbitpix: Bitpix) -> Vec<f64> {
    match cell {
        ColumnData::F32(v) => v.iter().map(|&x| x as f64).collect(),
        ColumnData::F64(v) => v.clone(),
        ColumnData::Bytes(b) => be_floats(b, zbitpix),
        _ => Vec::new(),
    }
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values.
fn be_to_i64(bytes: &[u8], bitpix: Bitpix) -> Vec<i64> {
    match bitpix {
        Bitpix::U8 => decode_be(bytes, u8::from_be_bytes)
            .iter()
            .map(|&x| x as i64)
            .collect(),
        Bitpix::I16 => decode_be(bytes, i16::from_be_bytes)
            .iter()
            .map(|&x| x as i64)
            .collect(),
        Bitpix::I32 => decode_be(bytes, i32::from_be_bytes)
            .iter()
            .map(|&x| x as i64)
            .collect(),
        Bitpix::I64 => decode_be(bytes, i64::from_be_bytes),
        Bitpix::F32 | Bitpix::F64 => Vec::new(), // excluded before this point
    }
}

/// Decode a big-endian buffer of `bitpix` floats into `f64`.
fn be_floats(bytes: &[u8], bitpix: Bitpix) -> Vec<f64> {
    match bitpix {
        Bitpix::F32 => decode_be(bytes, f32::from_be_bytes)
            .iter()
            .map(|&x| x as f64)
            .collect(),
        Bitpix::F64 => decode_be(bytes, f64::from_be_bytes),
        _ => Vec::new(),
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

/// Scatter a decoded integer tile into the typed output at `indices`, narrowing
/// each `i64` to the output element type. `out` is integer-typed (the integer
/// decode path); a float buffer is a logic error.
fn scatter_i64(out: &mut ImageData, indices: &[usize], vals: &[i64]) {
    match out {
        ImageData::U8(o) => scatter(o, indices, vals, |v| v as u8),
        ImageData::I16(o) => scatter(o, indices, vals, |v| v as i16),
        ImageData::I32(o) => scatter(o, indices, vals, |v| v as i32),
        ImageData::I64(o) => scatter(o, indices, vals, |v| v),
        ImageData::F32(_) | ImageData::F64(_) => unreachable!("integer decode, float output"),
    }
}

/// Scatter a dequantized float tile into the typed output at `indices`, narrowing
/// to the output element type (`f32`/`f64`; `NaN` nulls survive the cast).
fn scatter_f64(out: &mut ImageData, indices: &[usize], vals: &[f64]) {
    match out {
        ImageData::F32(o) => scatter(o, indices, vals, |v| v as f32),
        ImageData::F64(o) => scatter(o, indices, vals, |v| v),
        _ => unreachable!("float decode, integer output"),
    }
}

/// Write `convert(vals[k])` into `out[indices[k]]` for each tile element. Tiles
/// partition the image, so the index sets never overlap.
fn scatter<S, D>(out: &mut [D], indices: &[usize], vals: &[S], convert: impl Fn(S) -> D)
where
    S: Copy,
{
    for (&flat, &v) in indices.iter().zip(vals) {
        out[flat] = convert(v);
    }
}

#[cfg(test)]
mod tests;
