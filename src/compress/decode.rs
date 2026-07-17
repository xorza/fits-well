//! Tiled-image decompression (§10.1).
//!
//! Reassemble the per-tile codec output (`COMPRESSED_DATA`, with the
//! `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks) into the full [`Image`],
//! de-quantizing float tiles (`ZSCALE`/`ZZERO`) on the way. The per-codec work lives
//! in the sibling [`gzip`](crate::compress::gzip)/[`rice`](crate::compress::rice)/
//! [`plio`](crate::compress::plio)/[`hcompress`](crate::compress::hcompress) modules;
//! this drives the tile geometry, the
//! fallback-column resolution, and the narrow-and-scatter into the output plane.

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
use crate::data::BorrowedImage;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::data::shape_product;
use crate::data::view_words;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;
use crate::table_impl::BinTable;
use crate::table_impl::ColumnData;
use crate::table_impl::VlaCell;
use crate::table_impl::VlaColumn;
use std::ops::Range;

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
        if header.get_logical("ZIMAGE")? != Some(true) {
            return Err(FitsError::NotCompressedImage);
        }
        let bitpix = Bitpix::from_code(
            header
                .get_integer("ZBITPIX")?
                .ok_or(FitsError::MissingKeyword { name: "ZBITPIX" })?,
        )?;
        let codec = ImageCodec::parse(
            header
                .get_text("ZCMPTYPE")?
                .ok_or(FitsError::MissingKeyword { name: "ZCMPTYPE" })?,
        )?;
        let znaxis = header
            .get_integer("ZNAXIS")?
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
        debug_assert!(
            count <= words.len().saturating_mul(8) / bitpix.elem_size(),
            "decode scratch must hold every sample"
        );
        let ptr = words.as_mut_ptr() as *mut u8;
        // SAFETY: the caller sizes `words` for every sample; u64 alignment satisfies
        // every FITS scalar type, whose bit patterns are all valid.
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
    Image::new_scaled(layout.dims, samples, layout.scaling)
}

pub(crate) fn decompress_image_into_words<'a>(
    header: &Header,
    table: &BinTable,
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
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
    Ok(BorrowedImage {
        shape: layout.dims,
        scaling: layout.scaling,
        samples: view_words(words, layout.bitpix, nbytes),
    })
}

/// Original compressed-table row indices for tiles intersecting `ranges`.
pub(crate) fn compressed_image_tile_rows(
    header: &Header,
    ranges: &[Range<usize>],
) -> Result<Vec<usize>> {
    let layout = ImageLayout::from_header(header)?;
    validate_region(ranges, &layout.dims)?;
    if ranges.iter().any(Range::is_empty) || layout.dims.is_empty() {
        return Ok(Vec::new());
    }
    let tiles = read_tile_shape(header, &layout.dims)?;
    let counts: Vec<usize> = layout
        .dims
        .iter()
        .zip(&tiles)
        .map(|(&dim, &tile)| dim.div_ceil(tile))
        .collect();
    let starts: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| range.start / tile)
        .collect();
    let ends: Vec<usize> = ranges
        .iter()
        .zip(&tiles)
        .map(|(range, &tile)| (range.end - 1) / tile + 1)
        .collect();
    let tile_count = starts
        .iter()
        .zip(&ends)
        .try_fold(1usize, |count, (&start, &end)| {
            count.checked_mul(end - start)
        })
        .ok_or(FitsError::DataUnitOverflow)?;
    let mut coordinates = starts.clone();
    let mut selected = Vec::with_capacity(tile_count);
    for _ in 0..tile_count {
        let mut stride = 1usize;
        let mut index = 0usize;
        for axis in 0..coordinates.len() {
            index = index
                .checked_add(
                    coordinates[axis]
                        .checked_mul(stride)
                        .ok_or(FitsError::DataUnitOverflow)?,
                )
                .ok_or(FitsError::DataUnitOverflow)?;
            stride = stride
                .checked_mul(counts[axis])
                .ok_or(FitsError::DataUnitOverflow)?;
        }
        selected.push(index);
        for axis in 0..coordinates.len() {
            coordinates[axis] += 1;
            if coordinates[axis] < ends[axis] {
                break;
            }
            coordinates[axis] = starts[axis];
        }
    }
    Ok(selected)
}

