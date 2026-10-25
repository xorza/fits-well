//! Big-endian scalar (de)serialization shared by the image, table, and
//! compression layers. FITS data is always big-endian, so every typed decode or
//! encode funnels through these helpers — one per output shape (a fresh `Vec`, a
//! reused `Vec`, an existing slice, appended bytes), all built on the same
//! monomorphized per-element conversion.

use crate::allocation;
use crate::fits_error::Result;

/// Decode a *sequence* of big-endian buffers, in order, into one `Vec`. An
/// image or heap array is a single contiguous run, but a binary-table column read
/// visits one strided cell per row, so the elements it decodes are not adjacent.
/// `capacity` is the elements the cells hold in all, reserved once and fallibly: heap
/// arrays may overlap (§7.3.5), so their total is no bound the file's size sets.
pub(crate) fn decode_be_cells<'a, const N: usize, T, F>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    conv: F,
) -> Result<Vec<T>>
where
    F: Fn([u8; N]) -> T,
{
    let mut out = Vec::new();
    decode_be_cells_into(cells, capacity, conv, &mut out)?;
    Ok(out)
}

/// [`decode_be_cells`] into `out`, cleared first, so a caller reuses its buffer.
pub(crate) fn decode_be_cells_into<'a, const N: usize, T, F>(
    cells: impl Iterator<Item = &'a [u8]>,
    capacity: usize,
    conv: F,
    out: &mut Vec<T>,
) -> Result<()>
where
    F: Fn([u8; N]) -> T,
{
    out.clear();
    allocation::try_reserve_exact(out, capacity)?;
    extend_be_cells(cells, conv, out);
    Ok(())
}

/// Append every `N`-byte value of `cells` to `out`, which the caller sized.
fn extend_be_cells<'a, const N: usize, T, F>(
    cells: impl Iterator<Item = &'a [u8]>,
    conv: F,
    out: &mut Vec<T>,
) where
    F: Fn([u8; N]) -> T,
{
    for cell in cells {
        debug_assert_eq!(cell.len() % N, 0, "whole big-endian elements");
        out.extend(cell.as_chunks::<N>().0.iter().map(|c| conv(*c)));
    }
}

/// [`decode_be_cells_into`] of the one run `bytes`: `out`, cleared first, holds every
/// `N`-byte value of it.
///
/// Gated because the tiled codecs are its only callers — they widen each tile into a
/// scratch buffer held across tiles.
#[cfg(feature = "compression")]
pub(crate) fn decode_be_into<const N: usize, T, F>(bytes: &[u8], out: &mut Vec<T>, conv: F)
where
    F: Fn([u8; N]) -> T,
{
    out.clear();
    out.reserve(bytes.len() / N);
    extend_be_cells(std::iter::once(bytes), conv, out);
}

/// Decode a big-endian buffer into the host-endian slice `dst` (one element per
/// `N`-byte chunk; `dst.len()` must be `bytes.len() / N`) — what the reader's view path
/// decodes into a reused, `u64`-aligned scratch reinterpreted as `&mut [T]`, so a hot
/// read loop reuses the (already-faulted) output pages instead of allocating per image.
/// `conv` is a generic `Fn`, not a `fn` pointer: each call site passes a zero-sized fn
/// item, so the conversion inlines and the fixed-stride loop vectorizes.
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
/// dominates and won't vectorize). With `conv` inlined (see
/// [`decode_be_into_slice`]) the fixed-stride write loop vectorizes like the decode path.
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

#[cfg(test)]
mod tests {
    use std::iter;

    use crate::endian::*;

    #[test]
    fn decode_and_encode_are_inverse_and_big_endian() {
        assert_eq!(
            decode_be_cells(
                iter::once(&[0x00, 0x01, 0xFF, 0xFF][..]),
                2,
                i16::from_be_bytes
            )
            .unwrap(),
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
        // Nothing to decode empties the buffer rather than leaving it untouched.
        decode_be_into(&[], &mut reused, widen);
        assert_eq!(reused, Vec::<i64>::new());
    }
}
