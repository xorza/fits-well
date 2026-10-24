//! `CHECKSUM`/`DATASUM` integrity keywords (§4.4.2.7, Appendix J).
//!
//! The primitive is a 32-bit ones'-complement sum over the bytes interpreted as
//! big-endian 32-bit words (FITS units are multiples of 2880 = 720 words, so the
//! word split is always exact). A valid `CHECKSUM` forces the whole-HDU sum to
//! all-ones (negative zero).

pub(crate) mod checksum_report;
pub(crate) mod declared;
pub(crate) mod running;

/// The `CHECKSUM` value a header renders with before its sum is known (Appendix J.1).
pub(crate) const PLACEHOLDER: &str = "0000000000000000";

/// Accumulate the 32-bit ones'-complement checksum of `bytes` into `seed`.
pub(crate) fn accumulate(bytes: &[u8], seed: u32) -> u32 {
    debug_assert_eq!(bytes.len() % 4, 0, "checksum input must be word-aligned");
    let mut sum = seed;
    for word in bytes.as_chunks::<4>().0 {
        sum = combine(sum, u32::from_be_bytes(*word));
    }
    sum
}

pub(crate) const fn combine(left: u32, right: u32) -> u32 {
    let (sum, carry) = left.overflowing_add(right);
    sum.wrapping_add(carry as u32)
}

/// Encode the complement of `sum` into the 16 ASCII characters of a `CHECKSUM`
/// value (Appendix J.2), so that summing the HDU again yields negative zero. The
/// output is always alphanumeric (`0-9`, `A-Z`, `a-z`).
pub(crate) fn encode(sum: u32) -> [u8; 16] {
    // ASCII punctuation between the digit and letter ranges, to be avoided.
    const EXCLUDE: [i32; 13] = [
        0x3a, 0x3b, 0x3c, 0x3d, 0x3e, 0x3f, 0x40, 0x5b, 0x5c, 0x5d, 0x5e, 0x5f, 0x60,
    ];
    const OFFSET: i32 = 0x30; // ASCII '0'

    let sum = 0xFFFF_FFFF - sum;
    let bytes = sum.to_be_bytes();
    let mut asc = [0u8; 16];
    for (i, &b) in bytes.iter().enumerate() {
        let byte = i32::from(b);
        let quotient = byte / 4 + OFFSET;
        let remainder = byte % 4;
        // Four characters that sum to `byte`, then nudged off punctuation in
        // balanced pairs (one up, one down) so the column sum is preserved.
        let mut ch = [quotient, quotient, quotient, quotient];
        ch[0] += remainder;
        loop {
            let mut changed = false;
            for &ex in &EXCLUDE {
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
            asc[4 * j + i] = u8::try_from(ch[j]).expect("an encoded character is ASCII");
        }
    }
    // Rotate one character right to align with the value's start at column 12.
    asc.rotate_right(1);
    asc
}

#[cfg(test)]
mod tests;
