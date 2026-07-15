//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use std::io::Read;
use std::io::Write;

use crate::bitpix::Bitpix;
use crate::error::FitsError;
use crate::error::Result;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

use crate::compress::convert::be_to_i64_into;

/// Default deflate level — the [`crate::CompressOptions`] default and the fixed
/// level for table-column gzip. Level 1 favors write speed (gzip was the slowest
/// compress path at the higher default); raise `CompressOptions::gzip_level` for a
/// tighter ratio.
pub(crate) const DEFAULT_GZIP_LEVEL: u32 = 1;

/// Gzip a raw big-endian byte buffer at deflate `level` (0–9; the `GZIP_1` tile
/// encoder). The level is lossless — only the speed↔ratio tradeoff changes.
pub(crate) fn gzip_encode(raw: &[u8], level: u32) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(level));
    enc.write_all(raw).expect("gzip into a Vec cannot fail");
    enc.finish().expect("gzip finish into a Vec cannot fail")
}

/// `GZIP_2` encoder: shuffle `raw` into significance byte-planes, then gzip at `level`.
pub(crate) fn gzip2_encode(raw: &[u8], width: usize, level: u32) -> Vec<u8> {
    gzip_encode(&shuffle_bytes(raw, width), level)
}

/// Shuffle `raw` into `width`-byte significance planes (all byte-0s, then all
/// byte-1s, …) — the `GZIP_2` pre-pass. `width ≤ 1` is a no-op.
pub(crate) fn shuffle_bytes(raw: &[u8], width: usize) -> Vec<u8> {
    if width <= 1 {
        return raw.to_vec();
    }
    let n = raw.len() / width;
    let mut out = vec![0u8; raw.len()];
    for p in 0..width {
        for i in 0..n {
            out[p * n + i] = raw[i * width + p];
        }
    }
    out
}

/// Inverse of [`shuffle_bytes`]: gather significance planes back into elements.
pub(crate) fn unshuffle_bytes(shuffled: &[u8], width: usize) -> Vec<u8> {
    if width <= 1 {
        return shuffled.to_vec();
    }
    let n = shuffled.len() / width;
    let mut out = vec![0u8; shuffled.len()];
    for p in 0..width {
        for i in 0..n {
            out[i * width + p] = shuffled[p * n + i];
        }
    }
    out
}

/// Inflate a gzip stream to its exact declared size. Reading one byte past the
/// expected size detects expansion bombs without allowing an unbounded allocation.
pub(crate) fn gunzip(bytes: &[u8], expected: usize) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    GzDecoder::new(bytes)
        .take(expected.saturating_add(1) as u64)
        .read_to_end(&mut out)?;
    if out.len() > expected {
        return Err(FitsError::UnsupportedCompression {
            name: "gzip tile expands beyond its declared tile size".to_string(),
        });
    }
    if out.len() != expected {
        return Err(FitsError::DataSizeMismatch {
            expected,
            got: out.len(),
        });
    }
    Ok(out)
}

/// `GZIP_1`: inflate to the tile's big-endian byte stream, then decode per `bitpix`
/// into `out` (a reused buffer). The tile holds `tile_elems` values, bounding the
/// inflated size at `tile_elems × bitpix` bytes.
pub(crate) fn gzip_tile_into(
    bytes: &[u8],
    bitpix: Bitpix,
    tile_elems: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    let raw = gunzip(bytes, tile_elems.saturating_mul(bitpix.elem_size()))?;
    be_to_i64_into(&raw, bitpix, out);
    Ok(())
}

/// `GZIP_2`: like `GZIP_1` but the bytes are shuffled into significance planes
/// (all most-significant bytes first, …) before gzip. Inflate, then un-shuffle.
pub(crate) fn gzip2_tile_into(
    bytes: &[u8],
    bitpix: Bitpix,
    tile_elems: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    let raw = unshuffle_bytes(
        &gunzip(bytes, tile_elems.saturating_mul(bitpix.elem_size()))?,
        bitpix.elem_size(),
    );
    be_to_i64_into(&raw, bitpix, out);
    Ok(())
}
