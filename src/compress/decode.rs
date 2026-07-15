//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`](crate::compress::gzip)/[`rice`](crate::compress::rice)/
//! [`plio`](crate::compress::plio)/[`hcompress`](crate::compress::hcompress) modules;
//! this drives the tile geometry, the
//! fallback-column resolution, and the narrow-and-scatter into the output plane.

#[cfg(feature = "parallel")]
use crate::compress::DisjointSlice;
use crate::compress::convert::be_floats_into;
use crate::compress::convert::be_to_i64_into;
use crate::compress::convert::byte_cell;
use crate::compress::convert::bytepix_to_bitpix;
use crate::compress::convert::cell_to_f64_into;
use crate::compress::convert::cell_to_i64_into;
use crate::compress::convert::plio_cell;
use crate::compress::convert::zeroed_samples;
use crate::compress::geometry::TileGeometry;
use crate::compress::geometry::TileScratch;
#[cfg(feature = "parallel")]
use crate::compress::map_tiles;
use crate::compress::{DitherMethod, ImageCodec};
use crate::compress::{gzip, hcompress, plio, quantize, rice};

use crate::allocation;
use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::ImageView;
use crate::data::Scaling;
use crate::data::shape_product;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table::BinTable;
use crate::table::ColumnData;
use crate::table::VlaCell;
use crate::table::VlaColumn;

#[derive(Debug)]
struct ImageLayout {
    bitpix: Bitpix,
    dims: Vec<usize>,
    total: usize,
    codec: ImageCodec,
    scaling: Scaling,
}

impl ImageLayout {
    fn from_header(header: &Header) -> Result<ImageLayout> {
        if header.get_logical("ZIMAGE") != Some(true) {
            return Err(FitsError::NotCompressedImage);
        }
        let bitpix = Bitpix::from_code(
            header
                .get_integer("ZBITPIX")
                .ok_or(FitsError::MissingKeyword { name: "ZBITPIX" })?,
        )?;
        let codec = ImageCodec::parse(
            header
                .get_text("ZCMPTYPE")
                .ok_or(FitsError::MissingKeyword { name: "ZCMPTYPE" })?,
        )?;
        let znaxis = header
            .get_integer("ZNAXIS")
            .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })?;
        if !(0..=999).contains(&znaxis) {
            return Err(FitsError::KeywordOutOfRange { name: "ZNAXIS" });
        }
        let dims = read_axes(header, znaxis as usize)?;
        let total = shape_product(&dims)?;
        Ok(ImageLayout {
            bitpix,
            dims,
            total,
            codec,
            scaling: header.scaling()?,
        })
    }
}

