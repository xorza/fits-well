//! The 2880-byte block grid — the I/O quantum of FITS.
//!
//! Every header unit and every data unit occupies a whole number of 2880-byte
//! blocks, with the final block padded to fill the boundary. Because of this, a
//! conforming file's length on disk is always a multiple of [`BLOCK_SIZE`].

/// The fundamental layout unit: 2880 bytes = 36 × 80-byte cards.
pub const BLOCK_SIZE: usize = 2880;

/// A keyword record (card) is 80 bytes of restricted ASCII.
pub const CARD_SIZE: usize = 80;

/// `BLOCK_SIZE / CARD_SIZE` — exactly 36 cards per header block.
pub const CARDS_PER_BLOCK: usize = BLOCK_SIZE / CARD_SIZE;

/// Fill byte for header units and ASCII-table data units: ASCII space.
pub const SPACE_FILL: u8 = b' ';

/// Fill byte for all data units except ASCII tables: NUL (all bits zero).
pub const ZERO_FILL: u8 = 0;

/// Number of whole 2880-byte blocks needed to hold `len` bytes, rounding up.
///
/// `blocks_for(0) == 0` — a zero-length unit (e.g. `NAXIS = 0` data) occupies no
/// blocks at all.
pub fn blocks_for(len: u64) -> u64 {
    len.div_ceil(BLOCK_SIZE as u64)
}

/// `len` rounded up to the next 2880-byte boundary (the on-disk unit length).
pub fn padded_len(len: u64) -> u64 {
    blocks_for(len) * BLOCK_SIZE as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_geometry_constants_are_consistent() {
        assert_eq!(BLOCK_SIZE, 2880);
        assert_eq!(CARD_SIZE, 80);
        assert_eq!(CARDS_PER_BLOCK, 36);
        assert_eq!(CARDS_PER_BLOCK * CARD_SIZE, BLOCK_SIZE);
    }

    #[test]
    fn blocks_for_rounds_up_at_the_boundary() {
        // (input bytes, expected blocks)
        let cases = [
            (0u64, 0u64),
            (1, 1),
            (2879, 1),
            (2880, 1),
            (2881, 2),
            (5760, 2),
            (5761, 3),
        ];
        for (len, blocks) in cases {
            assert_eq!(blocks_for(len), blocks, "blocks_for({len})");
            assert_eq!(
                padded_len(len),
                blocks * BLOCK_SIZE as u64,
                "padded_len({len})"
            );
        }
    }

    #[test]
    fn padded_len_is_idempotent_on_aligned_input() {
        for blocks in [0u64, 1, 2, 199] {
            let aligned = blocks * BLOCK_SIZE as u64;
            assert_eq!(padded_len(aligned), aligned);
        }
    }
}