/// Decompress only the compact table rows in `tile_rows`, scattering their
/// intersections into one scratch-backed image section.
pub(crate) fn decompress_image_section_into_words<'a>(
    header: &Header,
    table: &BinTable,
    tile_rows: &[usize],
    ranges: &[Range<usize>],
    words: &'a mut Vec<u64>,
) -> Result<BorrowedImage<'a>> {
    let layout = ImageLayout::from_header(header)?;
    let selected_shape = validate_region(ranges, &layout.dims)?;
    if table.metadata().nrows != tile_rows.len() {
        return Err(FitsError::DataSizeMismatch {
            expected: tile_rows.len(),
            got: table.metadata().nrows,
        });
    }
    let total = shape_product(&selected_shape)?;
    let nbytes = total
        .checked_mul(layout.bitpix.elem_size())
        .ok_or(FitsError::DataUnitOverflow)?;
    allocation::try_resize(words, nbytes.div_ceil(8), 0)?;
    if total == 0 {
        return Ok(BorrowedImage {
            shape: selected_shape,
            scaling: layout.scaling,
            samples: view_words(words, layout.bitpix, nbytes),
        });
    }

    let tiles = read_tile_shape(header, &layout.dims)?;
    let rice = rice::rice_params(header, layout.bitpix)?;
    let is_float = layout.bitpix.is_float();
    let int_bitpix = if is_float {
        bytepix_to_bitpix(rice.bytepix)
    } else {
        layout.bitpix
    };
    let method = match header.get_text("ZQUANTIZ")?.unwrap_or("NO_DITHER") {
        "NO_DITHER" => DitherMethod::None,
        "SUBTRACTIVE_DITHER_1" => DitherMethod::Subtractive1,
        "SUBTRACTIVE_DITHER_2" => DitherMethod::Subtractive2,
        other if is_float => {
            return Err(FitsError::UnsupportedCompression {
                name: format!("float quantization {other}"),
            });
        }
        _ => DitherMethod::None,
    };
    let zdither0 = header.get_integer("ZDITHER0")?.unwrap_or(1);
    let zblank_keyword = header.get_integer("ZBLANK")?;
    let zblank_column = read_i64_column(table, "ZBLANK")?;
    let zscale = read_f64_column(table, "ZSCALE")?;
    let zzero = read_f64_column(table, "ZZERO")?;
    let primary = read_tiles(table, "COMPRESSED_DATA")?;
    let gzip_fallback = read_tiles(table, "GZIP_COMPRESSED_DATA")?;
    let uncompressed = read_tiles(table, "UNCOMPRESSED_DATA")?;
    let ctx = DecodeCtx {
        codec: layout.codec,
        zbitpix: layout.bitpix,
        int_bitpix,
        params: CodecParams {
            blocksize: rice.blocksize,
            bytepix: rice.bytepix,
            smooth: hcompress_smooth(header)?,
        },
    };
    let geom = TileGeometry::new(&layout.dims, &tiles);
    let output = DecodeBuffer::from_words(words, layout.bitpix, total);
    if is_float {
        let decode = |table_row: usize,
                      tile_row: usize,
                      scratch: &TileScratch,
                      out: &mut Vec<f64>,
                      ints: &mut Vec<i64>,
                      codecs: &mut CodecScratch| {
            let cols = TileColumns::read(table_row, primary, gzip_fallback, uncompressed)?;
            let dq = Dequant {
                scale: column_at(&zscale, table_row).unwrap_or(1.0),
                zero: column_at(&zzero, table_row).unwrap_or(0.0),
                method,
                irow: tile_row as i64 + zdither0,
                zblank: column_at(&zblank_column, table_row).or(zblank_keyword),
            };
            decode_float_tile_into(&ctx, cols, scratch.nelem(), dq, out, ints, codecs)
        };
        match output {
            DecodeBuffer::F32(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value as f32,
            )?,
            DecodeBuffer::F64(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value,
            )?,
            _ => unreachable!("a float ZBITPIX yields a float sample buffer"),
        }
    } else {
        let decode = |table_row: usize,
                      _tile_row: usize,
                      scratch: &TileScratch,
                      out: &mut Vec<i64>,
                      _ints: &mut Vec<i64>,
                      codecs: &mut CodecScratch| {
            let cols = TileColumns::read(table_row, primary, gzip_fallback, uncompressed)?;
            decode_one_tile_into(&ctx, cols, scratch.nelem(), out, codecs)
        };
        match output {
            DecodeBuffer::U8(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value as u8,
            )?,
            DecodeBuffer::I16(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value as i16,
            )?,
            DecodeBuffer::I32(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value as i32,
            )?,
            DecodeBuffer::I64(out) => run_decode_region(
                &geom,
                ranges,
                &selected_shape,
                tile_rows,
                out,
                decode,
                |value| value,
            )?,
            _ => unreachable!("an integer ZBITPIX yields an integer sample buffer"),
        }
    }
    Ok(BorrowedImage {
        shape: selected_shape,
        scaling: layout.scaling,
        samples: view_words(words, layout.bitpix, nbytes),
    })
}