#[derive(Debug)]
enum DecodeBuffer<'a> {
    U8(&'a mut [u8]),
    I16(&'a mut [i16]),
    I32(&'a mut [i32]),
    I64(&'a mut [i64]),
    F32(&'a mut [f32]),
    F64(&'a mut [f64]),
}

impl<'a> DecodeBuffer<'a> {
    fn from_samples(samples: &'a mut ImageData) -> DecodeBuffer<'a> {
        match samples {
            ImageData::U8(values) => DecodeBuffer::U8(values),
            ImageData::I16(values) => DecodeBuffer::I16(values),
            ImageData::I32(values) => DecodeBuffer::I32(values),
            ImageData::I64(values) => DecodeBuffer::I64(values),
            ImageData::F32(values) => DecodeBuffer::F32(values),
            ImageData::F64(values) => DecodeBuffer::F64(values),
        }
    }

    fn from_words(words: &'a mut [u64], bitpix: Bitpix, count: usize) -> DecodeBuffer<'a> {
        assert!(
            count <= words.len().saturating_mul(8) / bitpix.elem_size(),
            "decode scratch must hold every sample"
        );
        let ptr = words.as_mut_ptr() as *mut u8;
        // SAFETY: the assertion proves the initialized byte capacity; u64 alignment
        // satisfies every FITS scalar type, whose bit patterns are all valid.
        unsafe {
            match bitpix {
                Bitpix::U8 => DecodeBuffer::U8(std::slice::from_raw_parts_mut(ptr, count)),
                Bitpix::I16 => {
                    DecodeBuffer::I16(std::slice::from_raw_parts_mut(ptr as *mut i16, count))
                }
                Bitpix::I32 => {
                    DecodeBuffer::I32(std::slice::from_raw_parts_mut(ptr as *mut i32, count))
                }
                Bitpix::I64 => {
                    DecodeBuffer::I64(std::slice::from_raw_parts_mut(ptr as *mut i64, count))
                }
                Bitpix::F32 => {
                    DecodeBuffer::F32(std::slice::from_raw_parts_mut(ptr as *mut f32, count))
                }
                Bitpix::F64 => {
                    DecodeBuffer::F64(std::slice::from_raw_parts_mut(ptr as *mut f64, count))
                }
            }
        }
    }
}

/// Decompress a tiled-image `BINTABLE` into the full [`Image`] it encodes.
pub(crate) fn decompress_image(header: &Header, table: &BinTable) -> Result<Image> {
    let layout = ImageLayout::from_header(header)?;
    let mut samples = zeroed_samples(layout.bitpix, layout.total)?;
    if layout.total != 0 {
        decode_image_into(
            header,
            table,
            &layout,
            DecodeBuffer::from_samples(&mut samples),
        )?;
    }
    Ok(Image {
        shape: layout.dims,
        samples,
        scaling: layout.scaling,
    })
}

pub(crate) fn decompress_image_into_words<'a>(
    header: &Header,
    table: &BinTable,
    words: &'a mut Vec<u64>,
) -> Result<ImageView<'a>> {
    let layout = ImageLayout::from_header(header)?;
    let nbytes = layout
        .total
        .checked_mul(layout.bitpix.elem_size())
        .ok_or(FitsError::DataUnitOverflow)?;
    allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
    if layout.total != 0 {
        let output = DecodeBuffer::from_words(words, layout.bitpix, layout.total);
        decode_image_into(header, table, &layout, output)?;
    }
    Ok(view_words(words, layout.bitpix, nbytes))
}

fn decode_image_into(
    header: &Header,
    table: &BinTable,
    layout: &ImageLayout,
    output: DecodeBuffer<'_>,
) -> Result<()> {
    let is_float = layout.bitpix.is_float();
    let tiles: Vec<usize> = (1..=layout.dims.len())
        .map(|i| {
            header
                .get_integer(key!("ZTILE{i}").as_str())
                .map(|v| v.max(1) as usize)
                .unwrap_or(if i == 1 { layout.dims[0] } else { 1 })
        })
        .collect();

    let rice = rice::rice_params(header, layout.bitpix);
    // Float pixels are quantized to integers of `bytepix` bytes; decode the tile
    // as that integer type, then dequantize. Integer images decode as `zbitpix`.
    let int_bitpix = if is_float {
        bytepix_to_bitpix(rice.bytepix)
    } else {
        layout.bitpix
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
    let zblank_column = read_i64_column(table, "ZBLANK")?;
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
    let zscale = read_f64_column(table, "ZSCALE")?;
    let zzero = read_f64_column(table, "ZZERO")?;

    let geom = TileGeometry::new(&layout.dims, &tiles);
    let ntiles = geom.ntiles();

    // Decode and scatter each tile in one fused pass — parallel under the `parallel`
    // feature, where tiles write disjoint regions of `samples` concurrently (they
    // partition the image). Each value is narrowed to `ZBITPIX` as it lands, so there
    // is no whole-image `i64`/`f64` intermediate and no separate serial scatter tail.
    let ctx = DecodeCtx {
        codec: layout.codec,
        zbitpix: layout.bitpix,
        int_bitpix,
        params,
    };
    if is_float {
        let decode = |t: usize,
                      s: &TileScratch,
                      out: &mut Vec<f64>,
                      ints: &mut Vec<i64>,
                      codecs: &mut CodecScratch| {
            let cols = TileColumns::read(t, primary, gzip_fallback, uncompressed)?;
            let dq = Dequant {
                scale: column_at(&zscale, t).unwrap_or(1.0),
                zero: column_at(&zzero, t).unwrap_or(0.0),
                method,
                irow: t as i64 + zdither0,
                zblank: column_at(&zblank_column, t).or(zblank_keyword),
            };
            decode_float_tile_into(&ctx, cols, s.nelem(), dq, out, ints, codecs)
        };
        match output {
            DecodeBuffer::F32(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value as f32)?
            }
            DecodeBuffer::F64(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value)?
            }
            _ => unreachable!("a float ZBITPIX yields a float sample buffer"),
        }
    } else {
        let decode = |t: usize,
                      s: &TileScratch,
                      out: &mut Vec<i64>,
                      _ints: &mut Vec<i64>,
                      codecs: &mut CodecScratch| {
            let cols = TileColumns::read(t, primary, gzip_fallback, uncompressed)?;
            decode_one_tile_into(&ctx, cols, s.nelem(), out, codecs)
        };
        match output {
            DecodeBuffer::U8(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value as u8)?
            }
            DecodeBuffer::I16(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value as i16)?
            }
            DecodeBuffer::I32(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value as i32)?
            }
            DecodeBuffer::I64(out) => {
                run_decode_scatter(ntiles, &geom, out, decode, |value| value)?
            }
            _ => unreachable!("an integer ZBITPIX yields an integer sample buffer"),
        }
    }
    Ok(())
}

/// Decode every tile and scatter its values into `out` at the tile's positions,
/// narrowing each with `convert`. Under `parallel` the tiles run concurrently and
/// write disjoint regions of `out` directly (no collect, no serial scatter);
/// otherwise it is a plain fused loop.
fn run_decode_scatter<S, D>(
    ntiles: usize,
    geom: &TileGeometry,
    out: &mut [D],
    decode: impl Fn(usize, &TileScratch, &mut Vec<S>, &mut Vec<i64>, &mut CodecScratch) -> Result<()>
    + Sync
    + Send,
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
        let sink = DisjointSlice::new(out);
        let init = || {
            (
                TileScratch::default(),
                Vec::<S>::new(),
                Vec::<i64>::new(),
                CodecScratch::default(),
            )
        };
        map_tiles(
            ntiles,
            init,
            |(scratch, vals, ints, codecs), t| -> Result<()> {
                geom.tile_into(t, scratch);
                decode(t, scratch, vals, ints, codecs)?;
                ensure_tile_size(scratch.nelem(), vals.len())?;
                // SAFETY: the image tiles partition the pixel grid, so this tile's row
                // ranges are disjoint from every other tile's — concurrent writes through
                // `sink` never alias. `tile_into` clips rows to the image, which sized
                // `out`, so each row is in bounds.
                unsafe {
                    scatter_disjoint(&sink, &scratch.row_bases, scratch.row_len, vals, &convert)
                };
                Ok(())
            },
        )?;
        Ok(())
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut scratch = TileScratch::default();
        let mut vals: Vec<S> = Vec::new();
        let mut ints: Vec<i64> = Vec::new();
        let mut codecs = CodecScratch::default();
        for t in 0..ntiles {
            geom.tile_into(t, &mut scratch);
            decode(t, &scratch, &mut vals, &mut ints, &mut codecs)?;
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

#[cfg(feature = "parallel")]
unsafe fn scatter_disjoint<S: Copy, D>(
    sink: &DisjointSlice<D>,
    row_bases: &[usize],
    row_len: usize,
    vals: &[S],
    convert: &impl Fn(S) -> D,
) {
    let mut off = 0;
    for &base in row_bases {
        // SAFETY: image tiles partition the pixel grid, so these in-bounds row
        // ranges do not overlap any concurrently accessed range.
        unsafe { sink.map_from_slice(base, &vals[off..off + row_len], convert) };
        off += row_len;
    }
}

fn read_tiles<'a>(table: &'a BinTable, name: &str) -> Result<Option<VlaColumn<'a>>> {
    match table.column_index(name) {
        Some(c) => Ok(Some(table.column_by_idx(c)?.vla_column()?)),
        None => Ok(None),
    }
}

/// Read a per-tile `f64` column (e.g. `ZSCALE`/`ZZERO`), or `None` if absent.
fn read_f64_column(table: &BinTable, name: &str) -> Result<Option<Vec<f64>>> {
    let Some(c) = table.column_index(name) else {
        return Ok(None);
    };
    match table.column_by_idx(c)?.raw()? {
        ColumnData::F64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "f64 column",
        }),
    }
}

