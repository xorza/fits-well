//! Tiled image decompression (§10.1) — behind the `compression` feature.
//!
//! A compressed image is a `BINTABLE` with `ZIMAGE = T`: the original image
//! (`ZBITPIX`, `ZNAXISn`) is split into `ZTILEn` tiles, each compressed and stored
//! as a variable-length byte array in the `COMPRESSED_DATA` column. This module
//! decompresses the tiles (`GZIP_1`, `RICE_1`) and reassembles the full image.
//! Floating-point quantization (`ZSCALE`/`ZZERO`) and `HCOMPRESS_1`/`PLIO_1` are
//! not yet handled.

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
    if zbitpix.is_float() {
        // Float images are quantized to integers (ZSCALE/ZZERO) — not yet handled.
        return Err(FitsError::UnsupportedCompression {
            name: "floating-point quantization".to_string(),
        });
    }
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

    // The compressed tile bytes: one variable-length byte array per row/tile.
    let col = table
        .column_index("COMPRESSED_DATA")
        .ok_or(FitsError::MissingKeyword {
            name: "COMPRESSED_DATA",
        })?;
    let tile_bytes = table.read_vla_column(col)?;

    let ntiles_axis: Vec<usize> = dims
        .iter()
        .zip(&tiles)
        .map(|(&n, &t)| n.div_ceil(t))
        .collect();
    let ntiles: usize = ntiles_axis.iter().product();
    if tile_bytes.len() != ntiles {
        return Err(FitsError::RowWidthMismatch {
            computed: tile_bytes.len(),
            declared: ntiles,
        });
    }

    // Strides for the full image (axis 0 fastest).
    let mut stride = vec![1usize; znaxis];
    for i in 1..znaxis {
        stride[i] = stride[i - 1] * dims[i - 1];
    }

    let total: usize = dims.iter().product();
    let mut out = vec![0i64; total];
    for (t, cell) in tile_bytes.iter().enumerate() {
        let bytes = match cell {
            ColumnData::Bytes(b) => b.as_slice(),
            _ => {
                return Err(FitsError::UnsupportedCompression {
                    name: "COMPRESSED_DATA is not a byte array".to_string(),
                });
            }
        };
        // Tile origin and (possibly clipped at edges) dimensions.
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
        let values = decode_tile(&cmptype, bytes, tile_elems, zbitpix, blocksize, bytepix)?;
        scatter_tile(&mut out, &values, &origin, &tdims, &stride);
    }

    Ok(Image {
        shape: dims,
        samples: narrow(out, zbitpix),
        scaling: Scaling::from_header(header),
    })
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

/// Scatter a row-major tile (`tdims`, axis 0 fastest) into the full image `out`.
fn scatter_tile(
    out: &mut [i64],
    values: &[i64],
    origin: &[usize],
    tdims: &[usize],
    stride: &[usize],
) {
    for (local, &v) in values.iter().enumerate() {
        let mut rem = local;
        let mut flat = 0;
        for i in 0..tdims.len() {
            let c = rem % tdims[i];
            rem /= tdims[i];
            flat += (origin[i] + c) * stride[i];
        }
        out[flat] = v;
    }
}

/// Decode one tile's compressed bytes into `tile_elems` integer values.
fn decode_tile(
    cmptype: &str,
    bytes: &[u8],
    tile_elems: usize,
    zbitpix: Bitpix,
    blocksize: usize,
    bytepix: usize,
) -> Result<Vec<i64>> {
    match cmptype {
        "GZIP_1" => gzip_tile(bytes, tile_elems, zbitpix),
        "RICE_1" => Ok(rice_decode(bytes, tile_elems, bytepix, blocksize)),
        other => Err(FitsError::UnsupportedCompression {
            name: other.to_string(),
        }),
    }
}

/// `GZIP_1`: inflate to the tile's big-endian byte stream, then decode per `ZBITPIX`.
fn gzip_tile(bytes: &[u8], tile_elems: usize, zbitpix: Bitpix) -> Result<Vec<i64>> {
    let mut raw = Vec::with_capacity(tile_elems * zbitpix.elem_size());
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut raw)?;
    Ok(be_to_i64(&raw, zbitpix))
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
