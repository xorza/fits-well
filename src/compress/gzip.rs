//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use std::io::Read;

use crate::bitpix::Bitpix;
use crate::error::Result;

use super::be_to_i64;

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
