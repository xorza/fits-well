//! Tiled image compression (§10.1) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module orchestrates tile reassembly and (de)quantization; the per-codec
//! work lives in [`gzip`], [`rice`], [`plio`], and [`hcompress`].
//!
//! Decode: `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` (SMOOTH=0); float
//! images via per-tile `ZSCALE`/`ZZERO` linear dequantization (`NO_DITHER`) or the
//! raw-float gzip fallback. Write ([`encode_image`]): `GZIP_1`, `GZIP_2`, `RICE_1`
//! for integer images. Not yet: HCOMPRESS smoothing, subtractive dithering,
//! `ZBLANK`, float-quantization write, and `PLIO_1`/`HCOMPRESS_1` encoders.

mod gzip;
mod hcompress;
mod plio;
mod rice;

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
use crate::endian::decode_be;
use crate::endian::encode_be;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
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
        .ok_or(FitsError::MissingKeyword { name: "ZNAXIS" })? as usize;
    let dims = read_axes(header, "ZNAXIS", znaxis)?;
    let tiles: Vec<usize> = (1..=znaxis)
        .map(|i| {
            header
                .get_integer(&format!("ZTILE{i}"))
                .map(|v| v.max(1) as usize)
                .unwrap_or(if i == 1 { dims[0] } else { 1 })
        })
        .collect();

    let (blocksize, bytepix) = rice::rice_params(header, zbitpix);
    // Float pixels are quantized to integers of `bytepix` bytes; decode the tile
    // as that integer type, then dequantize. Integer images decode as `zbitpix`.
    let int_bitpix = if is_float {
        bytepix_to_bitpix(bytepix)
    } else {
        zbitpix
    };

    // Float quantization: NO_DITHER (linear) is supported; dithering is not yet.
    let zquantiz = header
        .get_text("ZQUANTIZ")
        .unwrap_or("NO_DITHER")
        .to_string();
    if is_float && zquantiz != "NO_DITHER" {
        return Err(FitsError::UnsupportedCompression {
            name: format!("float quantization {zquantiz}"),
        });
    }

    // Per-tile compressed data, with the conventional fallback columns.
    let primary = read_tiles(table, "COMPRESSED_DATA")?;
    let gzip_fallback = read_tiles(table, "GZIP_COMPRESSED_DATA")?;
    let uncompressed = read_tiles(table, "UNCOMPRESSED_DATA")?;
    // Per-tile linear dequantization parameters (float only).
    let zscale = read_f64_column(table, "ZSCALE");
    let zzero = read_f64_column(table, "ZZERO");

    let ntiles_axis: Vec<usize> = dims
        .iter()
        .zip(&tiles)
        .map(|(&n, &t)| n.div_ceil(t))
        .collect();
    let ntiles: usize = ntiles_axis.iter().product();

    let mut stride = vec![1usize; znaxis];
    for i in 1..znaxis {
        stride[i] = stride[i - 1] * dims[i - 1];
    }

    let total: usize = dims.iter().product();
    let mut out_i = vec![0i64; if is_float { 0 } else { total }];
    let mut out_f = vec![0f64; if is_float { total } else { 0 }];

    for t in 0..ntiles {
        // Tile origin and (edge-clipped) dimensions.
        let mut origin = vec![0usize; znaxis];
        let mut tdims = vec![0usize; znaxis];
        let mut rem = t;
        for i in 0..znaxis {
            let ti = rem % ntiles_axis[i];
            rem /= ntiles_axis[i];
            origin[i] = ti * tiles[i];
            tdims[i] = tiles[i].min(dims[i] - origin[i]);
        }
        let tile_elems: usize = tdims.iter().product();

        let indices = tile_flat_indices(&origin, &tdims, &stride);
        if is_float {
            let s = column_at(&zscale, t).unwrap_or(1.0);
            let z = column_at(&zzero, t).unwrap_or(0.0);
            let vals = decode_float_tile(
                &cmptype,
                primary.get(t),
                gzip_fallback.get(t),
                uncompressed.get(t),
                tile_elems,
                zbitpix,
                int_bitpix,
                blocksize,
                bytepix,
                s,
                z,
            )?;
            for (&flat, &v) in indices.iter().zip(&vals) {
                out_f[flat] = v;
            }
        } else {
            let vals = decode_one_tile(
                &cmptype,
                primary.get(t),
                gzip_fallback.get(t),
                uncompressed.get(t),
                tile_elems,
                int_bitpix,
                blocksize,
                bytepix,
            )?;
            for (&flat, &v) in indices.iter().zip(&vals) {
                out_i[flat] = v;
            }
        }
    }

    let samples = if is_float {
        float_samples(out_f, zbitpix)
    } else {
        narrow(out_i, zbitpix)
    };
    Ok(Image {
        shape: dims,
        samples,
        scaling: Scaling::from_header(header),
    })
}

