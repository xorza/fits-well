use crate::hdu::*;
use crate::header::from_card_lines as header;

#[test]
fn classifies_by_mandatory_keywords() {
    assert_eq!(
        HduKind::classify(&header(&["SIMPLE  = T"]), HduRole::Primary).unwrap(),
        HduKind::Primary
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'IMAGE   '"]), HduRole::Extension).unwrap(),
        HduKind::Image
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'TABLE   '"]), HduRole::Extension).unwrap(),
        HduKind::AsciiTable
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'BINTABLE'"]), HduRole::Extension).unwrap(),
        HduKind::BinTable
    );
    // A BINTABLE flagged ZIMAGE/ZTABLE is classified by its payload, not its container.
    assert_eq!(
        HduKind::classify(
            &header(&["XTENSION= 'BINTABLE'", "ZIMAGE  = T"]),
            HduRole::Extension
        )
        .unwrap(),
        HduKind::CompressedImage
    );
    assert_eq!(
        HduKind::classify(
            &header(&["XTENSION= 'BINTABLE'", "ZTABLE  = T"]),
            HduRole::Extension
        )
        .unwrap(),
        HduKind::CompressedTable
    );
    assert_eq!(
        HduKind::classify(&header(&["SIMPLE  = T"]), HduRole::RandomGroups).unwrap(),
        HduKind::RandomGroups
    );
    assert_eq!(
        HduKind::classify(&header(&["XTENSION= 'FOO     '"]), HduRole::Extension).unwrap(),
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
    let e = data_extent(&h, HduRole::Primary).unwrap();
    assert_eq!(e.data_bytes, 524_288);
    assert_eq!(e.padded_bytes, 527_040);
}

#[test]
fn dataless_primary_has_no_data_unit() {
    let e = data_extent(&header(&["BITPIX  = 8", "NAXIS   = 0"]), HduRole::Primary).unwrap();
    assert_eq!(e.data_bytes, 0);
    assert_eq!(e.padded_bytes, 0);
}

#[test]
fn rejects_negative_pcount_and_gcount_instead_of_clamping() {
    let neg_pcount = header(&[
        "XTENSION= 'BINTABLE'",
        "BITPIX  = 8",
        "NAXIS   = 2",
        "NAXIS1  = 4",
        "NAXIS2  = 3",
        "PCOUNT  = -1",
    ]);
    assert!(matches!(
        data_extent(&neg_pcount, HduRole::Extension),
        Err(FitsError::KeywordOutOfRange { name: "PCOUNT" })
    ));

    let negative_gcount = header(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 4",
        "PCOUNT  = 0",
        "GCOUNT  = -1",
    ]);
    assert!(matches!(
        data_extent(&negative_gcount, HduRole::Extension),
        Err(FitsError::KeywordOutOfRange { name: "GCOUNT" })
    ));

    let mut mistyped_pcount = neg_pcount;
    mistyped_pcount.set_internal("PCOUNT", "not an integer");
    assert!(matches!(
        data_extent(&mistyped_pcount, HduRole::Extension),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "PCOUNT" && expected == "integer"
    ));
}

#[test]
fn zero_gcount_describes_an_empty_grouped_data_unit() {
    let extension = header(&[
        "XTENSION= 'OTHER   '",
        "BITPIX  = 32",
        "NAXIS   = 1",
        "NAXIS1  = 4",
        "PCOUNT  = 3",
        "GCOUNT  = 0",
    ]);
    assert_eq!(
        data_extent(&extension, HduRole::Extension).unwrap(),
        DataExtent {
            data_bytes: 0,
            padded_bytes: 0,
        }
    );
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
    assert!(matches!(
        data_extent(&h, HduRole::Primary),
        Err(FitsError::DataUnitOverflow)
    ));
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
    let e = data_extent(&h, HduRole::RandomGroups).unwrap();
    assert_eq!(e.data_bytes, 572_832);
    assert_eq!(e.padded_bytes, 573_120);
}

#[test]
fn naxis1_random_groups_extent_uses_the_empty_product() {
    let h = header(&[
        "BITPIX  = -32",
        "NAXIS   = 1",
        "NAXIS1  = 0",
        "GROUPS  = T",
        "PCOUNT  = 1",
        "GCOUNT  = 2",
    ]);
    let extent = data_extent(&h, HduRole::RandomGroups).unwrap();
    assert_eq!(extent.data_bytes, 2 * (1 + 1) * 4);
    assert_eq!(extent.padded_bytes, 2880);
}

#[test]
fn role_specific_extents_require_counts_and_ignore_groups_on_extensions() {
    let primary_with_reserved_counts = header(&[
        "SIMPLE  = T",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 4",
        "PCOUNT  = 99",
        "GCOUNT  = 5",
    ]);
    let extent = data_extent(&primary_with_reserved_counts, HduRole::Primary).unwrap();
    assert_eq!(extent.data_bytes, 4);

    let missing_counts = header(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 4",
    ]);
    assert!(matches!(
        data_extent(&missing_counts, HduRole::Extension),
        Err(FitsError::MissingKeyword { name: "PCOUNT" })
    ));

    let missing_gcount = header(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 4",
        "PCOUNT  = 0",
    ]);
    assert!(matches!(
        data_extent(&missing_gcount, HduRole::Extension),
        Err(FitsError::MissingKeyword { name: "GCOUNT" })
    ));

    let extension_with_groups = header(&[
        "XTENSION= 'OTHER   '",
        "BITPIX  = 8",
        "NAXIS   = 2",
        "NAXIS1  = 2",
        "NAXIS2  = 3",
        "PCOUNT  = 1",
        "GCOUNT  = 2",
        "GROUPS  = T",
    ]);
    let extent = data_extent(&extension_with_groups, HduRole::Extension).unwrap();
    assert_eq!(extent.data_bytes, 2 * (1 + 2 * 3));
}

#[test]
fn random_groups_role_requires_the_zero_first_axis_signature() {
    let no_axes = header(&["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 0", "GROUPS  = T"]);
    assert!(matches!(
        HduRole::from_header(&no_axes, HduPosition::Primary),
        Err(FitsError::KeywordOutOfRange { name: "NAXIS" })
    ));

    let nonzero_first_axis = header(&[
        "SIMPLE  = T",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 2",
        "GROUPS  = T",
    ]);
    assert!(matches!(
        HduRole::from_header(&nonzero_first_axis, HduPosition::Primary),
        Err(FitsError::KeywordOutOfRange { name: "NAXIS1" })
    ));
}
