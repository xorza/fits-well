//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use std::io::Read;
use std::io::Write;

use crate::bitpix::Bitpix;
use crate::error::Result;

use super::be_to_i64;

/// Gzip a raw big-endian byte buffer (the `GZIP_1` tile encoder).
pub(super) fn gzip_encode(raw: &[u8]) -> Vec<u8> {
    let mut enc = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    enc.write_all(raw).expect("gzip into a Vec cannot fail");
    enc.finish().expect("gzip finish into a Vec cannot fail")
}

/// `GZIP_2` encoder: shuffle `raw` into significance byte-planes, then gzip.
pub(super) fn gzip2_encode(raw: &[u8], width: usize) -> Vec<u8> {
    if width <= 1 {
        return gzip_encode(raw);
    }
    let n = raw.len() / width;
    let mut shuffled = vec![0u8; raw.len()];
    for p in 0..width {
        for i in 0..n {
            shuffled[p * n + i] = raw[i * width + p];
        }
    }
    gzip_encode(&shuffled)
}

/// Shuffle `raw` into `width`-byte significance planes (all byte-0s, then all
/// byte-1s, …) — the `GZIP_2` pre-pass. `width ≤ 1` is a no-op.
pub(super) fn shuffle_bytes(raw: &[u8], width: usize) -> Vec<u8> {
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
pub(super) fn unshuffle_bytes(shuffled: &[u8], width: usize) -> Vec<u8> {
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

/// Inflate a gzip stream to its raw bytes.
pub(super) fn gunzip(bytes: &[u8]) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::GzDecoder::new(bytes).read_to_end(&mut out)?;
    Ok(out)
}

/// `GZIP_1`: inflate to the tile's big-endian byte stream, then decode per `bitpix`.
pub(super) fn gzip_tile(bytes: &[u8], bitpix: Bitpix) -> Result<Vec<i64>> {
    Ok(be_to_i64(&gunzip(bytes)?, bitpix))
}

/// `GZIP_2`: like `GZIP_1` but the bytes are shuffled into significance planes
/// (all most-significant bytes first, …) before gzip. Inflate, then un-shuffle.
pub(super) fn gzip2_tile(bytes: &[u8], bitpix: Bitpix) -> Result<Vec<i64>> {
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