/// Encode an integer [`Image`] as a tiled-compressed `BINTABLE`: returns the
/// `ZIMAGE` header and the data unit (per-tile `P` descriptors + the heap of
/// compressed tile bytes). `tile_shape` of the wrong length falls back to
/// row-tiling. Float images and codecs without an encoder are rejected.
pub(crate) fn encode_image(
    image: &Image,
    cmptype: &str,
    tile_shape: &[usize],
) -> Result<(Header, Vec<u8>)> {
    let bitpix = image.samples.bitpix();
    if bitpix.is_float() {
        return Err(FitsError::UnsupportedCompression {
            name: "float image compression (write)".to_string(),
        });
    }
    let dims = &image.shape;
    let znaxis = dims.len();
    let tiles: Vec<usize> = if tile_shape.len() == znaxis {
        tile_shape.iter().map(|&t| t.max(1)).collect()
    } else {
        (0..znaxis)
            .map(|i| if i == 0 { dims[0].max(1) } else { 1 })
            .collect()
    };

    let flat = widen(&image.samples);
    let mut stride = vec![1usize; znaxis];
    for i in 1..znaxis {
        stride[i] = stride[i - 1] * dims[i - 1];
    }
    let ntiles_axis: Vec<usize> = dims
        .iter()
        .zip(&tiles)
        .map(|(&n, &t)| n.div_ceil(t))
        .collect();
    let ntiles: usize = ntiles_axis.iter().product();
    let bytepix = bitpix.elem_size();

    let mut descriptors: Vec<(usize, usize)> = Vec::with_capacity(ntiles);
    let mut heap: Vec<u8> = Vec::new();
    for t in 0..ntiles {
        let mut origin = vec![0usize; znaxis];
        let mut tdims = vec![0usize; znaxis];
        let mut rem = t;
        for i in 0..znaxis {
            let ti = rem % ntiles_axis[i];
            rem /= ntiles_axis[i];
            origin[i] = ti * tiles[i];
            tdims[i] = tiles[i].min(dims[i] - origin[i]);
        }
        let vals: Vec<i64> = tile_flat_indices(&origin, &tdims, &stride)
            .iter()
            .map(|&i| flat[i])
            .collect();
        let bytes = match cmptype {
            "GZIP_1" => gzip::gzip_encode(&i64_to_be(&vals, bitpix)),
            "GZIP_2" => gzip::gzip2_encode(&i64_to_be(&vals, bitpix), bytepix),
            "RICE_1" => rice::rice_encode(&vals, bytepix, 32),
            other => {
                return Err(FitsError::UnsupportedCompression {
                    name: format!("{other} (write)"),
                });
            }
        };
        descriptors.push((bytes.len(), heap.len()));
        heap.extend_from_slice(&bytes);
    }

    // Data unit: a `P` array descriptor (count, heap offset) per tile, then the heap.
    let mut data = Vec::with_capacity(ntiles * 8 + heap.len());
    for &(nelem, offset) in &descriptors {
        data.extend_from_slice(&(nelem as i32).to_be_bytes());
        data.extend_from_slice(&(offset as i32).to_be_bytes());
    }
    data.extend_from_slice(&heap);

    let maxnelem = descriptors.iter().map(|&(n, _)| n).max().unwrap_or(0);
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .comment("XTENSION", "binary table extension");
    h.set("BITPIX", 8).set("NAXIS", 2);
    h.set("NAXIS1", 8).set("NAXIS2", ntiles as i64);
    h.set("PCOUNT", heap.len() as i64).set("GCOUNT", 1);
    h.set("TFIELDS", 1);
    h.set("TTYPE1", "COMPRESSED_DATA");
    h.set("TFORM1", format!("1PB({maxnelem})"));
    h.set("ZIMAGE", true)
        .comment("ZIMAGE", "this is a tiled-compressed image");
    h.set("ZCMPTYPE", cmptype);
    h.set("ZBITPIX", bitpix.code());
    h.set("ZNAXIS", znaxis as i64);
    for (i, &n) in dims.iter().enumerate() {
        h.set(&format!("ZNAXIS{}", i + 1), n as i64);
    }
    for (i, &t) in tiles.iter().enumerate() {
        h.set(&format!("ZTILE{}", i + 1), t as i64);
    }
    if cmptype == "RICE_1" {
        h.set("ZNAME1", "BLOCKSIZE").set("ZVAL1", 32);
        h.set("ZNAME2", "BYTEPIX").set("ZVAL2", bytepix as i64);
    }
    Ok((h, data))
}

