//! Big-endian scalar (de)serialization shared by the image, table, and
//! compression layers. FITS data is always big-endian, so every typed decode or
//! encode funnels through these three helpers.

/// Decode a packed big-endian buffer into host-endian values of a fixed-width
/// type, e.g. `decode_be(bytes, i16::from_be_bytes)`.
///
/// `conv` is a generic `Fn`, not a `fn` pointer: each call site passes a zero-sized
/// fn *item* (`i32::from_be_bytes`, …), so the per-element conversion monomorphizes
/// to a direct, inlinable call and the loop autovectorizes — a `fn`-pointer
/// parameter would force an indirect call per element and block both.
pub(crate) fn decode_be<const N: usize, T, F>(bytes: &[u8], conv: F) -> Vec<T>
where
    F: Fn([u8; N]) -> T,
{
    let mut out = Vec::new();
    decode_be_into(bytes, conv, &mut out);
    out
}

/// Decode a big-endian buffer into `out`, *reusing* its allocation — clear it, then
/// refill from `bytes`. The buffer-reusing counterpart to [`decode_be`], mirroring
/// [`extend_be`] on the encode side.
///
/// Profiling showed a fresh-`Vec`-per-call decode is dominated not by the byte-swap
/// but by the kernel faulting in the new output pages (`clear_page` + the
/// page-fault machinery, ~65% of the time) — so a caller decoding many same-typed
/// images into one `out` pays that fault traffic once, not per call (~4× on the
/// repeated path). After `clear()`, `extend` over the `ExactSizeIterator` reserves
/// once and reuses the capacity on every later call.
pub(crate) fn decode_be_into<const N: usize, T, F>(bytes: &[u8], conv: F, out: &mut Vec<T>)
where
    F: Fn([u8; N]) -> T,
{
    out.clear();
    out.extend(
        bytes
            .chunks_exact(N)
            .map(|c| conv(c.try_into().expect("chunks_exact yields N-byte arrays"))),
    );
}

/// Encode fixed-width values into a *fresh* big-endian byte buffer, e.g.
/// `encode_be(values, i16::to_be_bytes)`. `conv` is a generic `Fn` for the same
/// inlining/vectorization reason as [`decode_be`].
///
/// Only the compression codecs need the owning form (they build many small,
/// independent per-tile buffers); the image and table writers append in place via
/// [`extend_be`] into a reused buffer, so this is gated to where it is used.
#[cfg(feature = "compression")]
pub(crate) fn encode_be<const N: usize, T: Copy, F>(values: &[T], conv: F) -> Vec<u8>
where
    F: Fn(T) -> [u8; N],
{
    let mut out = Vec::new();
    extend_be(&mut out, values, conv);
    out
}

/// Append fixed-width values to `out` in big-endian order.
///
/// Grows `out` once and writes each element into its `N`-byte slot, rather than a
/// per-element `extend_from_slice` (a capacity check + memcpy per element that
/// dominates and won't vectorize). With `conv` inlined (see [`decode_be`]) the
/// fixed-stride write loop vectorizes like the decode path.
pub(crate) fn extend_be<const N: usize, T: Copy, F>(out: &mut Vec<u8>, values: &[T], conv: F)
where
    F: Fn(T) -> [u8; N],
{
    let start = out.len();
    out.resize(start + values.len() * N, 0);
    for (slot, &v) in out[start..].chunks_exact_mut(N).zip(values) {
        slot.copy_from_slice(&conv(v));
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
        // Encode direction via the always-compiled in-place primitive (the path the
        // image/table writers use); `encode_be` is the same write into a fresh Vec.
        let mut enc = Vec::new();
        extend_be(&mut enc, &[1i16, -1], i16::to_be_bytes);
        assert_eq!(enc, vec![0, 1, 0xFF, 0xFF]);

        // Appending starts at the buffer's current end, leaving prior bytes intact.
        let mut out = vec![0xAAu8];
        extend_be(&mut out, &[256i32], i32::to_be_bytes);
        assert_eq!(out, vec![0xAA, 0, 0, 1, 0]);
    }
}
