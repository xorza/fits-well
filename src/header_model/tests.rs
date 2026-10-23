use crate::fits_writer::render_header;

use crate::header_model::internals::records;
use crate::header_model::*;

fn sample() -> Header {
    Header::parse(&records(&[
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    2",
        "NAXIS1  =                  512",
        "NAXIS2  =                  256",
        "OBJECT  = 'Cygnus X-1'",
        "COMMENT  some annotation",
        "OBJECT  = 'shadowed'",
        "END",
    ]))
    .unwrap()
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
    let entry = h.entry(keywords::OBJECT).unwrap().unwrap();
    assert_eq!(entry.keyword, "OBJECT");
    assert_eq!(entry.value.and_then(Value::as_text), Some("Cygnus X-1"));
    assert_eq!(h.entry(keywords::EXTNAME).unwrap(), None);
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
        Err(FitsError::TypeMismatch { name, expected })
            if name == "OBJECT" && expected == "real"
    ));
    assert!(matches!(
        h.get_logical(keywords::BITPIX),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "BITPIX" && expected == "logical"
    ));

    // `require_*`: the value when present, `MissingKeyword` when absent, and the same
    // `TypeMismatch` as `get_*` when it holds another type.
    assert!(h.require_logical(keywords::SIMPLE).unwrap());
    assert_eq!(h.require_integer(keywords::BITPIX).unwrap(), 16);
    assert_eq!(h.require_real(keywords::BITPIX).unwrap(), 16.0);
    assert_eq!(h.require_text(keywords::OBJECT).unwrap(), "Cygnus X-1");
    assert!(matches!(
        h.require_logical(keywords::EXTEND),
        Err(FitsError::MissingKeyword {
            keyword: keywords::EXTEND
        })
    ));
    assert!(matches!(
        h.require_integer(keywords::PCOUNT),
        Err(FitsError::MissingKeyword {
            keyword: keywords::PCOUNT
        })
    ));
    assert!(matches!(
        h.require_real(keywords::BSCALE),
        Err(FitsError::MissingKeyword {
            keyword: keywords::BSCALE
        })
    ));
    assert!(matches!(
        h.require_text(keywords::EXTNAME),
        Err(FitsError::MissingKeyword {
            keyword: keywords::EXTNAME
        })
    ));
    assert!(matches!(
        h.require_text(keywords::BITPIX),
        Err(FitsError::TypeMismatch { name, expected }) if name == "BITPIX" && expected == "text"
    ));
}

#[test]
fn bounded_integer_getter_rejects_exact_values_outside_i64() {
    let h = Header::parse(&records(&[
        "MIN     = -9223372036854775808",
        "MAX     =  9223372036854775807",
        "BELOW   = -9223372036854775809",
        "ABOVE   =  9223372036854775808",
        "END",
    ]))
    .unwrap();
    assert_eq!(h.get_integer("MIN").unwrap(), Some(i64::MIN));
    assert_eq!(h.get_integer("MAX").unwrap(), Some(i64::MAX));
    for (keyword, decimal) in [
        ("BELOW", "-9223372036854775809"),
        ("ABOVE", "9223372036854775808"),
    ] {
        assert!(matches!(
            h.get_integer(keyword),
            Err(FitsError::IntegerOutOfRange { value, target })
                if value == decimal && target == "i64"
        ));
    }
    assert_eq!(h.get_real("ABOVE").unwrap(), Some(-(i64::MIN as f64)));
}

#[test]
fn iter_yields_every_record_in_order_with_duplicates() {
    let h = sample();
    let entries: Vec<_> = h.iter().collect();

    // Every stored card; END is implicit and not stored.
    assert_eq!(entries.len(), 8);
    let keywords: Vec<&str> = entries.iter().map(|e| e.keyword).collect();
    assert_eq!(
        keywords,
        [
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "OBJECT", "COMMENT", "OBJECT"
        ]
    );

    // A valued card carries its Value; the commentary card carries text, no value.
    assert_eq!(entries[0].value.and_then(Value::as_logical), Some(true)); // SIMPLE = T
    assert_eq!(entries[6].keyword, "COMMENT");
    assert_eq!(entries[6].value, None);
    // Commentary text starts at column 9, so the leading space is significant.
    assert_eq!(entries[6].comment, Some(" some annotation"));

    // Duplicates are preserved in order — unlike `get`, which collapses to the first.
    assert_eq!(
        entries[5].value.and_then(|v| v.as_text()),
        Some("Cygnus X-1")
    );
    assert_eq!(entries[7].value.and_then(|v| v.as_text()), Some("shadowed"));
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
    assert_eq!(h.comments().collect::<Vec<_>>(), [" some annotation"]);
    assert_eq!(h.history().count(), 0);
}

