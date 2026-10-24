use crate::bintable::descriptor;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::element_kind::ElementKind;
use crate::error::{FitsError, SizeUnit};

#[test]
fn decodes_signed_boundaries_and_rejects_malformed_widths() {
    let mut narrow = Vec::new();
    narrow.extend_from_slice(&i32::MAX.to_be_bytes());
    narrow.extend_from_slice(&i32::MAX.to_be_bytes());
    assert_eq!(
        PqDescriptor::decode(&narrow, DescriptorWidth::P).unwrap(),
        PqDescriptor {
            count: i32::MAX as usize,
            offset: i32::MAX as usize,
        }
    );

    let mut wide = Vec::new();
    wide.extend_from_slice(&i64::MAX.to_be_bytes());
    wide.extend_from_slice(&i64::MAX.to_be_bytes());
    let decoded = PqDescriptor::decode(&wide, DescriptorWidth::Q);
    if usize::BITS >= 64 {
        assert_eq!(
            decoded.unwrap(),
            PqDescriptor {
                count: i64::MAX as usize,
                offset: i64::MAX as usize,
            }
        );
    } else {
        assert!(matches!(decoded, Err(FitsError::DataUnitOverflow)));
    }
    assert_eq!(
        PqDescriptor::decode(&[0; 8], DescriptorWidth::P).unwrap(),
        PqDescriptor::EMPTY
    );
    assert_eq!(
        PqDescriptor::decode(&[0; 16], DescriptorWidth::Q).unwrap(),
        PqDescriptor::EMPTY
    );
    // An array of no elements has an undefined offset (§7.3.5): a negative one reads as
    // the empty array.
    let mut empty = Vec::new();
    empty.extend_from_slice(&0i32.to_be_bytes());
    empty.extend_from_slice(&(-5i32).to_be_bytes());
    assert_eq!(
        PqDescriptor::decode(&empty, DescriptorWidth::P).unwrap(),
        PqDescriptor::EMPTY
    );

    for case in descriptor::internals::malformed_descriptor_cases() {
        case.assert_error(&PqDescriptor::decode(&case.bytes, case.width).unwrap_err());
    }
    for (bytes, width, expected) in [
        (&[0u8; 7][..], DescriptorWidth::P, 8),
        (&[0u8; 15], DescriptorWidth::Q, 16),
    ] {
        assert!(matches!(
            PqDescriptor::decode(bytes, width),
            Err(FitsError::DataSizeMismatch { unit: SizeUnit::Bytes, expected: got, got: len })
                if got == expected && len == bytes.len() as u64
        ));
    }
}

#[test]
fn payload_lengths_and_heap_ranges_cover_boundaries() {
    for (count, expected) in [(0, 0), (1, 1), (8, 1), (9, 2)] {
        assert_eq!(ElementKind::Bit.payload_len(count), Some(expected));
    }
    assert_eq!(
        PqDescriptor {
            count: 3,
            offset: 5,
        }
        .heap_range(ElementKind::I16, 10, 21)
        .unwrap(),
        15..21
    );
    assert_eq!(
        PqDescriptor {
            count: 0,
            offset: usize::MAX,
        }
        .heap_range(ElementKind::U8, 10, 10)
        .unwrap(),
        10..10
    );

    let byte_alias = PqDescriptor {
        count: 2,
        offset: 2,
    };
    let integer_alias = PqDescriptor {
        count: 1,
        offset: 2,
    };
    assert_eq!(
        byte_alias.heap_range(ElementKind::U8, 8, 12).unwrap(),
        integer_alias.heap_range(ElementKind::I16, 8, 12).unwrap()
    );
    assert_eq!(
        PqDescriptor {
            count: 1,
            offset: usize::MAX - 1,
        }
        .heap_range(ElementKind::U8, 0, usize::MAX)
        .unwrap(),
        usize::MAX - 1..usize::MAX
    );
    assert!(matches!(
        PqDescriptor {
            count: 2,
            offset: usize::MAX - 1,
        }
        .heap_range(ElementKind::U8, 0, usize::MAX),
        Err(FitsError::DataUnitOverflow)
    ));
    assert!(matches!(
        PqDescriptor {
            count: 2,
            offset: 3,
        }
        .heap_range(ElementKind::U8, 10, 14),
        Err(FitsError::UnexpectedEof)
    ));
}

/// §10.1.3: a 32-bit `P` descriptor holds a signed 32-bit offset and count, so a heap or a cell
/// count past `i32::MAX` needs a 64-bit `Q`; exactly `i32::MAX` still fits a `P`.
#[cfg(feature = "compression")]
#[test]
fn a_descriptor_widens_past_the_32_bit_range() {
    let max = i32::MAX as usize;
    for (heap_len, max_count, width) in [
        (0, 0, DescriptorWidth::P),
        (max, max, DescriptorWidth::P),
        (max + 1, 0, DescriptorWidth::Q),
        (0, max + 1, DescriptorWidth::Q),
    ] {
        assert_eq!(
            DescriptorWidth::addressing(heap_len, max_count),
            width,
            "{heap_len} {max_count}"
        );
    }
}

/// §7.3.5: `P` is two big-endian i32s and `Q` two i64s, count then offset; a
/// value past the form's signed range is refused, not truncated.
#[test]
fn pq_descriptors_are_big_endian_count_then_offset() {
    let p_max = i32::MAX as usize;
    let q_max = i64::MAX as usize;
    for (width, count, offset, expected) in [
        (
            DescriptorWidth::P,
            7,
            40,
            Some(&[0, 0, 0, 7, 0, 0, 0, 40][..]),
        ),
        (
            DescriptorWidth::P,
            p_max,
            p_max,
            Some(&[0x7F, 0xFF, 0xFF, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF][..]),
        ),
        (DescriptorWidth::P, p_max + 1, 0, None),
        (DescriptorWidth::P, 0, p_max + 1, None),
        // A count past u32 and an offset past 2^33: no 32-bit truncation.
        (
            DescriptorWidth::Q,
            0x1_0000_0004,
            0x3_0000_0002,
            Some(&[0, 0, 0, 1, 0, 0, 0, 4, 0, 0, 0, 3, 0, 0, 0, 2][..]),
        ),
        (DescriptorWidth::Q, q_max + 1, 0, None),
        (DescriptorWidth::Q, 0, q_max + 1, None),
    ] {
        let mut slot = vec![0xA5; width.bytes()];
        let written = PqDescriptor { count, offset }.write(width, &mut slot);
        assert_eq!(
            PqDescriptor { count, offset }.validate(width).is_ok(),
            expected.is_some()
        );
        match expected {
            Some(bytes) => {
                written.unwrap();
                assert_eq!(slot, bytes, "{width:?} {count} {offset}");
            }
            None => assert!(
                matches!(
                    written,
                    Err(FitsError::DescriptorOutOfRange { width: w, count: c, offset: o })
                        if w == width && c == count && o == offset
                ),
                "{width:?} {count} {offset}"
            ),
        }
    }
}