fn decode_image_into(
    header: &Header,
    table: &BinTable,
    layout: &ImageLayout,
    output: DecodeBuffer<'_>,
) -> Result<()> {
    let is_float = layout.bitpix.is_float();
    let tiles = read_tile_shape(header, &layout.dims)?;

    let rice = rice::rice_params(header, layout.bitpix)?;
    // Float pixels are quantized to integers of `bytepix` bytes; decode the tile
    // as that integer type, then dequantize. Integer images decode as `zbitpix`.
    let int_bitpix = if is_float {
        bytepix_to_bitpix(rice.bytepix)
    } else {
        layout.bitpix
    };

    // Float quantization: NO_DITHER, SUBTRACTIVE_DITHER_1, and SUBTRACTIVE_DITHER_2.
    let zquantiz = header
        .get_text("ZQUANTIZ")?
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
    let zdither0 = header.get_integer("ZDITHER0")?.unwrap_or(1);
    // ZBLANK may be a keyword (constant) or a per-tile column; §10.1.3 says the
    // column value wins where present.
    let zblank_keyword = header.get_integer("ZBLANK")?;
    let zblank_column = read_i64_column(table, "ZBLANK")?;
    let smooth = hcompress_smooth(header)?;
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
            DecodeBuffer::F32(out) => run_decode_scatter(&geom, out, decode, |value| value as f32)?,
            DecodeBuffer::F64(out) => run_decode_scatter(&geom, out, decode, |value| value)?,
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
            DecodeBuffer::U8(out) => run_decode_scatter(&geom, out, decode, |value| value as u8)?,
            DecodeBuffer::I16(out) => run_decode_scatter(&geom, out, decode, |value| value as i16)?,
            DecodeBuffer::I32(out) => run_decode_scatter(&geom, out, decode, |value| value as i32)?,
            DecodeBuffer::I64(out) => run_decode_scatter(&geom, out, decode, |value| value)?,
            _ => unreachable!("an integer ZBITPIX yields an integer sample buffer"),
        }
    }
    Ok(())
}

fn run_decode_scatter<S, D>(
    geom: &TileGeometry,
    out: &mut [D],
    decode: impl Fn(usize, &TileScratch, &mut Vec<S>, &mut Vec<i64>, &mut CodecScratch) -> Result<()>
    + Sync
    + Send,
    convert: impl Fn(S) -> D + Sync + Send,
) -> Result<()>
where
    S: Copy + Send,
    D: Copy + Send + Sync,
{
    #[cfg(feature = "parallel")]
    {
        let init = || {
            (
                TileScratch::default(),
                Vec::<S>::new(),
                Vec::<i64>::new(),
                CodecScratch::default(),
            )
        };
        let decoded = map_tiles(
            geom.ntiles(),
            init,
            |(scratch, vals, ints, codecs), t| -> Result<Vec<D>> {
                geom.tile_into(t, scratch);
                decode(t, scratch, vals, ints, codecs)?;
                ensure_tile_size(scratch.nelem(), vals.len())?;
                Ok(vals.iter().copied().map(&convert).collect())
            },
        )?;
        geom.scatter_tiles(&decoded, out);
        Ok(())
    }
    #[cfg(not(feature = "parallel"))]
    {
        let mut scratch = TileScratch::default();
        let mut vals: Vec<S> = Vec::new();
        let mut ints: Vec<i64> = Vec::new();
        let mut codecs = CodecScratch::default();
        for t in 0..geom.ntiles() {
            geom.tile_into(t, &mut scratch);
            decode(t, &scratch, &mut vals, &mut ints, &mut codecs)?;
            ensure_tile_size(scratch.nelem(), vals.len())?;
            scatter_rows(out, &scratch.row_bases, scratch.row_len, &vals, &convert);
        }
        Ok(())
    }
}

