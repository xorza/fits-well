use crate::fits_error::{NonFinite, NumericTarget};
use crate::header_model::*;
use crate::internals::cards::{from_card_lines, record_lines, records};
use crate::internals::rng::SplitMix;

/// The name of each record of `header`, as text; `""` for none.
fn names(header: &Header) -> Vec<String> {
    header
        .iter()
        .map(|record| {
            record
                .name()
                .map_or_else(String::new, |name| name.to_string())
        })
        .collect()
}

fn sample() -> Header {
    from_card_lines(&[
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    2",
        "NAXIS1  =                  512",
        "NAXIS2  =                  256",
        "OBJECT  = 'Cygnus X-1'",
        "COMMENT  some annotation",
        "OBJECT  = 'shadowed'",
    ])
}

/// A valued record's text value; `None` for any other record.
fn valued_text(record: RecordView<'_>) -> Option<&str> {
    match record {
        RecordView::Valued { value, .. } | RecordView::Hierarch { value, .. } => value.as_text(),
        RecordView::Commentary { .. } | RecordView::Malformed { .. } => None,
    }
}

#[test]
fn parses_structural_keywords() {
    let h = sample();
    assert_eq!(h.bitpix().unwrap(), Bitpix::I16);
    assert_eq!(h.naxis().unwrap(), 2);
    assert_eq!(h.axes().unwrap(), vec![512, 256]);
}

#[test]
fn keyword_lookup_returns_first_occurrence() {
    let h = sample();
    assert_eq!(h.get_text(keywords::OBJECT).unwrap(), Some("Cygnus X-1"));
    assert_eq!(h.get("MISSING").unwrap(), None);
    // A name and its preset look up the same card; a name that is no keyword is an
    // error, not an absent card.
    assert_eq!(
        h.get("NAXIS1").unwrap(),
        h.get(keywords::NAXIS.indexed(1).unwrap()).unwrap()
    );
    for name in ["naxis", "NAXIS1234"] {
        assert!(matches!(
            h.get_integer(name),
            Err(FitsError::InvalidKeyword { name: n }) if n == name
        ));
    }
    // The inline comment reads like a value: the first card's, trimmed; `None` for a
    // card without one and for an absent card.
    let commented = from_card_lines(&["OBJECT  = 'M31' /  the target  ", "NAXIS   = 2"]);
    assert_eq!(
        commented.get_comment(keywords::OBJECT).unwrap(),
        Some("the target")
    );
    assert_eq!(commented.get_comment(keywords::NAXIS).unwrap(), None);
    assert_eq!(commented.get_comment(keywords::EXTNAME).unwrap(), None);
}

#[test]
fn strict_optional_getters_distinguish_absent_and_mistyped_values() {
    let h = sample();
    assert_eq!(h.get_logical(keywords::SIMPLE).unwrap(), Some(true));
    assert_eq!(h.get_integer(keywords::BITPIX).unwrap(), Some(16));
    assert_eq!(h.get_real(keywords::BITPIX).unwrap(), Some(16.0));
    assert_eq!(h.get_text(keywords::OBJECT).unwrap(), Some("Cygnus X-1"));
    assert_eq!(h.get_real("MISSING").unwrap(), None);
    assert!(matches!(
        h.get_real(keywords::OBJECT),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Real })
            if name == "OBJECT"
    ));
    assert!(matches!(
        h.get_logical(keywords::BITPIX),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Logical })
            if name == "BITPIX"
    ));

    // `require_*`: the value when present, `MissingKeyword` when absent, and the same
    // `ValueTypeMismatch` as `get_*` when it holds another type.
    assert!(h.require_logical(keywords::SIMPLE).unwrap());
    assert_eq!(h.require_integer(keywords::BITPIX).unwrap(), 16);
    assert_eq!(h.require_real(keywords::BITPIX).unwrap(), 16.0);
    assert_eq!(h.require_text(keywords::OBJECT).unwrap(), "Cygnus X-1");
    assert!(matches!(
        h.require_logical(keywords::EXTEND),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::EXTEND)
        })
    ));
    assert!(matches!(
        h.require_integer(keywords::PCOUNT),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::PCOUNT)
        })
    ));
    assert!(matches!(
        h.require_real(keywords::BSCALE),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::BSCALE)
        })
    ));
    assert!(matches!(
        h.require_text(keywords::EXTNAME),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::EXTNAME)
        })
    ));
    assert!(matches!(
        h.require_text(keywords::BITPIX),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Text }) if name == "BITPIX"
    ));
}

