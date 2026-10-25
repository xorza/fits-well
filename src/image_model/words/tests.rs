use crate::image_model::words::*;

#[test]
fn samples_view_the_same_bytes_the_mutable_view_wrote() {
    // Two `u64`s = 16 bytes = 4 `i32`s or 2 `f64`s. Write through the mutable
    // view, read back through the shared one, and check the exact values.
    let mut words = vec![0u64; 2];
    let ints: &mut [i32] = samples_mut(&mut words, 4);
    ints.copy_from_slice(&[1, -1, i32::MAX, i32::MIN]);
    assert_eq!(samples::<i32>(&words, 4), [1, -1, i32::MAX, i32::MIN]);

    // A shorter view of the same storage sees only its own prefix.
    assert_eq!(samples::<i32>(&words, 2), [1, -1]);

    // Reinterpreting as a wider type reads the same bytes: the first `u64` holds
    // the host-endian pair (1, -1), so as `f64` it is exactly that bit pattern.
    // Compared as bits, not values — this particular pattern is a NaN, and no
    // NaN compares equal to itself.
    let wide = samples::<f64>(&words, 1);
    assert_eq!(wide[0].to_bits(), words[0]);
}

#[test]
#[should_panic(expected = "3 samples do not fit 1 words")]
fn view_past_the_words_panics() {
    let _ = samples::<i32>(&[0], 3);
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