/// Read a per-tile integer column (e.g. a `ZBLANK` column), widening any integer
/// `TFORM` to `i64`, or `None` if absent.
fn read_i64_column(table: &BinTable, name: &str) -> Result<Option<Vec<i64>>> {
    let Some(c) = table.column_index(name) else {
        return Ok(None);
    };
    match table.column_by_idx(c)?.raw()? {
        ColumnData::Bytes(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I16(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I32(v) => Ok(Some(v.iter().map(|&x| x as i64).collect())),
        ColumnData::I64(v) => Ok(Some(v)),
        _ => Err(FitsError::TypeMismatch {
            name: name.to_string(),
            expected: "integer column",
        }),
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
    primary: Option<VlaCell<'a>>,
    gzip: Option<VlaCell<'a>>,
    uncompressed: Option<VlaCell<'a>>,
}

/// The resolved source for one tile — which non-empty column holds its bytes.
#[derive(Debug)]
enum TileSource<'a> {
    Compressed(VlaCell<'a>),
    Gzip(VlaCell<'a>),
    Uncompressed(VlaCell<'a>),
}

impl<'a> TileColumns<'a> {
    fn read(
        row: usize,
        primary: Option<VlaColumn<'a>>,
        gzip: Option<VlaColumn<'a>>,
        uncompressed: Option<VlaColumn<'a>>,
    ) -> Result<TileColumns<'a>> {
        Ok(TileColumns {
            primary: primary.map(|column| column.cell(row)).transpose()?,
            gzip: gzip.map(|column| column.cell(row)).transpose()?,
            uncompressed: uncompressed.map(|column| column.cell(row)).transpose()?,
        })
    }

    /// Pick the first non-empty source: primary `COMPRESSED_DATA`, then the
    /// gzip and uncompressed fallbacks; error if every column's cell is empty.
    fn resolve(self) -> Result<TileSource<'a>> {
        if let Some(c) = self.primary.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Compressed(c))
        } else if let Some(c) = self.gzip.filter(|cell| cell.element_count > 0) {
            Ok(TileSource::Gzip(c))
        } else if let Some(c) = self.uncompressed.filter(|cell| cell.element_count > 0) {
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

#[derive(Debug, Default)]
struct CodecScratch {
    gzip: gzip::GzipScratch,
    hcompress: hcompress::HcompressScratch,
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
    scratch: &mut CodecScratch,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => decode_tile_cell_into(ctx, cell, tile_elems, out, scratch),
        TileSource::Gzip(cell) => gzip::gzip_tile_into(
            byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
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
    scratch: &mut CodecScratch,
) -> Result<()> {
    match cols.resolve()? {
        TileSource::Compressed(cell) => {
            // Quantized integers (float images never use HCOMPRESS).
            decode_tile_cell_into(ctx, cell, tile_elems, ints, scratch)?;
            quantize::dequantize_into(ints, dq.scale, dq.zero, dq.method, dq.irow, dq.zblank, out);
            Ok(())
        }
        TileSource::Gzip(cell) => {
            // Raw floats, bounded at the tile's known byte size (`tile_elems` floats).
            let max = tile_elems.saturating_mul(ctx.zbitpix.elem_size());
            gzip::gunzip_into(byte_cell(cell)?, max, &mut scratch.gzip.bytes)?;
            be_floats_into(&scratch.gzip.bytes, ctx.zbitpix, out);
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
    cell: VlaCell<'_>,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut CodecScratch,
) -> Result<()> {
    let params = ctx.params;
    match ctx.codec {
        ImageCodec::Gzip1 => gzip::gzip_tile_into(
            byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
        ImageCodec::Gzip2 => gzip::gzip2_tile_into(
            byte_cell(cell)?,
            ctx.int_bitpix,
            tile_elems,
            out,
            &mut scratch.gzip,
        ),
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
                byte_cell(cell)?,
                tile_elems,
                params.bytepix,
                params.blocksize,
                out,
            )
        }
        ImageCodec::Plio1 => plio::plio_decode_be_into(plio_cell(cell)?, tile_elems, out),
        ImageCodec::Hcompress1 => hcompress::hcompress_tile_into(
            byte_cell(cell)?,
            params.smooth,
            tile_elems,
            out,
            &mut scratch.hcompress,
        ),
        // §10.4: a tile stored verbatim — the cell is the raw big-endian pixels.
        ImageCodec::NoCompress => {
            let bytes = byte_cell(cell)?;
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