#[test]
fn bounded_integer_getter_rejects_exact_values_outside_i64() {
    let h = from_card_lines(&[
        "MIN     = -9223372036854775808",
        "MAX     =  9223372036854775807",
        "BELOW   = -9223372036854775809",
        "ABOVE   =  9223372036854775808",
        "SEVEN   =                    7",
    ]);
    assert_eq!(h.get_integer("MIN").unwrap(), Some(i64::MIN));
    assert_eq!(h.get_integer("MAX").unwrap(), Some(i64::MAX));
    for (keyword, decimal) in [
        ("BELOW", "-9223372036854775809"),
        ("ABOVE", "9223372036854775808"),
    ] {
        assert!(matches!(
            h.get_integer(keyword),
            Err(FitsError::IntegerOutOfRange { value, target: NumericTarget::I64, .. })
                if value == decimal
        ));
    }
    assert_eq!(h.get_real("ABOVE").unwrap(), Some(-(i64::MIN as f64)));

    // The `usize` forms take 7, refuse a negative value as out of range and pass the
    // i64 refusal on; an absent card is missing, or the default.
    let key = |name| Keyword::try_from(name).unwrap();
    assert_eq!(h.require_usize(key("SEVEN")).unwrap(), 7);
    assert_eq!(h.optional_usize(key("SEVEN"), 3).unwrap(), 7);
    assert_eq!(h.optional_usize(key("ABSENT"), 3).unwrap(), 3);
    assert!(matches!(
        h.require_usize(key("ABSENT")),
        Err(FitsError::MissingKeyword { name: RecordName::Keyword(keyword) }) if keyword == "ABSENT"
    ));
    for result in [h.require_usize(key("MIN")), h.optional_usize(key("MIN"), 3)] {
        assert!(matches!(
            result,
            Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "MIN"
        ));
    }
    for result in [
        h.require_usize(key("ABOVE")),
        h.optional_usize(key("ABOVE"), 3),
    ] {
        assert!(matches!(
            result,
            Err(FitsError::IntegerOutOfRange {
                name: Some(RecordName::Keyword(keyword)),
                target: NumericTarget::I64,
                ..
            }) if keyword == "ABOVE"
        ));
    }
}

/// `append_filtered_from` appends the records `keep` keeps and every one named by no
/// keyword, in order, and the lookup finds the first record of each name after it.
#[test]
fn appending_a_filtered_header_keeps_order_and_lookup() {
    let source = from_card_lines(&[
        "OBJECT  = 'M31'",
        "COMMENT  kept whatever keep says",
        "EXPTIME =                   30 / seconds",
        "OBJECT  = 'shadowed'",
    ]);
    let mut header = from_card_lines(&["INSTRUME= 'CCD'"]);
    header.append_filtered_from(&source, |keyword| keyword != keywords::OBJECT);
    assert_eq!(names(&header), ["INSTRUME", "COMMENT", "EXPTIME"]);
    assert_eq!(header.get_integer("EXPTIME").unwrap(), Some(30));
    assert_eq!(header.get(keywords::OBJECT).unwrap(), None);
    header.append_filtered_from(&source, |_| true);
    assert_eq!(header.get_integer("EXPTIME").unwrap(), Some(30));
    assert_eq!(header.get_text(keywords::OBJECT).unwrap(), Some("M31"));
}

/// `rename_keywords` renames every valued record of a name in place, its value and
/// comment kept, and the lookup follows the new name.
#[cfg(feature = "compression")]
#[test]
fn renamed_keywords_keep_their_place_value_and_comment() {
    let mut header = from_card_lines(&[
        "ZTHEAP  =                    4 / heap start",
        "OTHER   =                    1",
        "ZTHEAP  =                    5",
    ]);
    header.rename_keywords(&[(keywords::ZTHEAP, keywords::THEAP)]);
    assert_eq!(names(&header), ["THEAP", "OTHER", "THEAP"]);
    assert_eq!(header.get(keywords::ZTHEAP).unwrap(), None);
    assert_eq!(header.get_integer(keywords::THEAP).unwrap(), Some(4));
    assert_eq!(
        header.records[0],
        Record::Valued {
            keyword: keywords::THEAP,
            value: Value::Integer(4.into()),
            comment: Some("heap start".to_string()),
        }
    );
}

