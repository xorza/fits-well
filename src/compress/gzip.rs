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

/// Default deflate level used by [`crate::Gzip::default`]. Level 1 favors write
/// speed (gzip was the slowest compress path at the higher default); construct
/// [`crate::Gzip::new`] or [`crate::Gzip::shuffled`] with a higher level for a
/// tighter ratio.
pub(crate) const DEFAULT_GZIP_LEVEL: u32 = 1;

#[derive(Debug, Default)]
pub(crate) struct GzipScratch {
    pub(crate) bytes: Vec<u8>,
    reordered: Vec<u8>,
}

/// Gzip a raw big-endian byte buffer at deflate `level` (0–9; the `GZIP_1` tile
/// encoder). The level is lossless — only the speed↔ratio tradeoff changes.
pub(crate) fn gzip_encode(raw: &[u8], level: u32) -> Vec<u8> {
    let mut enc = GzEncoder::new(Vec::new(), Compression::new(level));
    enc.write_all(raw).expect("gzip into a Vec cannot fail");
    enc.finish().expect("gzip finish into a Vec cannot fail")
}

/// `GZIP_2` encoder: shuffle `raw` into significance byte-planes, then gzip at `level`.
pub(crate) fn gzip2_encode(
    raw: &[u8],
    width: usize,
    level: u32,
    scratch: &mut GzipScratch,
) -> Vec<u8> {
    if width <= 1 {
        return gzip_encode(raw, level);
    }
    shuffle_bytes_into(raw, width, &mut scratch.reordered);
    gzip_encode(&scratch.reordered, level)
}

/// Shuffle `raw` into `width`-byte significance planes (all byte-0s, then all
/// byte-1s, …) — the `GZIP_2` pre-pass. `width ≤ 1` is a no-op.
pub(crate) fn shuffle_bytes_into(raw: &[u8], width: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(raw.len(), 0);
    if width <= 1 {
        out.copy_from_slice(raw);
        return;
    }
    let n = raw.len() / width;
    for p in 0..width {
        for i in 0..n {
            out[p * n + i] = raw[i * width + p];
        }
    }
}

/// Inverse of [`shuffle_bytes_into`]: gather significance planes back into elements.
pub(crate) fn unshuffle_bytes_into(shuffled: &[u8], width: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(shuffled.len(), 0);
    if width <= 1 {
        out.copy_from_slice(shuffled);
        return;
    }
    let n = shuffled.len() / width;
    for p in 0..width {
        for i in 0..n {
            out[i * width + p] = shuffled[p * n + i];
        }
    }
}

/// Inflate a gzip stream to its exact declared size. Reading one byte past the
/// expected size detects expansion bombs without allowing an unbounded allocation.
pub(crate) fn gunzip_into(bytes: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
    out.clear();
    GzDecoder::new(bytes)
        .take(expected.saturating_add(1) as u64)
        .read_to_end(out)?;
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
    Ok(())
}

pub(crate) fn gunzip2_into(
    bytes: &[u8],
    expected: usize,
    width: usize,
    inflated: &mut Vec<u8>,
    out: &mut Vec<u8>,
) -> Result<()> {
    if width <= 1 {
        return gunzip_into(bytes, expected, out);
    }
    gunzip_into(bytes, expected, inflated)?;
    unshuffle_bytes_into(inflated, width, out);
    Ok(())
}

/// `GZIP_1`: inflate to the tile's big-endian byte stream, then decode per `bitpix`
/// into `out` (a reused buffer). The tile holds `tile_elems` values, bounding the
/// inflated size at `tile_elems × bitpix` bytes.
pub(crate) fn gzip_tile_into(
    bytes: &[u8],
    bitpix: Bitpix,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut GzipScratch,
) -> Result<()> {
    gunzip_into(
        bytes,
        tile_elems.saturating_mul(bitpix.elem_size()),
        &mut scratch.bytes,
    )?;
    be_to_i64_into(&scratch.bytes, bitpix, out);
    Ok(())
}

/// `GZIP_2`: like `GZIP_1` but the bytes are shuffled into significance planes
/// (all most-significant bytes first, …) before gzip. Inflate, then un-shuffle.
pub(crate) fn gzip2_tile_into(
    bytes: &[u8],
    bitpix: Bitpix,
    tile_elems: usize,
    out: &mut Vec<i64>,
    scratch: &mut GzipScratch,
) -> Result<()> {
    let expected = tile_elems.saturating_mul(bitpix.elem_size());
    let GzipScratch {
        bytes: raw,
        reordered,
    } = scratch;
    gunzip2_into(bytes, expected, bitpix.elem_size(), reordered, raw)?;
    be_to_i64_into(raw, bitpix, out);
    Ok(())
}