fn run_decode_region<S, D>(
    geom: &TileGeometry,
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    tile_rows: &[usize],
    out: &mut [D],
    decode: impl Fn(
        usize,
        usize,
        &TileScratch,
        &mut Vec<S>,
        &mut Vec<i64>,
        &mut CodecScratch,
    ) -> Result<()>,
    convert: impl Fn(S) -> D,
) -> Result<()>
where
    S: Copy,
    D: Copy,
{
    let mut scratch = TileScratch::default();
    let mut values = Vec::new();
    let mut ints = Vec::new();
    let mut codecs = CodecScratch::default();
    for (table_row, &tile_row) in tile_rows.iter().enumerate() {
        geom.tile_into(tile_row, &mut scratch);
        decode(
            table_row,
            tile_row,
            &scratch,
            &mut values,
            &mut ints,
            &mut codecs,
        )?;
        ensure_tile_size(scratch.nelem(), values.len())?;
        scatter_region_tile(&scratch, ranges, selected_shape, &values, out, &convert);
    }
    Ok(())
}

fn scatter_region_tile<S: Copy, D: Copy>(
    tile: &TileScratch,
    ranges: &[Range<usize>],
    selected_shape: &[usize],
    values: &[S],
    out: &mut [D],
    convert: &impl Fn(S) -> D,
) {
    let x_start = tile.origin[0].max(ranges[0].start);
    let x_end = (tile.origin[0] + tile.tdims[0]).min(ranges[0].end);
    if x_start >= x_end {
        return;
    }
    let width = x_end - x_start;
    for row in 0..tile.row_bases.len() {
        let mut remainder = row;
        let mut output_base = 0usize;
        let mut output_stride = selected_shape[0];
        let mut selected = true;
        for axis in 1..tile.tdims.len() {
            let local = remainder % tile.tdims[axis];
            remainder /= tile.tdims[axis];
            let coordinate = tile.origin[axis] + local;
            if !ranges[axis].contains(&coordinate) {
                selected = false;
                break;
            }
            output_base += (coordinate - ranges[axis].start) * output_stride;
            output_stride *= selected_shape[axis];
        }
        if !selected {
            continue;
        }
        let source = row * tile.row_len + (x_start - tile.origin[0]);
        let destination = output_base + (x_start - ranges[0].start);
        for (slot, &value) in out[destination..destination + width]
            .iter_mut()
            .zip(&values[source..source + width])
        {
            *slot = convert(value);
        }
    }
}

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

fn read_tile_shape(header: &Header, dims: &[usize]) -> Result<Vec<usize>> {
    (1..=dims.len())
        .map(|i| -> Result<usize> {
            let default = if i == 1 { dims[0].max(1) } else { 1 };
            match header.get_integer(key!("ZTILE{i}").as_str())? {
                Some(value) => usize::try_from(value)
                    .ok()
                    .filter(|&value| value > 0)
                    .ok_or(FitsError::KeywordOutOfRange { name: "ZTILEn" }),
                None => Ok(default),
            }
        })
        .collect()
}

fn validate_region(ranges: &[Range<usize>], dims: &[usize]) -> Result<Vec<usize>> {
    if ranges.len() != dims.len() {
        return Err(FitsError::ImageRegionRankMismatch {
            region_rank: ranges.len(),
            image_rank: dims.len(),
        });
    }
    let mut shape = Vec::with_capacity(dims.len());
    for (axis, (range, &len)) in ranges.iter().zip(dims).enumerate() {
        if range.start > range.end || range.end > len {
            return Err(FitsError::ImageRegionOutOfBounds {
                axis,
                start: range.start,
                end: range.end,
                len,
            });
        }
        shape.push(range.end - range.start);
    }
    Ok(shape)
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
fn hcompress_smooth(header: &Header) -> Result<bool> {
    let mut i = 1;
    while let Some(name) = header.get_text(key!("ZNAME{i}").as_str())? {
        if name == "SMOOTH" {
            return Ok(header.get_integer(key!("ZVAL{i}").as_str())?.unwrap_or(0) != 0);
        }
        i += 1;
    }
    Ok(false)
}

/// Read the `ZNAXIS1..ZNAXISn` integer axis lengths.
fn read_axes(header: &Header, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| match header.get_integer(key!("ZNAXIS{i}").as_str())? {
            Some(value) => {
                usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { name: "ZNAXISn" })
            }
            None => Err(FitsError::MissingKeyword { name: "ZNAXISn" }),
        })
        .collect()
}