#[test]
fn iter_yields_every_record_in_order_with_duplicates() {
    let h = sample();
    let entries: Vec<_> = h.iter().collect();

    // Every stored card; END is implicit and not stored.
    assert_eq!(entries.len(), 8);
    let keywords = names(&h);
    assert_eq!(
        keywords,
        [
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "OBJECT", "COMMENT", "OBJECT"
        ]
    );

    // A valued card carries its keyword and Value; the commentary card its name and
    // text. Commentary text starts at column 9, so the leading space is significant.
    assert_eq!(
        entries[0],
        RecordView::Valued {
            keyword: keywords::SIMPLE,
            value: &Value::Logical(true),
            comment: None,
        }
    );
    assert_eq!(
        entries[6],
        RecordView::Commentary {
            name: CommentaryName::COMMENT,
            text: Some(" some annotation"),
        }
    );

    // Duplicates are preserved in order — unlike `get`, which collapses to the first.
    assert_eq!(valued_text(entries[5]), Some("Cygnus X-1"));
    assert_eq!(valued_text(entries[7]), Some("shadowed"));
    assert_eq!(h.get_text(keywords::OBJECT).unwrap(), Some("Cygnus X-1"));

    // The collection view: an exact length, iteration by reference, and positions that
    // count commentary records too.
    assert_eq!(h.len(), 8);
    assert!(!h.is_empty());
    assert!(Header::new().is_empty());
    assert_eq!(h.iter().len(), 8);
    assert_eq!((&h).into_iter().count(), 8);
    assert_eq!(h.index_of(keywords::OBJECT).unwrap(), Some(5));
    assert_eq!(h.index_of(keywords::COMMENT).unwrap(), Some(6));
    assert!(h.contains(keywords::COMMENT).unwrap());
    assert!(!h.contains(keywords::EXTNAME).unwrap());
    assert_eq!(
        h.commentary(CommentaryName::COMMENT).collect::<Vec<_>>(),
        [" some annotation"]
    );
    assert_eq!(h.commentary(CommentaryName::HISTORY).count(), 0);
}

#[test]
fn continue_records_reassemble_a_long_string() {
    let h = from_card_lines(&[
        "WEATHER = 'Partly cloudy during the evening f&'",
        "CONTINUE  'ollowed by cloudy skies overnight.&'",
        "CONTINUE  ' Low 21C. Winds NNE at 5 to 10 mph.'",
    ]);
    assert_eq!(
        h.get_text("WEATHER").unwrap(),
        Some(
            "Partly cloudy during the evening followed by cloudy skies overnight. \
             Low 21C. Winds NNE at 5 to 10 mph."
        )
    );
    // The two CONTINUE records are folded into the value card, not stored.
    assert_eq!(h.records.len(), 1);
}

#[test]
fn continue_records_concatenate_the_normative_comment_fragments() {
    let h = from_card_lines(&[
        "STRKEY  = 'This keyword value is continued &'",
        "CONTINUE  ' over multiple keyword records.&'",
        "CONTINUE  '&' / The comment field for this",
        "CONTINUE  '&' / keyword is also continued",
        "CONTINUE  '' / over multiple records.",
    ]);
    let Some(RecordView::Valued { value, comment, .. }) = h.iter().next() else {
        panic!("the CONTINUE chain reads back as one valued record");
    };
    assert_eq!(
        value.as_text(),
        Some("This keyword value is continued  over multiple keyword records.")
    );
    assert_eq!(
        comment,
        Some("The comment field for this keyword is also continued over multiple records.")
    );
    assert_eq!(h.records.len(), 1);
}

#[test]
fn trailing_ampersand_without_a_continue_is_a_literal() {
    let h = from_card_lines(&["NOTE    = 'ends with amp &'"]);
    assert_eq!(h.get_text("NOTE").unwrap(), Some("ends with amp &"));
}

/// A `CONTINUE` record no value card awaits is the commentary its bytes 9–80 spell, kept
/// as they are: a record already 80 bytes long renders back to its 80 bytes.
#[test]
fn orphan_continue_is_demoted_to_commentary() {
    let full = format!("CONTINUE  'abc'/{}", "x".repeat(64));
    let h = from_card_lines(&[&full]);
    let mut rendered = Vec::new();
    h.render_into(Signing::Unsigned, Space::Spare(0), &mut rendered)
        .unwrap();
    assert_eq!(&rendered[..80], full.as_bytes());

    let h = from_card_lines(&["CONTINUE  'no predecessor' / retained note"]);
    assert_eq!(h.records.len(), 1);
    assert_eq!(
        h.records[0],
        Record::Commentary {
            name: CommentaryName::CONTINUE,
            text: Some("  'no predecessor' / retained note".into()),
        }
    );
    assert_eq!(h.get("CONTINUE").unwrap(), None);

    let mut rendered = Vec::new();
    h.render_into(Signing::Unsigned, Space::Spare(0), &mut rendered)
        .unwrap();
    let expected = b"CONTINUE  'no predecessor' / retained note";
    assert_eq!(&rendered[..expected.len()], expected);
}

