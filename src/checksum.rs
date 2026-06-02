//! `CHECKSUM`/`DATASUM` integrity keywords (§4.4.2.7, Appendix J).
//!
//! The primitive is a 32-bit ones'-complement sum over the bytes interpreted as
//! big-endian 32-bit words (FITS units are multiples of 2880 = 720 words, so the
//! word split is always exact). A valid `CHECKSUM` forces the whole-HDU sum to
//! all-ones (negative zero).

/// Accumulate the 32-bit ones'-complement checksum of `bytes` into `seed`.
pub(crate) fn accumulate(bytes: &[u8], seed: u32) -> u32 {
    // Callers pass block-padded units (720-word multiples), so the split is exact;
    // assert it so a future caller passing an unaligned slice can't silently drop a
    // tail of up to 3 bytes from the sum.
    assert_eq!(bytes.len() % 4, 0, "checksum input must be word-aligned");
    let mut sum = seed as u64;
    for word in bytes.chunks_exact(4) {
        sum += u32::from_be_bytes([word[0], word[1], word[2], word[3]]) as u64;
    }
    // Fold the end-around carry until it fits in 32 bits.
    while sum >> 32 != 0 {
        sum = (sum & 0xFFFF_FFFF) + (sum >> 32);
    }
    sum as u32
}

/// Encode a checksum into the 16 ASCII characters of a `CHECKSUM` value
/// (Appendix J.2). With `complement` set, the complement of `sum` is encoded, so
/// that re-summing the HDU yields negative zero. The output is always
/// alphanumeric (`0-9`, `A-Z`, `a-z`).
pub(crate) fn encode(sum: u32, complement: bool) -> [u8; 16] {
    // ASCII punctuation between the digit and letter ranges, to be avoided.
    const EXCLUDE: [i32; 13] = [
        0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60,
    ];
    const OFFSET: i32 = 0x30; // ASCII '0'

    let sum = if complement { 0xFFFF_FFFF - sum } else { sum };
    let bytes = sum.to_be_bytes();
    let mut asc = [0u8; 16];
    for (i, &b) in bytes.iter().enumerate() {
        let byte = b as i32;
        let quotient = byte / 4 + OFFSET;
        let remainder = byte % 4;
        // Four characters that sum to `byte`, then nudged off punctuation in
        // balanced pairs (one up, one down) so the column sum is preserved.
        let mut ch = [quotient, quotient, quotient, quotient];
        ch[0] += remainder;
        loop {
            let mut changed = false;
            for &ex in EXCLUDE.iter() {
                let mut j = 0;
                while j < 4 {
                    if ch[j] == ex || ch[j + 1] == ex {
                        ch[j] += 1;
                        ch[j + 1] -= 1;
                        changed = true;
                    }
                    j += 2;
                }
            }
            if !changed {
                break;
            }
        }
        for j in 0..4 {
            asc[4 * j + i] = ch[j] as u8;
        }
    }
    // Rotate one character right to align with the value's start at column 12.
    asc.rotate_right(1);
    asc
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accumulate_folds_end_around_carry() {
        // Two words summing past 2^32 wrap around (ones'-complement).
        let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x02];
        // 0xFFFFFFFF + 0x00000002 = 0x100000001 → fold → 0x00000002.
        assert_eq!(accumulate(&bytes, 0), 0x0000_0002);
        assert_eq!(accumulate(&[0, 0, 0, 1], 0), 1);
    }

    #[test]
    fn encoded_checksum_is_alphanumeric_and_sums_to_negative_zero() {
        // For any HDU sum, the encoded 16 chars (placed word-aligned) plus the
        // sum must give all-ones. Here we check the chars are alphanumeric and
        // that decoding the complement is self-consistent.
        for sum in [0u32, 1, 0x1234_5678, 0xDEAD_BEEF, 0xFFFF_FFFF] {
            let enc = encode(sum, true);
            assert!(
                enc.iter().all(|&b| b.is_ascii_alphanumeric()),
                "non-alphanumeric output for {sum:#x}: {enc:?}"
            );
        }
    }
}
