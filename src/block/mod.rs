//! The 2880-byte block grid — the I/O quantum of FITS.
//!
//! Every header unit and every data unit occupies a whole number of 2880-byte
//! blocks, with the final block padded to fill the boundary. Because of this, a
//! conforming file's length on disk is always a multiple of [`BLOCK_SIZE`].

/// The fundamental layout unit: 2880 bytes = 36 × 80-byte cards.
pub const BLOCK_SIZE: usize = 2880;

/// A keyword record (card) is 80 bytes of restricted ASCII.
pub const CARD_SIZE: usize = 80;

/// Fill byte for header units and ASCII-table data units: ASCII space.
pub(crate) const SPACE_FILL: u8 = b' ';

/// Fill byte for all data units except ASCII tables: NUL (all bits zero).
pub(crate) const ZERO_FILL: u8 = 0;

/// Number of whole 2880-byte blocks needed to hold `len` bytes, rounding up.
///
/// `blocks_for(0) == 0` — a zero-length unit (e.g. `NAXIS = 0` data) occupies no
/// blocks at all.
fn blocks_for(len: u64) -> u64 {
    len.div_ceil(BLOCK_SIZE as u64)
}

/// `len` rounded up to the next 2880-byte boundary (the on-disk unit length).
///
/// Saturating: an absurd `len` (within `2880` of `u64::MAX`, only reachable from
/// a hostile header) clamps to `u64::MAX` rather than wrapping to a too-small
/// length that would corrupt the next-HDU seek. `data_extent` already rejects
/// such sizes upstream; this keeps the rounding itself defense-complete.
pub(crate) fn padded_len(len: u64) -> u64 {
    blocks_for(len).saturating_mul(BLOCK_SIZE as u64)
}

pub(crate) fn checked_padded_len(len: u64) -> Option<u64> {
    blocks_for(len).checked_mul(BLOCK_SIZE as u64)
}

/// Fill bytes needed to round `len` up to the next 2880-byte boundary; `0` when it
/// already sits on one.
///
/// The in-memory counterpart to [`padded_len`], for the writer: one call site pads a
/// buffer it holds, the other streams the fill straight to the sink and never
/// materializes the padded unit at all, so they share the length rather than the
/// rounding.
pub(crate) fn padding(len: usize) -> usize {
    (BLOCK_SIZE - len % BLOCK_SIZE) % BLOCK_SIZE
}

#[cfg(test)]
mod tests;