#[test]
fn missing_end_record_is_an_error() {
    let bytes = records(&["SIMPLE  =                    T"]);
    assert!(matches!(Header::parse(&bytes), Err(FitsError::MissingEnd)));
}

#[test]
fn builder_sets_replaces_and_indexes_keywords() {
    let mut h = Header::new();
    h.set_internal(keywords::SIMPLE, true)
        .set_comment_internal(keywords::SIMPLE, "conforms")
        .set_internal(keywords::BITPIX, 16)
        .set_internal(keywords::OBJECT, "NGC4151");
    assert_eq!(h.get_logical(keywords::SIMPLE).unwrap(), Some(true));
    assert_eq!(h.get_integer(keywords::BITPIX).unwrap(), Some(16));
    assert_eq!(h.get_text(keywords::OBJECT).unwrap(), Some("NGC4151"));
    assert_eq!(h.records.len(), 3);

    // Re-setting a keyword replaces in place — no duplicate card, index stable.
    h.set_internal(keywords::BITPIX, -32);
    assert_eq!(h.get_integer(keywords::BITPIX).unwrap(), Some(-32));
    assert_eq!(h.records.len(), 3);
    // The attached comment survives on its card.
    assert_eq!(h.records[0].comment(), Some("conforms"));

    #[cfg(feature = "compression")]
    {
        h.set_internal("ZFORM1", "1J")
            .set_internal("ZCTYP1", "GZIP_1")
            .set_internal("KEEP", 7);
        h.remove_where(|keyword| matches!(keyword.as_str(), "ZFORM1" | "ZCTYP1"));
        assert_eq!(h.get("ZFORM1").unwrap(), None);
        assert_eq!(h.get("ZCTYP1").unwrap(), None);
        assert_eq!(h.get_integer("KEEP").unwrap(), Some(7));
        assert_eq!(h.get_integer(keywords::BITPIX).unwrap(), Some(-32));
    }
}

#[test]
fn builder_rejects_nonstandard_keyword_names_without_mutating() {
    let mut header = Header::new();
    for keyword in ["TOO-LONG9", "ESO DET"] {
        assert!(matches!(
            header.set(keyword, 1),
            Err(FitsError::InvalidKeyword { name }) if name == keyword
        ));
        assert!(matches!(
            header.insert(0, keyword, 1),
            Err(FitsError::InvalidKeyword { name }) if name == keyword
        ));
    }
    assert!(header.records.is_empty());
}

#[test]
fn builder_appends_commentary_cards() {
    let mut h = Header::new();
    h.set_internal(keywords::SIMPLE, true);
    h.push_commentary(CommentaryName::COMMENT, "made by fits")
        .unwrap();
    h.push_commentary(CommentaryName::HISTORY, "step 1")
        .unwrap();
    assert_eq!(h.records.len(), 3);
    assert!(matches!(h.records[1], Record::Commentary { .. }));
    assert_eq!(names(&h)[1..], ["COMMENT", "HISTORY"]);
    // Commentary cards are not keyword-indexed.
    assert_eq!(h.get(keywords::COMMENT).unwrap(), None);
}

