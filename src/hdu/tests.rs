use super::*;
use crate::block::CARD_SIZE;

fn header(lines: &[&str]) -> Header {
    let mut buf = Vec::new();
    for line in lines {
        let mut card = [b' '; CARD_SIZE];
        card[..line.len()].copy_from_slice(line.as_bytes());
        buf.extend_from_slice(&card);
    }
    let mut end = [b' '; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    Header::parse(&buf).unwrap()
}

#[test]
fn classifies_by_mandatory_keywords() {
    assert_eq!(
        HduKind::classify(&header(&["SIMPLE  = T"])),
        HduKind::Primary
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'IMAGE   '"])),
        HduKind::Image
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'TABLE   '"])),
        HduKind::AsciiTable
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'BINTABLE'"])),
        HduKind::BinTable
    );
    assert_eq!(
        HduKind::classify(&header(&["SIMPLE  = T", "GROUPS  = T"])),
        HduKind::RandomGroups
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'FOO     '"])),
        HduKind::Other
    );
}

#[test]
fn image_data_extent_matches_hand_computed_size() {
    // 512×512 signed 16-bit image: 2 × 512 × 512 = 524288 bytes,
    // rounded up to 183 blocks = 527040 bytes.
    let h = header(&[
        "BITPIX  = 16",
        "NAXIS   = 2",
        "NAXIS1  = 512",
        "NAXIS2  = 512",
    ]);
    let e = data_extent(&h).unwrap();
    assert_eq!(e.data_bytes, 524_288);
    assert_eq!(e.padded_bytes, 527_040);
}

#[test]
fn dataless_primary_has_no_data_unit() {
    let e = data_extent(&header(&["BITPIX  = 8", "NAXIS   = 0"])).unwrap();
    assert_eq!(e.data_bytes, 0);
    assert_eq!(e.padded_bytes, 0);
}

#[test]
fn rejects_malformed_pcount_and_gcount_instead_of_clamping() {
    let neg_pcount = header(&[
        "XTENSION= 'BINTABLE'",
        "BITPIX  = 8",
        "NAXIS   = 2",
        "NAXIS1  = 4",
        "NAXIS2  = 3",
        "PCOUNT  = -1",
    ]);
    assert!(matches!(
        data_extent(&neg_pcount),
        Err(FitsError::WrongValueType { name: "PCOUNT" })
    ));

    let zero_gcount = header(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 4",
        "GCOUNT  = 0",
    ]);
    assert!(matches!(
        data_extent(&zero_gcount),
        Err(FitsError::WrongValueType { name: "GCOUNT" })
    ));
}

#[test]
fn axis_product_overflow_is_an_error_not_a_wrap() {
    // Three axes near 2^32 overflow u64 when multiplied — must not wrap into
    // a small, plausible-but-wrong byte count.
    let h = header(&[
        "BITPIX  = 8",
        "NAXIS   = 3",
        "NAXIS1  = 4294967296",
        "NAXIS2  = 4294967296",
        "NAXIS3  = 4294967296",
    ]);
    assert!(matches!(data_extent(&h), Err(FitsError::DataUnitOverflow)));
}

#[test]
fn random_groups_extent_skips_the_zero_first_axis() {
    // BITPIX=-32 (4 bytes), per-group array = 3×4×1×1×1 = 12 elems,
    // plus PCOUNT=6 params, over GCOUNT=7956 groups:
    // 4 × 7956 × (6 + 12) = 572832 bytes → 199 blocks = 573120.
    let h = header(&[
        "BITPIX  = -32",
        "NAXIS   = 6",
        "NAXIS1  = 0",
        "NAXIS2  = 3",
        "NAXIS3  = 4",
        "NAXIS4  = 1",
        "NAXIS5  = 1",
        "NAXIS6  = 1",
        "GROUPS  = T",
        "PCOUNT  = 6",
        "GCOUNT  = 7956",
    ]);
    let e = data_extent(&h).unwrap();
    assert_eq!(e.data_bytes, 572_832);
    assert_eq!(e.padded_bytes, 573_120);
}