#[test]
fn continue_records_reassemble_a_long_string() {
    let h = Header::parse(&records(&[
        "WEATHER = 'Partly cloudy during the evening f&'",
        "CONTINUE  'ollowed by cloudy skies overnight.&'",
        "CONTINUE  ' Low 21C. Winds NNE at 5 to 10 mph.'",
        "END",
    ]))
    .unwrap();
    assert_eq!(
        h.get_text("WEATHER").unwrap(),
        Some(
            "Partly cloudy during the evening followed by cloudy skies overnight. \
             Low 21C. Winds NNE at 5 to 10 mph."
        )
    );
    // The two CONTINUE records are folded into the value card, not stored.
    assert_eq!(h.cards.len(), 1);
}

#[test]
fn continue_records_concatenate_the_normative_comment_fragments() {
    let h = Header::parse(&records(&[
        "STRKEY  = 'This keyword value is continued &'",
        "CONTINUE  ' over multiple keyword records.&'",
        "CONTINUE  '&' / The comment field for this",
        "CONTINUE  '&' / keyword is also continued",
        "CONTINUE  '' / over multiple records.",
        "END",
    ]))
    .unwrap();
    let entry = h.iter().next().unwrap();
    assert_eq!(
        entry.value.and_then(Value::as_text),
        Some("This keyword value is continued  over multiple keyword records.")
    );
    assert_eq!(
        entry.comment,
        Some("The comment field for this keyword is also continued over multiple records.")
    );
    assert_eq!(h.cards.len(), 1);
}

#[test]
fn trailing_ampersand_without_a_continue_is_a_literal() {
    let h = Header::parse(&records(&["NOTE    = 'ends with amp &'", "END"])).unwrap();
    assert_eq!(h.get_text("NOTE").unwrap(), Some("ends with amp &"));
}