#[test]
fn ordered_header_edits_preserve_duplicates_and_rebuild_lookup() {
    let mut header = Header::new();
    header.set(keywords::OBJECT, "first").unwrap();
    header.append(keywords::OBJECT, "second").unwrap();
    header.insert(1, "EXPTIME", 30.0).unwrap();
    header
        .set(keywords::INSTRUME, "CCD-1")
        .unwrap()
        .push_commentary(CommentaryName::BLANK, "")
        .unwrap();

    assert_eq!(header.get_text(keywords::OBJECT).unwrap(), Some("first"));
    assert_eq!(header.get_real("EXPTIME").unwrap(), Some(30.0));
    assert_eq!(header.get_text(keywords::INSTRUME).unwrap(), Some("CCD-1"));
    assert_eq!(
        names(&header),
        ["OBJECT", "EXPTIME", "OBJECT", "INSTRUME", ""]
    );

    assert_eq!(
        header.remove(keywords::OBJECT).unwrap(),
        Some(Record::Valued {
            keyword: keywords::OBJECT,
            value: Value::Text("first".to_string()),
            comment: None,
        })
    );
    assert_eq!(header.get_text(keywords::OBJECT).unwrap(), Some("second"));
    assert_eq!(header.remove_all(keywords::OBJECT).unwrap(), 1);
    assert_eq!(header.get(keywords::OBJECT).unwrap(), None);

    // `set_optional`: `Some` sets as `set` does; `None` removes every record with the
    // keyword, duplicates included, so the header holds the value or nothing.
    header
        .set_optional(keywords::OBJECT, Some("third"))
        .unwrap();
    header.append(keywords::OBJECT, "fourth").unwrap();
    assert_eq!(header.get_text(keywords::OBJECT).unwrap(), Some("third"));
    header.set_optional(keywords::OBJECT, None::<&str>).unwrap();
    assert!(!header.contains(keywords::OBJECT).unwrap());
    header.set_optional(keywords::OBJECT, None::<&str>).unwrap();
    assert_eq!(header.len(), 3);
    // A commentary keyword is refused with a value and without one alike.
    header
        .push_commentary(CommentaryName::COMMENT, "kept")
        .unwrap();
    for value in [Some("text"), None] {
        assert!(matches!(
            header.set_optional(keywords::COMMENT, value),
            Err(FitsError::ReservedName {
                name: RecordName::Keyword(keywords::COMMENT)
            })
        ));
    }
    assert_eq!(
        header
            .commentary(CommentaryName::COMMENT)
            .collect::<Vec<_>>(),
        ["kept"]
    );
    assert!(matches!(
        header.remove_at(99),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::HeaderRecord,
            index: 99,
            len: 4
        })
    ));
    // A commentary name of any printable text names its records, and a parse strips
    // the trailing blanks a push strips.
    let free = CommentaryName::try_from("ab cd").unwrap();
    header.push_commentary("ab cd", "free text  ").unwrap();
    assert_eq!(header.commentary(free).collect::<Vec<_>>(), ["free text"]);
    assert_eq!(header.index_of(free).unwrap(), Some(4));
    assert_eq!(header.index_of(keywords::COMMENT).unwrap(), Some(3));
    assert_eq!(header.remove_all(free).unwrap(), 1);
    assert_eq!(header.len(), 4);
}

#[test]
fn values_convert_from_the_widths_callers_hold() {
    // Every conversion is exact: an f32 widens to the same f64, and a usize past
    // i64::MAX keeps its exact decimal.
    assert_eq!(Value::from(7u16).as_integer().unwrap(), Some(7));
    assert_eq!(Value::from(1.5f32).as_real().unwrap(), Some(1.5));
    assert_eq!(
        Value::from(0.1f32).as_real().unwrap(),
        Some(f64::from(0.1f32))
    );
    assert_eq!(Value::from(42usize).as_integer().unwrap(), Some(42));
    let mut header = Header::new();
    header.set("WIDE", usize::MAX).unwrap();
    let Some(Value::Integer(wide)) = header.get("WIDE").unwrap() else {
        panic!("a usize is an exact integer");
    };
    assert_eq!(wide.to_string(), usize::MAX.to_string());
}

