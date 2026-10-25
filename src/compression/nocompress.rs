//! `NOCOMPRESS` (§10.4): a tile stored as its samples' big-endian bytes.

use crate::fits_error::{Result, SizeUnit};

/// The tile's bytes from `stream`, which holds exactly the `expected` bytes of its
/// samples.
pub(super) const fn decode(stream: &[u8], expected: usize) -> Result<&[u8]> {
    if stream.len() != expected {
        return Err(SizeUnit::Bytes.mismatch(expected, stream.len()));
    }
    Ok(stream)
}
