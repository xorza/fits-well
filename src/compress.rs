//! Tiled image decompression (§10.1) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! in `COMPRESSED_DATA` (with `GZIP_COMPRESSED_DATA`/`UNCOMPRESSED_DATA` fallbacks).
//! This module decompresses the tiles and reassembles the full image.
//!
//! Supported: `GZIP_1`, `GZIP_2`, `RICE_1`; float images via per-tile
//! `ZSCALE`/`ZZERO` linear dequantization (`NO_DITHER`) or the raw-float gzip
//! fallback. Not yet: `PLIO_1`, `HCOMPRESS_1`, subtractive dithering, `ZBLANK`,
//! and all compression *writing*.

use std::io::Read;

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::data::Scaling;
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

    let (blocksize, bytepix) = rice_params(header, zbitpix);
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

/// Read a compressed-data column's per-tile cells, or `None` if absent.
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
        gzip_tile(as_bytes(cell)?, tile_elems, int_bitpix)
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
        Ok(be_floats(&gunzip(as_bytes(cell)?)?, zbitpix))
    } else if let Some(cell) = uncompressed.filter(|c| cell_len(c) > 0) {
        Ok(cell_to_f64(cell, zbitpix))
    } else {
        Err(FitsError::UnsupportedCompression {
            name: "empty float tile".to_string(),
        })
    }
}

/// Decode a big-endian buffer of `bitpix` floats into `f64`.
fn be_floats(bytes: &[u8], bitpix: Bitpix) -> Vec<f64> {
    match bitpix {
        Bitpix::F32 => bytes
            .chunks_exact(4)
            .map(|c| f32::from_be_bytes([c[0], c[1], c[2], c[3]]) as f64)
            .collect(),
        Bitpix::F64 => bytes
            .chunks_exact(8)
            .map(|c| f64::from_be_bytes(c.try_into().expect("8-byte chunk")))
            .collect(),
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

fn bytepix_to_bitpix(bytepix: usize) -> Bitpix {
    match bytepix {
        1 => Bitpix::U8,
        2 => Bitpix::I16,
        8 => Bitpix::I64,
        _ => Bitpix::I32,
    }
}

/// Build float samples from a dequantized `f64` buffer.
fn float_samples(values: Vec<f64>, zbitpix: Bitpix) -> ImageData {
    match zbitpix {
        Bitpix::F32 => ImageData::F32(values.iter().map(|&v| v as f32).collect()),
        _ => ImageData::F64(values),
    }
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

/// Rice `(blocksize, bytepix)` from the `ZNAMEi`/`ZVALi` parameters, defaulting to
/// 32 and `|ZBITPIX|/8`.
fn rice_params(header: &Header, zbitpix: Bitpix) -> (usize, usize) {
    let mut blocksize = 32;
    let mut bytepix = zbitpix.elem_size();
    let mut i = 1;
    while let Some(name) = header.get_text(&format!("ZNAME{i}")) {
        if let Some(v) = header.get_integer(&format!("ZVAL{i}")) {
            match name {
                "BLOCKSIZE" => blocksize = v.max(1) as usize,
                "BYTEPIX" => bytepix = v.max(1) as usize,
                _ => {}
            }
        }
        i += 1;
    }
    (blocksize, bytepix)
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
        "GZIP_1" => gzip_tile(as_bytes(cell)?, tile_elems, int_bitpix),
        "GZIP_2" => gzip2_tile(as_bytes(cell)?, tile_elems, int_bitpix),
        "RICE_1" => Ok(rice_decode(as_bytes(cell)?, tile_elems, bytepix, blocksize)),
        other => Err(FitsError::UnsupportedCompression {
            name: other.to_string(),
        }),
    }
}

/// Inflate a gzip stream to its raw bytes.
fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// `GZIP_1`: inflate to the tile's big-endian byte stream, then decode per `bitpix`.
fn gzip_tile(bytes: &[u8], _tile_elems: usize, bitpix: Bitpix) -> Result<Vec<i64>> {
    Ok(be_to_i64(&gunzip(bytes)?, bitpix))
}

/// `GZIP_2`: like `GZIP_1` but the bytes are shuffled into significance planes
/// (all most-significant bytes first, …) before gzip. Inflate, then un-shuffle.
fn gzip2_tile(bytes: &[u8], _tile_elems: usize, bitpix: Bitpix) -> Result<Vec<i64>> {
    let width = bitpix.elem_size();
    let shuffled = gunzip(bytes)?;
    if width == 1 {
        return Ok(be_to_i64(&shuffled, bitpix));
    }
    let n = shuffled.len() / width;
    let mut raw = vec![0u8; shuffled.len()];
    // Plane p (p=0 most significant) holds byte p of every value, in order.
    for p in 0..width {
        for i in 0..n {
            raw[i * width + p] = shuffled[p * n + i];
        }
    }
    Ok(be_to_i64(&raw, bitpix))
}

/// Decode a big-endian buffer of `bitpix` integers into widened `i64` values.
fn be_to_i64(bytes: &[u8], bitpix: Bitpix) -> Vec<i64> {
    match bitpix {
        Bitpix::U8 => bytes.iter().map(|&b| b as i64).collect(),
        Bitpix::I16 => bytes
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]) as i64)
            .collect(),
        Bitpix::I32 => bytes
            .chunks_exact(4)
            .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]) as i64)
            .collect(),
        Bitpix::I64 => bytes
            .chunks_exact(8)
            .map(|c| i64::from_be_bytes(c.try_into().expect("8-byte chunk")))
            .collect(),
        Bitpix::F32 | Bitpix::F64 => Vec::new(), // excluded before this point
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

