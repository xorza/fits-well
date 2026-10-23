use crate::bintable::descriptor;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::tform_kind::TformKind;
use crate::error::FitsError;

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

    for case in descriptor::internals::malformed_descriptor_cases() {
        case.assert_error(&PqDescriptor::decode(&case.bytes, case.width).unwrap_err());
    }
    for (bytes, width, expected) in [
        (&[0u8; 7][..], DescriptorWidth::P, 8),
        (&[0u8; 15], DescriptorWidth::Q, 16),
    ] {
        assert!(matches!(
            PqDescriptor::decode(bytes, width),
            Err(FitsError::DataSizeMismatch { expected: got, got: len })
                if got == expected && len == bytes.len()
        ));
    }
}

#[test]
fn payload_lengths_and_heap_ranges_cover_boundaries() {
    for (count, expected) in [(0, 0), (1, 1), (8, 1), (9, 2)] {
        assert_eq!(
            descriptor::payload_len(TformKind::Bit, count).unwrap(),
            expected
        );
    }
    assert_eq!(
        PqDescriptor {
            count: 3,
            offset: 5,
        }
        .heap_range(TformKind::I16, 10, 21)
        .unwrap(),
        15..21
    );
    assert_eq!(
        PqDescriptor {
            count: 0,
            offset: usize::MAX,
        }
        .heap_range(TformKind::Byte, 10, 10)
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
        byte_alias.heap_range(TformKind::Byte, 8, 12).unwrap(),
        integer_alias.heap_range(TformKind::I16, 8, 12).unwrap()
    );
    assert_eq!(
        PqDescriptor {
            count: 1,
            offset: usize::MAX - 1,
        }
        .heap_range(TformKind::Byte, 0, usize::MAX)
        .unwrap(),
        usize::MAX - 1..usize::MAX
    );
    assert!(matches!(
        PqDescriptor {
            count: 2,
            offset: usize::MAX - 1,
        }
        .heap_range(TformKind::Byte, 0, usize::MAX),
        Err(FitsError::DataUnitOverflow)
    ));
    assert!(matches!(
        PqDescriptor {
            count: 2,
            offset: 3,
        }
        .heap_range(TformKind::Byte, 10, 14),
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
