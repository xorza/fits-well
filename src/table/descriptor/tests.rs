use crate::error::FitsError;
use crate::table_impl::TformKind;
use crate::table_impl::descriptor;
use crate::table_impl::descriptor::PqDescriptor;

#[test]
fn decodes_signed_boundaries_and_rejects_malformed_widths() {
    let mut narrow = Vec::new();
    narrow.extend_from_slice(&i32::MAX.to_be_bytes());
    narrow.extend_from_slice(&i32::MAX.to_be_bytes());
    assert_eq!(
        PqDescriptor::decode(&narrow, false).unwrap(),
        PqDescriptor {
            count: i32::MAX as usize,
            offset: i32::MAX as usize,
        }
    );

    let mut wide = Vec::new();
    wide.extend_from_slice(&i64::MAX.to_be_bytes());
    wide.extend_from_slice(&i64::MAX.to_be_bytes());
    let decoded = PqDescriptor::decode(&wide, true);
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
        PqDescriptor::decode(&[0; 8], false).unwrap(),
        PqDescriptor::EMPTY
    );
    assert_eq!(
        PqDescriptor::decode(&[0; 16], true).unwrap(),
        PqDescriptor::EMPTY
    );

    for case in descriptor::internals::malformed_descriptor_cases() {
        case.assert_error(PqDescriptor::decode(&case.bytes, case.wide).unwrap_err());
    }
    for (bytes, wide, expected) in [(&[0u8; 7][..], false, 8), (&[0u8; 15], true, 16)] {
        assert!(matches!(
            PqDescriptor::decode(bytes, wide),
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
