use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tform::Storage;
use crate::bintable::tform::Tform;
use crate::fits_error::FitsError;
use std::assert_matches;

#[test]
fn parses_tform_repeat_and_kind() {
    let cases = [
        ("8A", Tform::new(8, ElementKind::Character, Storage::Fixed)),
        ("3D", Tform::new(3, ElementKind::F64, Storage::Fixed)),
        ("0D", Tform::new(0, ElementKind::F64, Storage::Fixed)),
        ("1J", Tform::new(1, ElementKind::I32, Storage::Fixed)),
        ("E", Tform::new(1, ElementKind::F32, Storage::Fixed)), // bare code ⇒ repeat 1
        ("16X", Tform::new(16, ElementKind::Bit, Storage::Fixed)),
        // P/Q carry the heap element type.
        (
            "1PE(5)",
            Tform::new(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1QD",
            Tform::new(1, ElementKind::F64, Storage::Array(DescriptorWidth::Q)),
        ),
        // The characters after a code, or after a P/Q's element type, are undefined
        // and ignored (§7.3.1, §7.3.5).
        (
            "60A10",
            Tform::new(60, ElementKind::Character, Storage::Fixed),
        ),
        ("1Jjunk", Tform::new(1, ElementKind::I32, Storage::Fixed)),
        (
            "1PE(5)junk",
            Tform::new(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1PE(5",
            Tform::new(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1QB()",
            Tform::new(1, ElementKind::U8, Storage::Array(DescriptorWidth::Q)),
        ),
    ];
    for (s, expected) in cases {
        assert_eq!(s.parse::<Tform>().unwrap(), expected, "{s}");
        assert_eq!(
            expected.to_string().parse::<Tform>().unwrap(),
            expected,
            "{s}"
        );
    }
    // The repeat count is always written, and the ignored characters never are.
    let printed = cases.map(|(_, tform)| tform.to_string());
    assert_eq!(
        printed,
        [
            "8A", "3D", "0D", "1J", "1E", "16X", "1PE", "1QD", "60A", "1J", "1PE", "1PE", "1QB"
        ]
    );
    for bad in ["9Z", "", "1P", "2PE(5)", "3QD", "1PP", "1PQ", "1QP", "1QQ"] {
        // "1P" lacks the heap element-type letter; "2PE"/"3QD" violate the §6.3
        // rule that a P/Q descriptor's repeat count is 0 or 1.
        assert_matches!(
            bad.parse::<Tform>(),
            Err(FitsError::InvalidTform { tform }) if tform == bad,
            "{bad}"
        );
    }
}

#[test]
fn byte_width_handles_arrays_bits_and_descriptors() {
    assert_eq!("8A".parse::<Tform>().unwrap().byte_width(), 8);
    assert_eq!("3D".parse::<Tform>().unwrap().byte_width(), 24);
    assert_eq!("0D".parse::<Tform>().unwrap().byte_width(), 0);
    assert_eq!("16X".parse::<Tform>().unwrap().byte_width(), 2); // 16 bits = 2 bytes
    assert_eq!("9X".parse::<Tform>().unwrap().byte_width(), 2); //  9 bits = 2 bytes
    assert_eq!("1PB".parse::<Tform>().unwrap().byte_width(), 8); // 32-bit descriptor
    assert_eq!("1QB".parse::<Tform>().unwrap().byte_width(), 16); // 64-bit descriptor
}
