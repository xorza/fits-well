use crate::fits_error::ValueKind;
use crate::hdu::hdu_kind::*;
use crate::internals::cards;
use std::assert_matches;

#[test]
fn classifies_by_mandatory_keywords() {
    for (cards, position, kind) in [
        (&["SIMPLE  = T"][..], HduPosition::Primary, HduKind::Primary),
        (
            &["XTENSION= 'IMAGE   '"],
            HduPosition::Extension,
            HduKind::Image,
        ),
        (
            &["XTENSION= 'TABLE   '"],
            HduPosition::Extension,
            HduKind::AsciiTable,
        ),
        (
            &["XTENSION= 'BINTABLE'"],
            HduPosition::Extension,
            HduKind::BinTable,
        ),
        // A BINTABLE flagged ZIMAGE/ZTABLE is classified by its payload, not its container.
        (
            &["XTENSION= 'BINTABLE'", "ZIMAGE  = T"],
            HduPosition::Extension,
            HduKind::CompressedImage,
        ),
        (
            &["XTENSION= 'BINTABLE'", "ZTABLE  = T"],
            HduPosition::Extension,
            HduKind::CompressedTable,
        ),
        (
            &["SIMPLE  = T", "NAXIS   = 1", "NAXIS1  = 0", "GROUPS  = T"],
            HduPosition::Primary,
            HduKind::RandomGroups,
        ),
        (
            &["XTENSION= 'FOO     '"],
            HduPosition::Extension,
            HduKind::Other,
        ),
    ] {
        assert_eq!(
            HduKind::from_header(&cards::from_card_lines(cards), position).unwrap(),
            kind,
            "{cards:?}"
        );
    }
}

/// The data bytes of each kind, or the error its header gives. A random-groups array skips
/// its zero first axis and adds `PCOUNT` parameters to each group; an extension takes
/// `PCOUNT` and `GCOUNT` whatever its `GROUPS`; a primary ignores them.
#[test]
fn data_bytes_follow_each_kind_or_name_the_defect() {
    let missing = |name| FitsError::MissingKeyword {
        name: RecordName::Keyword(name),
    };
    let range = |keyword| FitsError::InvalidKeywordValue {
        keyword,
        defect: KeywordDefect::Range,
    };
    let cases: [(&[&str], HduKind, Result<u64>); 13] = [
        // 2 × 512 × 512.
        (
            &[
                "BITPIX  = 16",
                "NAXIS   = 2",
                "NAXIS1  = 512",
                "NAXIS2  = 512",
            ],
            HduKind::Primary,
            Ok(524_288),
        ),
        (&["BITPIX  = 8", "NAXIS   = 0"], HduKind::Primary, Ok(0)),
        (
            &[
                "BITPIX  = 8",
                "NAXIS   = 1",
                "NAXIS1  = 4",
                "PCOUNT  = 99",
                "GCOUNT  = 5",
            ],
            HduKind::Primary,
            Ok(4),
        ),
        // 4 × 7956 × (6 + 3 × 4 × 1 × 1 × 1).
        (
            &[
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
            ],
            HduKind::RandomGroups,
            Ok(572_832),
        ),
        // 4 × 2 × (1 + the empty product, 1).
        (
            &[
                "BITPIX  = -32",
                "NAXIS   = 1",
                "NAXIS1  = 0",
                "GROUPS  = T",
                "PCOUNT  = 1",
                "GCOUNT  = 2",
            ],
            HduKind::RandomGroups,
            Ok(16),
        ),
        // 1 × 2 × (1 + 2 × 3).
        (
            &[
                "XTENSION= 'OTHER   '",
                "BITPIX  = 8",
                "NAXIS   = 2",
                "NAXIS1  = 2",
                "NAXIS2  = 3",
                "PCOUNT  = 1",
                "GCOUNT  = 2",
                "GROUPS  = T",
            ],
            HduKind::Image,
            Ok(14),
        ),
        (
            &[
                "XTENSION= 'OTHER   '",
                "BITPIX  = 32",
                "NAXIS   = 1",
                "NAXIS1  = 4",
                "PCOUNT  = 3",
                "GCOUNT  = 0",
            ],
            HduKind::Image,
            Ok(0),
        ),
        (
            &[
                "XTENSION= 'IMAGE   '",
                "BITPIX  = 8",
                "NAXIS   = 1",
                "NAXIS1  = 4",
            ],
            HduKind::Image,
            Err(missing(keywords::PCOUNT)),
        ),
        (
            &[
                "XTENSION= 'IMAGE   '",
                "BITPIX  = 8",
                "NAXIS   = 1",
                "NAXIS1  = 4",
                "PCOUNT  = 0",
            ],
            HduKind::Image,
            Err(missing(keywords::GCOUNT)),
        ),
        (
            &[
                "XTENSION= 'BINTABLE'",
                "BITPIX  = 8",
                "NAXIS   = 2",
                "NAXIS1  = 4",
                "NAXIS2  = 3",
                "PCOUNT  = -1",
            ],
            HduKind::Image,
            Err(range(keywords::PCOUNT)),
        ),
        (
            &[
                "XTENSION= 'IMAGE   '",
                "BITPIX  = 8",
                "NAXIS   = 1",
                "NAXIS1  = 4",
                "PCOUNT  = 0",
                "GCOUNT  = -1",
            ],
            HduKind::Image,
            Err(range(keywords::GCOUNT)),
        ),
        (
            &[
                "XTENSION= 'IMAGE   '",
                "BITPIX  = 8",
                "NAXIS   = 0",
                "PCOUNT  = 'not an integer'",
            ],
            HduKind::Image,
            Err(FitsError::ValueTypeMismatch {
                name: RecordName::Keyword(keywords::PCOUNT),
                expected: ValueKind::Integer,
            }),
        ),
        // 2³² cubed passes `u64`: an error, never a small wrapped count.
        (
            &[
                "BITPIX  = 8",
                "NAXIS   = 3",
                "NAXIS1  = 4294967296",
                "NAXIS2  = 4294967296",
                "NAXIS3  = 4294967296",
            ],
            HduKind::Primary,
            Err(FitsError::DataUnitOverflow),
        ),
    ];
    for (cards, kind, expected) in cases {
        assert_eq!(
            format!("{:?}", kind.data_bytes(&cards::from_card_lines(cards))),
            format!("{expected:?}"),
            "{cards:?}"
        );
    }
}

#[test]
fn random_groups_kind_requires_the_zero_first_axis_signature() {
    let no_axes =
        cards::from_card_lines(&["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 0", "GROUPS  = T"]);
    assert_matches!(
        HduKind::from_header(&no_axes, HduPosition::Primary),
        Err(FitsError::InvalidKeywordValue {
            keyword: keywords::NAXIS,
            ..
        })
    );

    let nonzero_first_axis = cards::from_card_lines(&[
        "SIMPLE  = T",
        "BITPIX  = 8",
        "NAXIS   = 1",
        "NAXIS1  = 2",
        "GROUPS  = T",
    ]);
    assert_matches!(
        HduKind::from_header(&nonzero_first_axis, HduPosition::Primary),
        Err(FitsError::InvalidKeywordValue { keyword, defect: KeywordDefect::Structure })
            if keyword == "NAXIS1"
    );
}