/// Widen integer image samples to `i64` (float buffers yield empty).
fn widen(samples: &ImageData) -> Vec<i64> {
    match samples {
        ImageData::U8(v) => v.iter().map(|&x| x as i64).collect(),
        ImageData::I16(v) => v.iter().map(|&x| x as i64).collect(),
        ImageData::I32(v) => v.iter().map(|&x| x as i64).collect(),
        ImageData::I64(v) => v.clone(),
        _ => Vec::new(),
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

fn column_at(col: &Option<Vec<f64>>, t: usize) -> Option<f64> {
    col.as_ref().and_then(|v| v.get(t).copied())
}

/// Decode one tile, honoring the fallback columns: the primary `COMPRESSED_DATA`
/// (via `ZCMPTYPE`), else gzip'd `GZIP_COMPRESSED_DATA`, else raw `UNCOMPRESSED_DATA`.
#[allow(clippy::too_many_arguments)]
fn decode_one_tile(
    cmptype: &str,
    primary: Option<&ColumnData>,
    gzip_fallback: Option<&ColumnData>,
    uncompressed: Option<&ColumnData>,
    tile_elems: usize,
    int_bitpix: Bitpix,
    blocksize: usize,
    bytepix: usize,
) -> Result<Vec<i64>> {
    if let Some(cell) = primary.filter(|c| cell_len(c) > 0) {
        decode_tile_cell(cmptype, cell, tile_elems, int_bitpix, blocksize, bytepix)
    } else if let Some(cell) = gzip_fallback.filter(|c| cell_len(c) > 0) {
        gzip::gzip_tile(as_bytes(cell)?, int_bitpix)
    } else if let Some(cell) = uncompressed.filter(|c| cell_len(c) > 0) {
        Ok(cell_to_i64(cell))
    } else {
        Err(FitsError::UnsupportedCompression {
            name: "empty tile (no compressed or uncompressed data)".to_string(),
        })
    }
}

/// Decode one tile of a *float* image. A primary `COMPRESSED_DATA` cell holds
/// quantized integers (dequantized as `scale·int + zero`); otherwise the
/// `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks hold the raw float values.
#[allow(clippy::too_many_arguments)]
fn decode_float_tile(
    cmptype: &str,
    primary: Option<&ColumnData>,
    gzip_fallback: Option<&ColumnData>,
    uncompressed: Option<&ColumnData>,
    tile_elems: usize,
    zbitpix: Bitpix,
    int_bitpix: Bitpix,
    blocksize: usize,
    bytepix: usize,
    scale: f64,
    zero: f64,
) -> Result<Vec<f64>> {
    if let Some(cell) = primary.filter(|c| cell_len(c) > 0) {
        let ints = decode_tile_cell(cmptype, cell, tile_elems, int_bitpix, blocksize, bytepix)?;
        Ok(ints.iter().map(|&v| scale * v as f64 + zero).collect())
    } else if let Some(cell) = gzip_fallback.filter(|c| cell_len(c) > 0) {
        Ok(be_floats(&gzip::gunzip(as_bytes(cell)?)?, zbitpix))
    } else if let Some(cell) = uncompressed.filter(|c| cell_len(c) > 0) {
        Ok(cell_to_f64(cell, zbitpix))
    } else {
        Err(FitsError::UnsupportedCompression {
            name: "empty float tile".to_string(),
        })
    }
}

/// Decode one tile's primary `COMPRESSED_DATA` cell into `tile_elems` integer
/// values, per `ZCMPTYPE`. The cell is a byte array except for `PLIO_1` (i16).
fn decode_tile_cell(
    cmptype: &str,
    cell: &ColumnData,
    tile_elems: usize,
    int_bitpix: Bitpix,
    blocksize: usize,
    bytepix: usize,
) -> Result<Vec<i64>> {
    match cmptype {
        "GZIP_1" => gzip::gzip_tile(as_bytes(cell)?, int_bitpix),
        "GZIP_2" => gzip::gzip2_tile(as_bytes(cell)?, int_bitpix),
        "RICE_1" => Ok(rice::rice_decode(
            as_bytes(cell)?,
            tile_elems,
            bytepix,
            blocksize,
        )),
        "PLIO_1" => Ok(plio::plio_decode(as_i16(cell)?, tile_elems)),
        "HCOMPRESS_1" => hcompress::hcompress_tile(as_bytes(cell)?),
        other => Err(FitsError::UnsupportedCompression {
            name: other.to_string(),
        }),
    }
}

/// Strides give the flat indices in the full image for a tile's row-major elements.
fn tile_flat_indices(origin: &[usize], tdims: &[usize], stride: &[usize]) -> Vec<usize> {
    let tile_elems: usize = tdims.iter().product();
    (0..tile_elems)
        .map(|local| {
            let mut rem = local;
            let mut flat = 0;
            for i in 0..tdims.len() {
                let c = rem % tdims[i];
                rem /= tdims[i];
                flat += (origin[i] + c) * stride[i];
            }
            flat
        })
        .collect()
}

/// Read `PREFIX1..PREFIXn` integer axis lengths.
fn read_axes(header: &Header, prefix: &str, n: usize) -> Result<Vec<usize>> {
    (1..=n)
        .map(|i| {
            header
                .get_integer(&format!("{prefix}{i}"))
                .map(|v| v.max(0) as usize)
                .ok_or(FitsError::MissingKeyword { name: "ZNAXISn" })
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

/// Narrow a widened `i64` buffer back to the typed samples of `bitpix`.
fn narrow(values: Vec<i64>, bitpix: Bitpix) -> ImageData {
    match bitpix {
        Bitpix::U8 => ImageData::U8(values.iter().map(|&v| v as u8).collect()),
        Bitpix::I16 => ImageData::I16(values.iter().map(|&v| v as i16).collect()),
        Bitpix::I32 => ImageData::I32(values.iter().map(|&v| v as i32).collect()),
        Bitpix::I64 => ImageData::I64(values),
        Bitpix::F32 => ImageData::F32(Vec::new()),
        Bitpix::F64 => ImageData::F64(Vec::new()),
    }
}

/// Build float samples from a dequantized `f64` buffer.
fn float_samples(values: Vec<f64>, zbitpix: Bitpix) -> ImageData {
    match zbitpix {
        Bitpix::F32 => ImageData::F32(values.iter().map(|&v| v as f32).collect()),
        _ => ImageData::F64(values),
    }
}

#[cfg(test)]
mod tests;
