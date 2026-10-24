//! `NOCOMPRESS` (§10.4): a tile stored as its samples' big-endian bytes.

use crate::error::{Result, SizeUnit};

/// Append the tile's big-endian bytes `raw` to `out`.
pub(super) fn encode(raw: &[u8], out: &mut Vec<u8>) {
    out.extend_from_slice(raw);
}

/// The tile's bytes from `stream`, which holds exactly the `expected` bytes of its
/// samples.
pub(super) const fn decode(stream: &[u8], expected: usize) -> Result<&[u8]> {
    if stream.len() != expected {
        return Err(SizeUnit::Bytes.mismatch(expected, stream.len()));
    }
    Ok(stream)
}