/// A MSB-first bit reader over a compressed byte stream.
struct BitReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    fn new(bytes: &'a [u8]) -> Self {
        BitReader {
            bytes,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    /// Read `n` bits (MSB-first); past end-of-input reads as zero bits.
    fn read(&mut self, n: u32) -> u64 {
        if n == 0 {
            return 0;
        }
        while self.nbits < n {
            let byte = self.bytes.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.acc = (self.acc << 8) | byte as u64;
            self.nbits += 8;
        }
        self.nbits -= n;
        (self.acc >> self.nbits) & ((1u64 << n) - 1)
    }

    /// Count and consume leading zero bits up to (and including) the next 1.
    fn read_zeros(&mut self) -> u64 {
        let mut z = 0;
        while self.read(1) == 0 {
            z += 1;
        }
        z
    }
}

/// Decode a `RICE_1` tile into `nx` integer values (cfitsio bitstream layout).
fn rice_decode(bytes: &[u8], nx: usize, bytepix: usize, blocksize: usize) -> Vec<i64> {
    let nbits_pp = (8 * bytepix) as u32;
    let (fsbits, fsmax) = match bytepix {
        1 => (3u32, 6u32),
        2 => (4, 14),
        _ => (5, 25), // 4-byte (and wider) pixels
    };
    let mask = if nbits_pp >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits_pp) - 1
    };

    let mut br = BitReader::new(bytes);
    let mut lastpix = br.read(nbits_pp); // literal first pixel (big-endian)
    let mut out = Vec::with_capacity(nx);
    let mut i = 0;
    while i < nx {
        let fs = br.read(fsbits) as i64 - 1;
        let imax = (i + blocksize).min(nx);
        for _ in i..imax {
            let diff = if fs < 0 {
                0
            } else if fs as u32 == fsmax {
                br.read(nbits_pp) // uncompressed block
            } else {
                (br.read_zeros() << fs) | br.read(fs as u32)
            };
            // Undo the zigzag mapping, then the differencing (modular at pixel width).
            let d = if diff & 1 == 1 {
                !(diff >> 1)
            } else {
                diff >> 1
            };
            lastpix = lastpix.wrapping_add(d) & mask;
            out.push(sign_extend(lastpix, nbits_pp));
        }
        i = imax;
    }
    out
}

/// Interpret the low `nbits` of `v` as a two's-complement signed value.
fn sign_extend(v: u64, nbits: u32) -> i64 {
    let shift = 64 - nbits;
    ((v << shift) as i64) >> shift
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::FitsReader;
    use std::fs::File;

    fn open(name: &str) -> FitsReader<File> {
        FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap()
    }

    /// The fixtures encode value(x, y) = x*7 − y*5 over a 24×16 i16 image.
    fn expect_pixel(flat: usize) -> i16 {
        let (x, y) = (flat % 24, flat / 24);
        (x as i16) * 7 - (y as i16) * 5
    }

    fn check_decoded(name: &str) {
        let mut f = open(name);
        let img = f.read_compressed_image(1).unwrap();
        assert_eq!(img.shape, vec![24, 16]);
        match img.samples {
            ImageData::I16(v) => {
                assert_eq!(v.len(), 24 * 16);
                for (i, &got) in v.iter().enumerate() {
                    assert_eq!(got, expect_pixel(i), "pixel {i} of {name}");
                }
            }
            other => panic!("expected I16, got {other:?}"),
        }
    }

    #[test]
    fn decompresses_gzip_1_tiled_image() {
        check_decoded("comp_gzip_i16.fits");
    }

    #[test]
    fn decompresses_rice_1_tiled_image() {
        check_decoded("comp_rice_i16.fits");
    }

    #[test]
    fn decompresses_gzip_2_tiled_image() {
        check_decoded("comp_gzip2_i16.fits");
    }

    /// Compare a compressed-float decode against astropy's reconstructed reference.
    fn check_float(compressed: &str, reference: &str) {
        let got = match open(compressed).read_compressed_image(1).unwrap().samples {
            ImageData::F32(v) => v,
            other => panic!("expected F32, got {other:?}"),
        };
        let want = match open(reference).read_image(0).unwrap().samples {
            ImageData::F32(v) => v,
            other => panic!("expected F32 reference, got {other:?}"),
        };
        assert_eq!(got.len(), 24 * 16);
        assert_eq!(got, want, "{compressed} must match astropy");
    }

    #[test]
    fn decompresses_unquantized_float_via_gzip_fallback() {
        // Smooth data stored losslessly: ZSCALE=0, raw floats gzip'd in
        // GZIP_COMPRESSED_DATA (COMPRESSED_DATA empty).
        check_float("comp_ricef_nodither.fits", "comp_ref_f32.fits");
    }

    #[test]
    fn decompresses_quantized_float_no_dither() {
        // Noisy data genuinely quantized: per-tile ZSCALE≠0, integers RICE-packed in
        // COMPRESSED_DATA, dequantized as ZSCALE·int + ZZERO.
        check_float("comp_ricef_quant.fits", "comp_ref_quant_f32.fits");
    }

    #[test]
    fn read_compressed_image_rejects_a_plain_bintable() {
        // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
        let mut f = open("DDTSUVDATA.fits");
        assert!(matches!(
            f.read_compressed_image(1),
            Err(FitsError::NotCompressedImage)
        ));
    }

    #[test]
    fn bit_reader_reads_msb_first() {
        let mut br = BitReader::new(&[0b1011_0010, 0b1111_0000]);
        assert_eq!(br.read(1), 1);
        assert_eq!(br.read(3), 0b011);
        assert_eq!(br.read(4), 0b0010);
        assert_eq!(br.read(4), 0b1111);
    }
}
