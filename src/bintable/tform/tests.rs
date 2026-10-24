use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tform::Storage;
use crate::bintable::tform::Tform;
use crate::bintable::tform::internals::tform;
use crate::error::FitsError;

#[test]
fn parses_tform_repeat_and_kind() {
    let cases = [
        ("8A", tform(8, ElementKind::Character, Storage::Fixed)),
        ("3D", tform(3, ElementKind::F64, Storage::Fixed)),
        ("0D", tform(0, ElementKind::F64, Storage::Fixed)),
        ("1J", tform(1, ElementKind::I32, Storage::Fixed)),
        ("E", tform(1, ElementKind::F32, Storage::Fixed)), // bare code ⇒ repeat 1
        ("16X", tform(16, ElementKind::Bit, Storage::Fixed)),
        // P/Q carry the heap element type.
        (
            "1PE(5)",
            tform(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1QD",
            tform(1, ElementKind::F64, Storage::Array(DescriptorWidth::Q)),
        ),
        // The characters after a code, or after a P/Q's element type, are undefined
        // and ignored (§7.3.1, §7.3.5).
        ("60A10", tform(60, ElementKind::Character, Storage::Fixed)),
        ("1Jjunk", tform(1, ElementKind::I32, Storage::Fixed)),
        (
            "1PE(5)junk",
            tform(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1PE(5",
            tform(1, ElementKind::F32, Storage::Array(DescriptorWidth::P)),
        ),
        (
            "1QB()",
            tform(1, ElementKind::U8, Storage::Array(DescriptorWidth::Q)),
        ),
    ];
    for (s, expected) in cases {
        assert_eq!(s.parse::<Tform>().unwrap(), expected, "{s}");
    }
    for bad in ["9Z", "", "1P", "2PE(5)", "3QD", "1PP", "1PQ", "1QP", "1QQ"] {
        // "1P" lacks the heap element-type letter; "2PE"/"3QD" violate the §6.3
        // rule that a P/Q descriptor's repeat count is 0 or 1.
        assert!(
            matches!(bad.parse::<Tform>(), Err(FitsError::InvalidTform { .. })),
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
