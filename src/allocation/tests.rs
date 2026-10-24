use crate::allocation::*;

#[test]
fn oversized_allocations_return_the_requested_byte_count() {
    let mut values = Vec::<u64>::new();
    let bytes = (usize::MAX as u64).saturating_mul(8);
    assert!(matches!(
        try_reserve(&mut values, usize::MAX),
        Err(FitsError::DataUnitTooLarge { bytes: got }) if got == bytes
    ));
    assert!(matches!(
        try_resize(&mut values, usize::MAX, 0),
        Err(FitsError::DataUnitTooLarge { bytes: got }) if got == bytes
    ));
}

#[test]
fn resize_reuses_capacity_and_preserves_exact_length() {
    let mut values = Vec::with_capacity(8);
    let ptr = values.as_ptr();
    try_resize(&mut values, 8, 3u8).unwrap();
    assert_eq!(values, [3; 8]);
    try_resize(&mut values, 2, 9).unwrap();
    assert_eq!(values, [3, 3]);
    assert_eq!(values.as_ptr(), ptr);
}