#[test]
fn fallible_header_mutation_rejects_invalid_inputs_without_changes() {
    let mut header = Header::new();
    assert!(matches!(
        header.set("bad", 1),
        Err(FitsError::InvalidKeyword { name }) if name == "bad"
    ));
    for keyword in ["END", "CONTINUE", "COMMENT", "HISTORY"] {
        assert!(matches!(
            header.set(keyword, 1),
            Err(FitsError::ReservedName { name: RecordName::Keyword(reserved) }) if reserved == keyword
        ));
    }
    // A commentary name carries no value, nor an inline comment.
    let free = CommentaryName::try_from("ab cd").unwrap();
    assert!(matches!(
        header.set(free, 1),
        Err(FitsError::ReservedName { name: RecordName::Commentary(name) }) if name == free
    ));
    assert!(matches!(
        header.set_comment(free, "c"),
        Err(FitsError::ReservedName { name: RecordName::Commentary(name) }) if name == free
    ));
    // A commentary record that would read back as an `END`, valued, `HIERARCH` or
    // `CONTINUE` record is refused.
    for (name, text) in [
        ("END", "x"),
        ("SIMPLE", "= T"),
        ("A=B", "x"),
        ("HIERARCH", " ESO DET = 1"),
        ("CONTINUE", "  'abc'"),
    ] {
        assert!(
            matches!(
                header.push_commentary(name, text),
                Err(FitsError::AmbiguousCommentary { name: refused }) if refused.as_str() == name
            ),
            "{name:?} {text:?}"
        );
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            header.set("REAL", value),
            Err(FitsError::InvalidHeaderValue { name, reason: NonFinite::Real })
                if name.to_string() == "REAL"
        ));
    }
    assert!(matches!(
        header.set("COMPLEX",
            Value::ComplexReal {
                re: 1.0,
                im: f64::NAN,
            },
        ),
        Err(FitsError::InvalidHeaderValue { name, reason: NonFinite::Complex })
            if name.to_string() == "COMPLEX"
    ));
    assert!(matches!(
        header.set(keywords::OBJECT, "Véga"),
        Err(FitsError::InvalidAscii { text: TextOrigin::Value(name) }) if name.to_string() == "OBJECT"
    ));
    assert!(matches!(
        header.set("COMPLEX",
            Value::ComplexInteger {
                re: u128::MAX.into(),
                im: u128::MAX.into(),
            },
        ),
        Err(FitsError::HeaderCardTooLong {
            name,
            length: 92,
        }) if name.to_string() == "COMPLEX"
    ));
    assert!(header.records.is_empty());

    for keyword in ["", " ESO DET", "ESO DET ", "ESO=DET", "LONGER123"] {
        assert!(matches!(
            header.set(keyword, 1),
            Err(FitsError::InvalidKeyword { name }) if name == keyword
        ));
    }
    assert!(matches!(
        header.set("ESO\nDET", 1),
        Err(FitsError::InvalidKeyword { name }) if name == "ESO\nDET"
    ));
    assert!(header.records.is_empty());

    header.set("VALUE", 1).unwrap();
    header.set_comment("VALUE", &"c".repeat(47)).unwrap();
    assert!(matches!(
        header.set_comment("VALUE", &"c".repeat(48)),
        Err(FitsError::HeaderCardTooLong {
            name,
            length: 81,
        }) if name.to_string() == "VALUE"
    ));
    assert_eq!(header.records[0].comment(), Some("c".repeat(47).as_str()));
    assert!(matches!(
        header.set_comment("VALUE", "bad\ncomment"),
        Err(FitsError::InvalidAscii { text: TextOrigin::Comment(name) }) if name.to_string() == "VALUE"
    ));
    let before = header.clone();
    assert!(matches!(
        header.set_comment("ABSENT", "lost"),
        Err(FitsError::MissingKeyword { name: RecordName::Keyword(keyword) }) if keyword == "ABSENT"
    ));
    assert_eq!(header.records, before.records);

    header
        .push_commentary(CommentaryName::COMMENT, &"x".repeat(72))
        .unwrap();
    assert!(matches!(
        header.push_commentary(CommentaryName::COMMENT, &"x".repeat(73)),
        Err(FitsError::HeaderCardTooLong {
            name,
            length: 81,
        }) if name.to_string() == "COMMENT"
    ));
    assert!(matches!(
        header.push_commentary(CommentaryName::HISTORY, "history\nline"),
        Err(FitsError::InvalidAscii { text: TextOrigin::Comment(name) }) if name.to_string() == "HISTORY"
    ));
    assert_eq!(header.records.len(), 2);
}

#[test]
fn missing_mandatory_keyword_is_reported() {
    let h = from_card_lines(&["SIMPLE  =                    T"]);
    assert!(matches!(
        h.bitpix(),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::BITPIX)
        })
    ));
    assert!(matches!(
        h.naxis(),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::NAXIS)
        })
    ));
}

#[test]
fn naxis_beyond_999_is_rejected() {
    // §4.4.1 caps NAXIS at 999; an absurd value must error rather than drive
    // `Vec::with_capacity(NAXIS)` in `axes()` (an allocation DoS from a tiny header).
    let mut h = Header::new();
    h.set_internal(keywords::NAXIS, 1000);
    assert!(matches!(
        h.naxis(),
        Err(FitsError::KeywordOutOfRange {
            keyword: keywords::NAXIS
        })
    ));
    h.set_internal(keywords::NAXIS, 999);
    assert_eq!(h.naxis().unwrap(), 999);
}

/// The blank records before `END` are space, not content; a blank record among the
/// others stays.
#[test]
fn trailing_blank_records_are_space() {
    let header = from_card_lines(&[
        "SIMPLE  =                    T",
        "",
        "COMMENT x",
        "",
        "",
        "",
    ]);
    assert_eq!(header.len(), 3);
    assert!(matches!(
        header.iter().nth(1),
        Some(RecordView::Commentary {
            name: CommentaryName::BLANK,
            text: None
        })
    ));
}

