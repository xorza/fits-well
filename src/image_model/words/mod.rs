//! Reinterpreting a `u64`-backed scratch buffer as typed FITS samples.
//!
//! The reader's view path and the compressed-image decoder both write host-endian
//! samples into a caller-owned `Vec<u64>` and then hand back typed slices over it.
//! The `u64` backing is what makes that sound: it is 8-byte aligned, which
//! satisfies every FITS scalar type, its words are always initialized, and it is
//! reused across reads so a hot loop pays the output allocation once. Both
//! directions of that reinterpretation live here, safe, with the one check that
//! keeps them sound — the length, as `bytemuck::cast_slice` checks it.

use std::slice;

use crate::image_model::sample::Sample;

/// View the front of `words` as `count` host-endian samples.
///
/// # Panics
/// When `count` samples need more bytes than `words` holds: the one check that keeps
/// the view in bounds, so it runs in a release build too.
pub(crate) fn samples<T: Sample>(words: &[u64], count: usize) -> &[T] {
    assert!(
        fits(words.len(), count, size_of::<T>()),
        "{count} samples do not fit {} words",
        words.len()
    );
    // SAFETY: the assert keeps the view inside `words`, whose `u64`s are initialized
    // and 8-byte aligned, and every bit pattern is a valid `T`, as `Sample`'s sealed
    // `Raw` promises.
    unsafe { slice::from_raw_parts(words.as_ptr().cast::<T>(), count) }
}

/// The [`samples`] view as a mutable slice, for a decode that writes through it.
///
/// # Panics
/// As [`samples`].
pub(crate) fn samples_mut<T: Sample>(words: &mut [u64], count: usize) -> &mut [T] {
    assert!(
        fits(words.len(), count, size_of::<T>()),
        "{count} samples do not fit {} words",
        words.len()
    );
    // SAFETY: as in `samples`; the borrow of `words` is exclusive.
    unsafe { slice::from_raw_parts_mut(words.as_mut_ptr().cast::<T>(), count) }
}

/// The bytes of `values`, which a read fills with stored big-endian samples to swap in
/// place.
pub(crate) const fn bytes_mut<T: Sample>(values: &mut [T]) -> &mut [u8] {
    let len = size_of_val(values);
    // SAFETY: the bytes are those of `values`, borrowed exclusively; a `u8` has no
    // alignment and no invalid value, and every byte pattern is a valid `T`, as
    // `Sample`'s sealed `Raw` promises, so any byte written leaves `values` valid.
    unsafe { slice::from_raw_parts_mut(values.as_mut_ptr().cast::<u8>(), len) }
}

/// Whether `count` elements of `elem_size` bytes fit `words` `u64`s.
const fn fits(words: usize, count: usize, elem_size: usize) -> bool {
    count.saturating_mul(elem_size) <= words.saturating_mul(8)
}

#[cfg(test)]
mod tests;
