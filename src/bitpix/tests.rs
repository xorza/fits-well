use crate::bitpix::*;

#[test]
fn code_round_trips_for_every_variant() {
    for bp in [
        Bitpix::U8,
        Bitpix::I16,
        Bitpix::I32,
        Bitpix::I64,
        Bitpix::F32,
        Bitpix::F64,
    ] {
        assert_eq!(Bitpix::from_code(bp.code()).unwrap(), bp);
    }
}

#[test]
fn codes_and_sizes_match_the_standard() {
    // (BITPIX code, byte size, is_float)
    let cases = [
        (Bitpix::U8, 8, 1, false),
        (Bitpix::I16, 16, 2, false),
        (Bitpix::I32, 32, 4, false),
        (Bitpix::I64, 64, 8, false),
        (Bitpix::F32, -32, 4, true),
        (Bitpix::F64, -64, 8, true),
    ];
    for (bp, code, size, is_float) in cases {
        assert_eq!(bp.code(), code);
        assert_eq!(bp.elem_size(), size);
        assert_eq!(bp.is_float(), is_float);
        assert_eq!(bp.is_integer(), !is_float);
    }
}

#[test]
fn rejects_codes_outside_the_allowed_set() {
    for bad in [0, 7, 1, -1, 24, 128, -16] {
        assert!(matches!(
            Bitpix::from_code(bad),
            Err(FitsError::InvalidBitpix { code }) if code == bad
        ));
    }
}