/// HIERARCH records are found by name without regard to case, through every keyed
/// method a keyword takes, and set in place or appended; a long string continues on
/// `CONTINUE` records and reads back whole, and the records keep their order with the
/// others.
#[test]
fn hierarch_records_are_found_and_set_by_name() {
    let mut header = from_card_lines(&[
        "SIMPLE  =                    T",
        "HIERARCH ESO DET CHIP1 NAME = 'CCD-44'",
        "HIERARCH ESO DET DIT = 2.5",
        "OBJECT  = 'M31'",
    ]);
    assert_eq!(
        header.get("HIERARCH eso det  chip1 name").unwrap(),
        Some(&Value::Text("CCD-44".into()))
    );
    assert_eq!(header.get_real("HIERARCH ESO DET DIT").unwrap(), Some(2.5));
    assert_eq!(header.require_real("HIERARCH ESO DET DIT").unwrap(), 2.5);
    let chip2: RecordName = "HIERARCH ESO DET CHIP2 NAME".parse().unwrap();
    assert_eq!(header.get(chip2.clone()).unwrap(), None);
    assert!(matches!(
        header.require_text(chip2.clone()),
        Err(FitsError::MissingKeyword { name }) if name == chip2
    ));
    assert!(matches!(
        header.get_integer("HIERARCH ESO DET CHIP1 NAME"),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Hierarch(name), expected: ValueKind::Integer })
            if name.as_str() == "ESO DET CHIP1 NAME"
    ));
    assert_eq!(header.index_of("HIERARCH ESO DET DIT").unwrap(), Some(2));
    assert!(!header.contains(chip2).unwrap());
    assert!(matches!(
        header.get("HIERARCH A=B"),
        Err(FitsError::InvalidKeyword { name }) if name == "HIERARCH A=B"
    ));
    header.append("HIERARCH ESO DET DIT", 3.0).unwrap();
    assert_eq!(header.remove_all("HIERARCH ESO DET DIT").unwrap(), 2);
    header.set("HIERARCH ESO DET CHIP1 NAME", "CCD-45").unwrap();
    let long = "x".repeat(100);
    header.set("HIERARCH ESO OBS NAME", long.as_str()).unwrap();
    assert!(matches!(
        header.set("HIERARCH ESO BAD", "caf\u{e9}"),
        Err(FitsError::InvalidAscii { .. })
    ));

    let mut bytes = Vec::new();
    header
        .render_into(Signing::Unsigned, Space::Spare(0), &mut bytes)
        .unwrap();
    let lines: Vec<&str> = record_lines(&bytes).take(6).collect();
    // 80 − (9 + 12 + 3) − 3 = 53 characters fit the first record, 47 the next.
    assert_eq!(
        lines,
        [
            "SIMPLE  =                    T",
            "HIERARCH ESO DET CHIP1 NAME = 'CCD-45  '",
            "OBJECT  = 'M31     '",
            &format!("HIERARCH ESO OBS NAME = '{}&'", "x".repeat(53)),
            &format!("CONTINUE  '{}'", "x".repeat(47)),
            "END",
        ]
    );
    let reread = Header::parse(&bytes).unwrap();
    assert_eq!(
        reread.get("HIERARCH ESO OBS NAME").unwrap(),
        Some(&Value::Text(long))
    );
    assert_eq!(
        names(&reread),
        [
            "SIMPLE",
            "HIERARCH ESO DET CHIP1 NAME",
            "OBJECT",
            "HIERARCH ESO OBS NAME"
        ]
    );
}

/// A HIERARCH record whose value does not parse keeps its name: its lookup fails as a
/// keyword's does, `set` replaces it in its place, and `remove` removes it. Its comment
/// reads with `get_comment` and changes with `set_comment`.
#[test]
fn a_malformed_hierarch_record_keeps_its_name() {
    let mut h = from_card_lines(&[
        "HIERARCH ESO DET GAIN = 1.2.3",
        "HIERARCH ESO DET NAME = 'CCD' / the chip",
    ]);
    assert!(matches!(
        h.get("HIERARCH ESO DET GAIN"),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        h.set_comment("HIERARCH ESO DET GAIN", "gain"),
        Err(FitsError::InvalidValue { .. })
    ));
    assert_eq!(
        h.get_comment("HIERARCH eso det name").unwrap(),
        Some("the chip")
    );
    h.set_comment("HIERARCH ESO DET NAME", "the detector")
        .unwrap();
    assert_eq!(
        h.get_comment("HIERARCH ESO DET NAME").unwrap(),
        Some("the detector")
    );
    h.set("HIERARCH ESO DET GAIN", 1.5).unwrap();
    assert_eq!(h.len(), 2);
    assert_eq!(
        h.get("HIERARCH ESO DET GAIN").unwrap(),
        Some(&Value::Real(1.5))
    );
    assert_eq!(names(&h)[0], "HIERARCH ESO DET GAIN");
    assert!(matches!(
        h.remove("HIERARCH ESO DET NAME").unwrap(),
        Some(Record::Hierarch { .. })
    ));
    assert_eq!(h.get("HIERARCH ESO DET NAME").unwrap(), None);
    assert_eq!(h.remove("HIERARCH ESO DET NAME").unwrap(), None);
}

