//! Reinterpreting a `u64`-backed scratch buffer as typed FITS samples.
//!
//! The reader's view path and the compressed-image decoder both write host-endian
//! samples into a caller-owned `Vec<u64>` and then hand back typed slices over it.
//! The `u64` backing is what makes that sound: it is 8-byte aligned, which
//! satisfies every FITS scalar type, and it is reused across reads so a hot loop
//! pays the output allocation once. Both directions of that reinterpretation live
//! here, once, so the `unsafe` argument is made in a single place rather than at
//! each of the three call sites that need it.

/// A FITS array element type that may be viewed over a `u64`-backed buffer.
///
/// # Safety
///
/// Implementors must be plain-data scalars with no invalid bit patterns and an
/// alignment of at most 8. Both properties are what let [`samples`] and
/// [`samples_mut`] reinterpret `u64` storage as `T` without checking anything at
/// run time. The set is closed: exactly the six types a [`Bitpix`] selects.
///
/// [`Bitpix`]: crate::bitpix::Bitpix
pub(crate) unsafe trait Sample: Copy {}

macro_rules! impl_sample {
    ($($type:ty),+ $(,)?) => {
        // SAFETY: every listed type is a `Copy` integer or IEEE-754 float — plain
        // data, every bit pattern valid, alignment at most 8 (`i64`/`f64`).
        $(unsafe impl Sample for $type {})+
    };
}

impl_sample!(u8, i16, i32, i64, f32, f64);

/// View the front of `words` as `count` host-endian samples.
///
/// # Safety
///
/// The first `count * size_of::<T>()` bytes of `words` must hold initialized
/// values. Alignment and bit-pattern validity are *not* the caller's obligation:
/// the `u64` backing supplies the first and `T: Sample` the second.
pub(crate) unsafe fn samples<T: Sample>(words: &[u64], count: usize) -> &[T] {
    debug_assert!(
        fits(words.len(), count, size_of::<T>()),
        "sample count fits"
    );
    // SAFETY: `u64` storage is 8-aligned, which satisfies every `Sample`; the caller
    // guarantees the `count` elements are initialized; `Sample` has no invalid bit
    // patterns, so every one of them is a valid `T`.
    unsafe { std::slice::from_raw_parts(words.as_ptr() as *const T, count) }
}

/// The [`samples`] view as a mutable slice, for a decode that writes through it.
///
/// # Safety
///
/// As [`samples`]. A zeroed buffer satisfies the initialization requirement for
/// every `Sample`, so a caller that resized the storage with `0` may write through
/// the result without reading it first.
pub(crate) unsafe fn samples_mut<T: Sample>(words: &mut [u64], count: usize) -> &mut [T] {
    debug_assert!(
        fits(words.len(), count, size_of::<T>()),
        "sample count fits"
    );
    // SAFETY: as `samples`, plus the buffer is uniquely borrowed for the returned
    // slice's lifetime, so no other view of these bytes can exist.
    unsafe { std::slice::from_raw_parts_mut(words.as_mut_ptr() as *mut T, count) }
}

/// Whether `count` elements of `elem_size` bytes fit `words` `u64`s.
fn fits(words: usize, count: usize, elem_size: usize) -> bool {
    count.saturating_mul(elem_size) <= words.saturating_mul(8)
}

#[cfg(test)]
mod tests {
    use crate::words::*;

    #[test]
    fn samples_view_the_same_bytes_the_mutable_view_wrote() {
        // Two `u64`s = 16 bytes = 4 `i32`s or 2 `f64`s. Write through the mutable
        // view, read back through the shared one, and check the exact values.
        let mut words = vec![0u64; 2];
        // SAFETY: 4 * 4 = 16 bytes, exactly the buffer; zeroed, so initialized.
        let ints: &mut [i32] = unsafe { samples_mut(&mut words, 4) };
        ints.copy_from_slice(&[1, -1, i32::MAX, i32::MIN]);
        // SAFETY: the same 16 bytes, written above.
        assert_eq!(
            unsafe { samples::<i32>(&words, 4) },
            [1, -1, i32::MAX, i32::MIN]
        );

        // A shorter view of the same storage sees only its own prefix.
        // SAFETY: 2 * 4 = 8 bytes, within the buffer.
        assert_eq!(unsafe { samples::<i32>(&words, 2) }, [1, -1]);

        // Reinterpreting as a wider type reads the same bytes: the first `u64` holds
        // the host-endian pair (1, -1), so as `f64` it is exactly that bit pattern.
        // Compared as bits, not values — this particular pattern is a NaN, and no
        // NaN compares equal to itself.
        // SAFETY: 1 * 8 = 8 bytes, within the buffer.
        let wide = unsafe { samples::<f64>(&words, 1) };
        assert_eq!(wide[0].to_bits(), words[0]);
    }

    #[test]
    fn capacity_check_counts_bytes_not_elements() {
        // 3 u64s = 24 bytes: room for 24 u8, 12 i16, 6 i32, or 3 f64 — and no more.
        for (elem_size, fitting) in [(1, 24), (2, 12), (4, 6), (8, 3)] {
            assert!(fits(3, fitting, elem_size), "{fitting} × {elem_size}");
            assert!(
                !fits(3, fitting + 1, elem_size),
                "{} × {elem_size}",
                fitting + 1
            );
        }
        // An absurd count saturates rather than wrapping to a passing product.
        assert!(!fits(1, usize::MAX, 8));
    }
}
