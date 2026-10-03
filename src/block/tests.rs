use crate::block::*;

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
            Some(blocks * BLOCK_SIZE as u64),
            "padded_len({len})"
        );
        // The writer's in-memory helper must agree with the rounding exactly:
        // the fill it emits is the difference the rounding implies, and a unit
        // already on a boundary takes none.
        assert_eq!(
            padding(len as usize),
            (padded_len(len).unwrap() - len) as usize,
            "padding({len})"
        );
    }
}

#[test]
fn padded_len_refuses_a_boundary_past_u64_max() {
    // ⌊u64::MAX / 2880⌋·2880 = 18446744073709549440, which is u64::MAX − 2175. One
    // byte more needs a block that ends 705 bytes past u64::MAX.
    let last = u64::MAX / 2880 * 2880;
    assert_eq!(last, 18_446_744_073_709_549_440);
    assert_eq!(padded_len(last), Some(last));
    assert_eq!(padded_len(last + 1), None);
    assert_eq!(padded_len(u64::MAX), None);
}
