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
            blocks * BLOCK_SIZE as u64,
            "padded_len({len})"
        );
        // The writer's in-memory helper must agree with the rounding exactly:
        // the fill it emits is the difference the rounding implies, and a unit
        // already on a boundary takes none.
        assert_eq!(
            padding(len as usize),
            (padded_len(len) - len) as usize,
            "padding({len})"
        );
    }
}

#[test]
fn padded_len_saturates_instead_of_wrapping() {
    // An absurd length (only reachable from a hostile header) must clamp to
    // u64::MAX, never wrap: `blocks_for(u64::MAX) · 2880` overflows u64, and a
    // wrapping multiply yields a value far *smaller* than the input — which
    // would corrupt the next-HDU seek. Saturating keeps padded_len ≥ its input.
    assert_eq!(padded_len(u64::MAX), u64::MAX);
    assert_eq!(checked_padded_len(u64::MAX), None);
}
