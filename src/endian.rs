//! Big-endian scalar (de)serialization shared by the image, table, and
//! compression layers. FITS data is always big-endian, so every typed decode or
//! encode funnels through these three helpers.

use crate::error::FitsError;
use crate::error::Result;

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
    bytes
        .chunks_exact(N)
        .map(|c| conv(c.try_into().expect("chunks_exact yields N-byte arrays")))
        .collect()
}

/// Decode a big-endian buffer into the host-endian slice `dst` (one element per
/// `N`-byte chunk; `dst.len()` must be `bytes.len() / N`). The slice-writing
/// counterpart to [`decode_be`] — used by the reader's view path, which decodes into
/// a reused, `u64`-aligned scratch reinterpreted as `&mut [T]` so a hot read loop
/// reuses the (already-faulted) output pages instead of allocating per image.
/// `conv` is inlined per the [`decode_be`] note, so the fixed-stride loop vectorizes.
pub(crate) fn decode_be_into_slice<const N: usize, T, F>(bytes: &[u8], dst: &mut [T], conv: F)
where
    F: Fn([u8; N]) -> T,
{
    debug_assert_eq!(
        dst.len(),
        bytes.len() / N,
        "dst must hold one element per chunk"
    );
    for (d, c) in dst.iter_mut().zip(bytes.chunks_exact(N)) {
        *d = conv(c.try_into().expect("chunks_exact yields N-byte arrays"));
    }
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

/// Validate the signed integer range of a FITS `P` or `Q` descriptor.
pub(crate) fn validate_pq_descriptor(wide: bool, count: u64, offset: u64) -> Result<()> {
    if wide {
        i64::try_from(count).map_err(|_| FitsError::DataUnitOverflow)?;
        i64::try_from(offset).map_err(|_| FitsError::DataUnitOverflow)?;
    } else {
        i32::try_from(count).map_err(|_| FitsError::DataUnitOverflow)?;
        i32::try_from(offset).map_err(|_| FitsError::DataUnitOverflow)?;
    }
    Ok(())
}

/// Write a validated big-endian `P` or `Q` descriptor into an existing row slot.
pub(crate) fn write_pq_descriptor(
    out: &mut [u8],
    wide: bool,
    count: u64,
    offset: u64,
) -> Result<()> {
    validate_pq_descriptor(wide, count, offset)?;
    let expected = if wide { 16 } else { 8 };
    assert_eq!(out.len(), expected, "descriptor slot width");
    if wide {
        out[..8].copy_from_slice(&i64::try_from(count).unwrap().to_be_bytes());
        out[8..].copy_from_slice(&i64::try_from(offset).unwrap().to_be_bytes());
    } else {
        out[..4].copy_from_slice(&i32::try_from(count).unwrap().to_be_bytes());
        out[4..].copy_from_slice(&i32::try_from(offset).unwrap().to_be_bytes());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::endian::*;

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

        let mut descriptor = [0u8; 16];
        write_pq_descriptor(&mut descriptor, true, 3, u32::MAX as u64 + 8).unwrap();
        assert_eq!(i64::from_be_bytes(descriptor[..8].try_into().unwrap()), 3);
        assert_eq!(
            i64::from_be_bytes(descriptor[8..].try_into().unwrap()),
            u32::MAX as i64 + 8
        );
    }
}