#[test]
fn orphan_continue_is_demoted_to_commentary() {
    let h = Header::parse(&records(&[
        "CONTINUE  'no predecessor' / retained note",
        "END",
    ]))
    .unwrap();
    assert_eq!(h.cards.len(), 1);
    assert!(matches!(h.cards[0], Card::Commentary { .. }));
    assert_eq!(
        h.cards[0].comment(),
        Some("  'no predecessor' / retained note")
    );
    assert_eq!(h.get("CONTINUE").unwrap(), None);

    let mut rendered = Vec::new();
    render_header(&h, &mut rendered).unwrap();
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
    assert_eq!(h.cards.len(), 3);

    // Re-setting a keyword replaces in place — no duplicate card, index stable.
    h.set_internal(keywords::BITPIX, -32);
    assert_eq!(h.get_integer(keywords::BITPIX).unwrap(), Some(-32));
    assert_eq!(h.cards.len(), 3);
    // The attached comment survives on its card.
    assert_eq!(h.cards[0].comment(), Some("conforms"));

    #[cfg(feature = "compression")]
    {
        h.set_internal("ZFORM1", "1J")
            .set_internal("ZCTYP1", "GZIP_1")
            .set_internal("KEEP", 7);
        h.remove_where(|keyword| matches!(keyword, "ZFORM1" | "ZCTYP1"));
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
    assert!(header.cards.is_empty());
}

#[test]
fn builder_appends_commentary_cards() {
    let mut h = Header::new();
    h.set_internal(keywords::SIMPLE, true);
    h.push_comment("made by fits").unwrap();
    h.push_history("step 1").unwrap();
    assert_eq!(h.cards.len(), 3);
    assert!(matches!(h.cards[1], Card::Commentary { .. }));
    assert_eq!(h.cards[1].keyword(), "COMMENT");
    assert_eq!(h.cards[2].keyword(), "HISTORY");
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
        .push_blank("")
        .unwrap();

    assert_eq!(header.get_text(keywords::OBJECT).unwrap(), Some("first"));
    assert_eq!(header.get_real("EXPTIME").unwrap(), Some(30.0));
    assert_eq!(header.get_text(keywords::INSTRUME).unwrap(), Some("CCD-1"));
    assert_eq!(
        header.iter().map(|entry| entry.keyword).collect::<Vec<_>>(),
        ["OBJECT", "EXPTIME", "OBJECT", "INSTRUME", ""]
    );

    let removed = header.remove(keywords::OBJECT).unwrap().unwrap();
    assert_eq!(removed.keyword, "OBJECT");
    assert_eq!(removed.value.unwrap().as_text(), Some("first"));
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
    header.push_comment("kept").unwrap();
    for value in [Some("text"), None] {
        assert!(matches!(
            header.set_optional(keywords::COMMENT, value),
            Err(FitsError::ReservedKeyword {
                keyword: keywords::COMMENT
            })
        ));
    }
    assert_eq!(header.comments().collect::<Vec<_>>(), ["kept"]);
    assert!(matches!(
        header.remove_at(99),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::HeaderRecord,
            index: 99,
            len: 4
        })
    ));
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
            Err(FitsError::ReservedKeyword { keyword: reserved }) if reserved == keyword
        ));
    }
    for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
        assert!(matches!(
            header.set("REAL", value),
            Err(FitsError::InvalidHeaderValue { keyword, reason })
                if keyword == "REAL" && reason == "real values must be finite"
        ));
    }
    assert!(matches!(
        header.set("COMPLEX",
            Value::ComplexReal {
                re: 1.0,
                im: f64::NAN,
            },
        ),
        Err(FitsError::InvalidHeaderValue { keyword, reason })
            if keyword == "COMPLEX" && reason == "complex real components must be finite"
    ));
    assert!(matches!(
        header.set(keywords::OBJECT, "Véga"),
        Err(FitsError::InvalidAscii {
            context: "header text value"
        })
    ));
    assert!(matches!(
        header.set("COMPLEX",
            Value::ComplexInteger {
                re: u128::MAX.into(),
                im: u128::MAX.into(),
            },
        ),
        Err(FitsError::HeaderCardTooLong {
            keyword,
            length: 92,
        }) if keyword == "COMPLEX"
    ));
    assert!(header.cards.is_empty());

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
    assert!(header.cards.is_empty());

    header.set("VALUE", 1).unwrap();
    header.set_comment("VALUE", &"c".repeat(47)).unwrap();
    assert!(matches!(
        header.set_comment("VALUE", &"c".repeat(48)),
        Err(FitsError::HeaderCardTooLong {
            keyword,
            length: 81,
        }) if keyword == "VALUE"
    ));
    assert_eq!(header.cards[0].comment(), Some("c".repeat(47).as_str()));
    assert!(matches!(
        header.set_comment("VALUE", "bad\ncomment"),
        Err(FitsError::InvalidAscii {
            context: "header comment"
        })
    ));
    let before = header.clone();
    assert!(matches!(
        header.set_comment("ABSENT", "lost"),
        Err(FitsError::KeywordNotFound { keyword }) if keyword == "ABSENT"
    ));
    assert_eq!(header.cards, before.cards);

    header.push_comment(&"x".repeat(72)).unwrap();
    assert!(matches!(
        header.push_comment(&"x".repeat(73)),
        Err(FitsError::HeaderCardTooLong {
            keyword,
            length: 81,
        }) if keyword == "COMMENT"
    ));
    assert!(matches!(
        header.push_history("history\nline"),
        Err(FitsError::InvalidAscii {
            context: "header comment"
        })
    ));
    assert_eq!(header.cards.len(), 2);
}

#[test]
fn missing_mandatory_keyword_is_reported() {
    let h = Header::parse(&records(&["SIMPLE  =                    T", "END"])).unwrap();
    assert!(matches!(
        h.bitpix(),
        Err(FitsError::MissingKeyword {
            keyword: keywords::BITPIX
        })
    ));
    assert!(matches!(
        h.naxis(),
        Err(FitsError::MissingKeyword {
            keyword: keywords::NAXIS
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