/// Each edit keeps both indexes on the first record of each name: after a seeded run of
/// inserts and removals of duplicates, every lookup agrees with a scan of the records.
#[test]
fn edits_keep_the_indexes_on_the_first_record_of_each_name() {
    let keywords = ["A", "B", "C"];
    let hierarch = ["X Y", "Z"];
    let mut h = Header::new();
    let mut draws = SplitMix::new(3);
    for step in 0..400 {
        let len = h.len();
        let at = usize::try_from(draws.next()).unwrap() % (len + 1);
        match draws.next() % 4 {
            0 if len > 0 => {
                h.remove_at(at % len).unwrap();
            }
            1 => {
                let name = hierarch[step % hierarch.len()];
                h.insert_record(
                    at,
                    Record::Hierarch {
                        name: name.parse().unwrap(),
                        value: Value::Integer(i64::try_from(step).unwrap().into()),
                        comment: None,
                    },
                )
                .unwrap();
            }
            _ => {
                let keyword = keywords[step % keywords.len()];
                h.insert(at, keyword, i64::try_from(step).unwrap()).unwrap();
            }
        }
        for keyword in keywords {
            let first = h.iter().find_map(|record| match record {
                RecordView::Valued {
                    keyword: k, value, ..
                } if k == keyword => Some(value),
                _ => None,
            });
            assert_eq!(h.get(keyword).unwrap(), first, "step {step}: {keyword}");
        }
        for name in hierarch {
            let wanted: HierarchName = name.parse().unwrap();
            let first = h.iter().find_map(|record| match record {
                RecordView::Hierarch { name, value, .. } if *name == wanted => Some(value),
                _ => None,
            });
            assert_eq!(h.get(wanted).unwrap(), first, "step {step}: {name}");
        }
    }
}

/// `insert_record` puts back what `remove_at` took: a header rebuilt from its own
/// records, one at a time, equals it.
#[test]
fn insert_record_is_the_inverse_of_remove_at() {
    let original = sample();
    let mut h = original.clone();
    let record = h.remove_at(3).unwrap();
    assert_eq!(h.get_integer("NAXIS1").unwrap(), None);
    h.insert_record(3, record).unwrap();
    assert_eq!(h, original);
    assert_eq!(h.get_integer("NAXIS1").unwrap(), Some(512));

    let mut rebuilt = Header::new();
    for index in 0..original.len() {
        rebuilt
            .insert_record(index, h.remove_at(0).unwrap())
            .unwrap();
    }
    assert_eq!(rebuilt, original);
    assert_eq!(rebuilt.get_text("OBJECT").unwrap(), Some("Cygnus X-1"));
    assert!(matches!(
        Header::new().insert_record(1, Record::commentary(CommentaryName::COMMENT, "x")),
        Err(FitsError::IndexOutOfBounds {
            index: 1,
            len: 0,
            ..
        })
    ));
}

/// A signed render sets `CHECKSUM` and `DATASUM` in their place, keeping their comments,
/// or after the records, and leaves the header as it is; the HDU it heads, with no data,
/// sums to negative zero (Appendix J). Blank records fill the blocks asked, and a header
/// that needs more is refused.
#[test]
fn a_signed_render_sets_the_checksums_in_place() {
    let h = from_card_lines(&[
        "SIMPLE  =                    T",
        "CHECKSUM= 'stale' / old",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
    ]);
    let mut out = Vec::new();
    h.render_into(Signing::WithDataSum(0), Space::Blocks(1), &mut out)
        .unwrap();
    assert_eq!(out.len(), BLOCK_SIZE);
    let lines: Vec<&str> = record_lines(&out).collect();
    assert_eq!(lines[0], "SIMPLE  =                    T");
    assert!(lines[1].starts_with("CHECKSUM= '") && lines[1].ends_with("' / old"));
    assert_ne!(&lines[1][11..27], checksum::PLACEHOLDER);
    assert_eq!(lines[4], "DATASUM = '0       '");
    assert_eq!(lines[5..35], [""; 30]);
    assert_eq!(lines[35], "END");
    assert_eq!(checksum::accumulate(&out, 0), 0xFFFF_FFFF);
    assert_eq!(h.get_text(keywords::CHECKSUM).unwrap(), Some("stale"));
    assert_eq!(h.get(keywords::DATASUM).unwrap(), None);

    // 36 records and `END` take two blocks.
    let full = from_card_lines(&["COMMENT x"; 36]);
    assert!(matches!(
        full.render_into(Signing::Unsigned, Space::Blocks(1), &mut out),
        Err(FitsError::HeaderDoesNotFit {
            needed: 2,
            available: 1
        })
    ));
}
