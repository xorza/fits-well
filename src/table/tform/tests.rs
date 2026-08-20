use crate::error::FitsError;
use crate::table_impl::tform::Tform;
use crate::table_impl::tform::internals::tform;
use crate::table_impl::tform_kind::TformKind;

#[test]
fn parses_tform_repeat_and_kind() {
    let cases = [
        ("8A", tform(8, TformKind::Char, None)),
        ("3D", tform(3, TformKind::F64, None)),
        ("0D", tform(0, TformKind::F64, None)),
        ("1J", tform(1, TformKind::I32, None)),
        ("E", tform(1, TformKind::F32, None)), // bare code ⇒ repeat 1
        ("16X", tform(16, TformKind::Bit, None)),
        // P/Q carry the heap element type.
        (
            "1PE(5)",
            tform(1, TformKind::ArrayDesc32, Some(TformKind::F32)),
        ),
        (
            "1QD",
            tform(1, TformKind::ArrayDesc64, Some(TformKind::F64)),
        ),
    ];
    for (s, expected) in cases {
        assert_eq!(Tform::parse(s).unwrap(), expected, "{s}");
    }
    for bad in [
        "9Z",
        "",
        "1P",
        "2PE(5)",
        "3QD",
        "1PP",
        "1PQ",
        "1QP",
        "1QQ",
        "1Jjunk",
        "1PE(5)junk",
        "1PE(5",
        "1PE()",
    ] {
        // "1P" lacks the heap element-type letter; "2PE"/"3QD" violate the §6.3
        // rule that a P/Q descriptor's repeat count is 0 or 1.
        assert!(
            matches!(Tform::parse(bad), Err(FitsError::InvalidTform { .. })),
            "{bad}"
        );
    }
}

#[test]
fn byte_width_handles_arrays_bits_and_descriptors() {
    assert_eq!(Tform::parse("8A").unwrap().byte_width(), 8);
    assert_eq!(Tform::parse("3D").unwrap().byte_width(), 24);
    assert_eq!(Tform::parse("0D").unwrap().byte_width(), 0);
    assert_eq!(Tform::parse("16X").unwrap().byte_width(), 2); // 16 bits = 2 bytes
    assert_eq!(Tform::parse("9X").unwrap().byte_width(), 2); //  9 bits = 2 bytes
    assert_eq!(Tform::parse("1PB").unwrap().byte_width(), 8); // 32-bit descriptor
    assert_eq!(Tform::parse("1QB").unwrap().byte_width(), 16); // 64-bit descriptor
}
