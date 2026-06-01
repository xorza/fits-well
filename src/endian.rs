//! Big-endian scalar (de)serialization shared by the image, table, and
//! compression layers. FITS data is always big-endian, so every typed decode or
//! encode funnels through these three helpers.

/// Decode a packed big-endian buffer into host-endian values of a fixed-width
/// type, e.g. `decode_be(bytes, i16::from_be_bytes)`.
pub(crate) fn decode_be<const N: usize, T>(bytes: &[u8], conv: fn([u8; N]) -> T) -> Vec<T> {
    bytes
        .chunks_exact(N)
        .map(|c| conv(c.try_into().expect("chunks_exact yields N-byte arrays")))
        .collect()
}

/// Encode fixed-width values into a big-endian byte buffer, e.g.
/// `encode_be(values, i16::to_be_bytes)`.
pub(crate) fn encode_be<const N: usize, T: Copy>(values: &[T], conv: fn(T) -> [u8; N]) -> Vec<u8> {
    let mut out = Vec::with_capacity(values.len() * N);
    extend_be(&mut out, values, conv);
    out
}

/// Append fixed-width values to `out` in big-endian order.
pub(crate) fn extend_be<const N: usize, T: Copy>(
    out: &mut Vec<u8>,
    values: &[T],
    conv: fn(T) -> [u8; N],
) {
    for &v in values {
        out.extend_from_slice(&conv(v));
    }
}

/// Append a variable-length-array descriptor — element count and heap byte offset
/// — as a big-endian `Q` (64-bit, `wide`) or `P` (32-bit) pair. The values are
/// carried as `u64` up to here so a `Q` descriptor can address a heap or count
/// beyond the 4 GiB a `P` allows (truncating earlier would defeat `Q`'s purpose).
pub(crate) fn push_pq_descriptor(out: &mut Vec<u8>, wide: bool, count: u64, offset: u64) {
    if wide {
        out.extend_from_slice(&(count as i64).to_be_bytes());
        out.extend_from_slice(&(offset as i64).to_be_bytes());
    } else {
        out.extend_from_slice(&(count as i32).to_be_bytes());
        out.extend_from_slice(&(offset as i32).to_be_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decode_and_encode_are_inverse_and_big_endian() {
        assert_eq!(
            decode_be(&[0x00, 0x01, 0xFF, 0xFF], i16::from_be_bytes),
            vec![1i16, -1]
        );
        assert_eq!(
            encode_be(&[1i16, -1], i16::to_be_bytes),
            vec![0, 1, 0xFF, 0xFF]
        );

        let mut out = vec![0xAAu8];
        extend_be(&mut out, &[256i32], i32::to_be_bytes);
        assert_eq!(out, vec![0xAA, 0, 0, 1, 0]);
    }
}
