//! Big-endian scalar (de)serialization shared by the image, table, and
//! compression layers. FITS data is always big-endian, so every typed decode or
//! encode funnels through these helpers — one per output shape (a fresh `Vec`, a
//! reused `Vec`, an existing slice, appended bytes), all built on the same
//! monomorphized per-element conversion.

use crate::error::FitsError;
use crate::error::Result;
use std::iter;

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
    decode_be_cells(iter::once(bytes), bytes.len() / N, conv)
}

/// [`decode_be`] over a *sequence* of buffers, decoded in order into one `Vec`. An
/// image or heap array is a single contiguous run, but a binary-table column read
/// visits one strided cell per row, so the elements it decodes are not adjacent.
/// `capacity` is a `Vec::with_capacity` hint only.
pub(crate) fn decode_be_cells<'a, const N: usize, T, F>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    conv: F,
) -> Vec<T>
where
    F: Fn([u8; N]) -> T,
{
    let mut out = Vec::with_capacity(capacity);
    for cell in cells {
        debug_assert_eq!(cell.len() % N, 0, "whole big-endian elements");
        out.extend(cell.as_chunks::<N>().0.iter().map(|c| conv(*c)));
    }
    out
}

/// [`decode_be`] into a reused buffer rather than a fresh `Vec`: clears `out`, then
/// decodes every `N`-byte chunk of `bytes` into it. The per-element loop is the same
/// one [`decode_be`] uses and vectorizes identically.
///
/// Gated because the tiled codecs are its only callers — they widen each tile into a
/// scratch buffer held across tiles, so they need the reused-buffer shape rather than
/// [`decode_be`]'s fresh allocation.
#[cfg(feature = "compression")]
pub(crate) fn decode_be_into<const N: usize, T, F>(bytes: &[u8], out: &mut Vec<T>, conv: F)
where
    F: Fn([u8; N]) -> T,
{
    out.clear();
    // The final length is exactly one element per chunk, so ask for that and no more.
    out.reserve_exact(bytes.len() / N);
    out.extend(bytes.as_chunks::<N>().0.iter().map(|c| conv(*c)));
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
    for (d, c) in dst.iter_mut().zip(bytes.as_chunks::<N>().0) {
        *d = conv(*c);
    }
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
    for (slot, &v) in out[start..].as_chunks_mut::<N>().0.iter_mut().zip(values) {
        *slot = conv(v);
    }
}

/// Validate the signed integer range of a FITS `P` or `Q` descriptor.
#[expect(
    clippy::map_err_ignore,
    reason = "a `TryFromIntError` says only that the value does not fit, which the error it becomes states"
)]
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
        // Encode direction via the in-place primitive the image/table writers use.
        let mut enc = Vec::new();
        extend_be(&mut enc, &[1i16, -1], i16::to_be_bytes);
        assert_eq!(enc, vec![0, 1, 0xFF, 0xFF]);

        // Appending starts at the buffer's current end, leaving prior bytes intact.
        let mut out = vec![0xAAu8];
        extend_be(&mut out, &[256i32], i32::to_be_bytes);
        assert_eq!(out, vec![0xAA, 0, 0, 1, 0]);
    }

    /// Gated with the primitive itself: `decode_be_into` exists only for the tiled
    /// codecs, so a build without `compression` has nothing to exercise.
    #[cfg(feature = "compression")]
    #[test]
    fn decode_be_into_replaces_the_reused_buffer_contents() {
        // Unlike `extend_be`, this *replaces* rather than appends — a long fill
        // followed by a short one must leave no stale tail. It also widens through
        // `conv`, which is how the codecs decode straight into their `i64` scratch.
        let widen = |b| i64::from(i16::from_be_bytes(b));
        let mut reused = vec![99i64; 8];
        decode_be_into(&[0x00, 0x01, 0xFF, 0xFF], &mut reused, widen);
        assert_eq!(reused, vec![1i64, -1]);
        decode_be_into(&[0x00, 0x02], &mut reused, widen);
        assert_eq!(reused, vec![2i64]);
        // A trailing partial element is not an element: `chunks_exact` drops it.
        decode_be_into(&[0x00, 0x03, 0x7F], &mut reused, widen);
        assert_eq!(reused, vec![3i64]);
        // Nothing to decode empties the buffer rather than leaving it untouched.
        decode_be_into(&[], &mut reused, widen);
        assert_eq!(reused, Vec::<i64>::new());
    }

    /// §7.3.5: `P` is two big-endian i32s and `Q` two i64s, count then offset; a
    /// value past the form's signed range is refused, not truncated.
    #[test]
    fn pq_descriptors_are_big_endian_count_then_offset() {
        let p_max = i32::MAX as u64;
        let q_max = i64::MAX as u64;
        for (wide, count, offset, expected) in [
            (false, 7, 40, Some(&[0, 0, 0, 7, 0, 0, 0, 40][..])),
            (
                false,
                p_max,
                p_max,
                Some(&[0x7F, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF][..]),
            ),
            (false, p_max + 1, 0, None),
            (false, 0, p_max + 1, None),
            // A count past u32 and an offset past 2^33: no 32-bit truncation.
            (
                true,
                0x1_0000_0004,
                0x3_0000_0002,
                Some(&[0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 3, 0, 0, 0, 2][..]),
            ),
            (true, q_max + 1, 0, None),
            (true, 0, q_max + 1, None),
        ] {
            let mut slot = vec![0xA5; if wide { 16 } else { 8 }];
            let written = write_pq_descriptor(&mut slot, wide, count, offset);
            match expected {
                Some(bytes) => {
                    written.unwrap();
                    assert_eq!(slot, bytes, "{wide} {count} {offset}");
                }
                None => assert!(
                    matches!(written, Err(FitsError::DataUnitOverflow)),
                    "{wide} {count} {offset}"
                ),
            }
        }
    }
}
