use crate::writer::render_header;

use crate::header::*;

fn header_bytes(lines: &[&str]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(lines.len() * CARD_SIZE);
    for line in lines {
        assert!(line.len() <= CARD_SIZE);
        let mut card = [b' '; CARD_SIZE];
        card[..line.len()].copy_from_slice(line.as_bytes());
        buf.extend_from_slice(&card);
    }
    buf
}

fn sample() -> Header {
    Header::parse(&header_bytes(&[
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
fn end_is_implicit_and_not_stored() {
    let h = sample();
    // 8 content cards: SIMPLE, BITPIX, NAXIS, NAXIS1, NAXIS2, OBJECT, COMMENT, OBJECT.
    assert_eq!(h.cards.len(), 8);
    assert!(h.cards.iter().all(|c| c.kind != CardKind::End));
}

#[test]
fn keyword_lookup_returns_first_occurrence() {
    let h = sample();
    assert_eq!(h.get_text("OBJECT").unwrap(), Some("Cygnus X-1"));
    assert_eq!(h.get("MISSING"), None);
}

#[test]
fn strict_optional_getters_distinguish_absent_and_mistyped_values() {
    let h = sample();
    assert_eq!(h.get_logical("SIMPLE").unwrap(), Some(true));
    assert_eq!(h.get_integer("BITPIX").unwrap(), Some(16));
    assert_eq!(h.get_real("BITPIX").unwrap(), Some(16.0));
    assert_eq!(h.get_text("OBJECT").unwrap(), Some("Cygnus X-1"));
    assert_eq!(h.get_real("MISSING").unwrap(), None);
    assert!(matches!(
        h.get_real("OBJECT"),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "OBJECT" && expected == "real"
    ));
    assert!(matches!(
        h.get_logical("BITPIX"),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "BITPIX" && expected == "logical"
    ));
}

#[test]
fn bounded_integer_getter_rejects_exact_values_outside_i64() {
    let h = Header::parse(&header_bytes(&[
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
    assert_eq!(
        h.get_real("ABOVE").unwrap(),
        Some(9_223_372_036_854_775_808.0)
    );
}

#[test]
fn iter_yields_every_record_in_order_with_duplicates() {
    let h = sample();
    let entries: Vec<_> = h.iter().collect();

    // Every stored card, END excluded — same count `end_is_implicit` checks.
    assert_eq!(entries.len(), 8);
    let keywords: Vec<&str> = entries.iter().map(|e| e.keyword).collect();
    assert_eq!(
        keywords,
        [
            "SIMPLE", "BITPIX", "NAXIS", "NAXIS1", "NAXIS2", "OBJECT", "COMMENT", "OBJECT"
        ]
    );

    // A valued card carries its Value; the commentary card carries text, no value.
    assert_eq!(entries[0].value.and_then(|v| v.as_logical()), Some(true)); // SIMPLE = T
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
    assert_eq!(h.get_text("OBJECT").unwrap(), Some("Cygnus X-1"));
}

#[test]
fn continue_records_reassemble_a_long_string() {
    let h = Header::parse(&header_bytes(&[
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
    let h = Header::parse(&header_bytes(&[
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
    let h = Header::parse(&header_bytes(&["NOTE    = 'ends with amp &'", "END"])).unwrap();
    assert_eq!(h.get_text("NOTE").unwrap(), Some("ends with amp &"));
}

#[test]
fn orphan_continue_is_demoted_to_commentary() {
    let h = Header::parse(&header_bytes(&[
        "CONTINUE  'no predecessor' / retained note",
        "END",
    ]))
    .unwrap();
    assert_eq!(h.cards.len(), 1);
    assert_eq!(h.cards[0].kind, CardKind::Commentary);
    assert_eq!(
        h.cards[0].comment.as_deref(),
        Some("  'no predecessor' / retained note")
    );
    assert_eq!(h.get("CONTINUE"), None);

    let mut rendered = Vec::new();
    render_header(&h, &mut rendered).unwrap();
    let expected = b"CONTINUE  'no predecessor' / retained note";
    assert_eq!(&rendered[..expected.len()], expected);
}

#[test]
fn missing_end_record_is_an_error() {
    let bytes = header_bytes(&["SIMPLE  =                    T"]);
    assert!(matches!(Header::parse(&bytes), Err(FitsError::MissingEnd)));
}

#[test]
fn builder_sets_replaces_and_indexes_keywords() {
    let mut h = Header::new();
    h.set_internal("SIMPLE", true)
        .comment_internal("SIMPLE", "conforms")
        .set_internal("BITPIX", 16)
        .set_internal("OBJECT", "NGC4151");
    assert_eq!(h.get_logical("SIMPLE").unwrap(), Some(true));
    assert_eq!(h.get_integer("BITPIX").unwrap(), Some(16));
    assert_eq!(h.get_text("OBJECT").unwrap(), Some("NGC4151"));
    assert_eq!(h.cards.len(), 3);

    // Re-setting a keyword replaces in place — no duplicate card, index stable.
    h.set_internal("BITPIX", -32);
    assert_eq!(h.get_integer("BITPIX").unwrap(), Some(-32));
    assert_eq!(h.cards.len(), 3);
    // The attached comment survives on its card.
    assert_eq!(h.cards[0].comment.as_deref(), Some("conforms"));

    #[cfg(feature = "compression")]
    {
        h.set_internal("ZFORM1", "1J")
            .set_internal("ZCTYP1", "GZIP_1")
            .set_internal("KEEP", 7);
        h.remove_where(|keyword| matches!(keyword, "ZFORM1" | "ZCTYP1"));
        assert_eq!(h.get("ZFORM1"), None);
        assert_eq!(h.get("ZCTYP1"), None);
        assert_eq!(h.get_integer("KEEP").unwrap(), Some(7));
        assert_eq!(h.get_integer("BITPIX").unwrap(), Some(-32));
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
    h.set_internal("SIMPLE", true);
    h.push_comment("made by fits").unwrap();
    h.push_history("step 1").unwrap();
    assert_eq!(h.cards.len(), 3);
    assert_eq!(h.cards[1].kind, CardKind::Commentary);
    assert_eq!(h.cards[1].keyword, "COMMENT");
    assert_eq!(h.cards[2].keyword, "HISTORY");
    // Commentary cards are not keyword-indexed.
    assert_eq!(h.get("COMMENT"), None);
}

#[test]
fn ordered_header_edits_preserve_duplicates_and_rebuild_lookup() {
    let mut header = Header::new();
    header.set("OBJECT", "first").unwrap();
    header.append("OBJECT", "second").unwrap();
    header.insert(1, "EXPTIME", 30.0).unwrap();
    header
        .set("INSTRUME", "CCD-1")
        .unwrap()
        .push_blank("")
        .unwrap();

    assert_eq!(header.get_text("OBJECT").unwrap(), Some("first"));
    assert_eq!(header.get_real("EXPTIME").unwrap(), Some(30.0));
    assert_eq!(header.get_text("INSTRUME").unwrap(), Some("CCD-1"));
    assert_eq!(
        header.iter().map(|entry| entry.keyword).collect::<Vec<_>>(),
        ["OBJECT", "EXPTIME", "OBJECT", "INSTRUME", ""]
    );

    let removed = header.remove("OBJECT").unwrap();
    assert_eq!(removed.keyword, "OBJECT");
    assert_eq!(removed.value.unwrap().as_text(), Some("first"));
    assert_eq!(header.get_text("OBJECT").unwrap(), Some("second"));
    assert_eq!(header.remove_all("OBJECT"), 1);
    assert_eq!(header.get("OBJECT"), None);
    assert!(matches!(
        header.remove_at(99),
        Err(FitsError::HeaderIndexOutOfBounds { index: 99, len: 3 })
    ));
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
            Err(FitsError::ReservedKeyword { name }) if name == keyword
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
        header.set(
            "COMPLEX",
            Value::ComplexReal {
                re: 1.0,
                im: f64::NAN,
            },
        ),
        Err(FitsError::InvalidHeaderValue { keyword, reason })
            if keyword == "COMPLEX" && reason == "complex real components must be finite"
    ));
    assert!(matches!(
        header.set("OBJECT", "Véga"),
        Err(FitsError::InvalidAscii {
            context: "header text value"
        })
    ));
    assert!(matches!(
        header.set(
            "COMPLEX",
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
    header.comment("VALUE", &"c".repeat(47)).unwrap();
    assert!(matches!(
        header.comment("VALUE", &"c".repeat(48)),
        Err(FitsError::HeaderCardTooLong {
            keyword,
            length: 81,
        }) if keyword == "VALUE"
    ));
    assert_eq!(
        header.cards[0].comment.as_deref(),
        Some("c".repeat(47).as_str())
    );
    assert!(matches!(
        header.comment("VALUE", "bad\ncomment"),
        Err(FitsError::InvalidAscii {
            context: "header comment"
        })
    ));

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
fn built_header_round_trips_through_render_and_parse() {
    let mut h = Header::new();
    h.set_internal("SIMPLE", true)
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 0)
        .set_internal("OBJECT", "test");
    let mut bytes = Vec::new();
    render_header(&h, &mut bytes).unwrap();
    let back = Header::parse(&bytes).unwrap();
    assert_eq!(back.cards, h.cards);
}

#[test]
fn missing_mandatory_keyword_is_reported() {
    let h = Header::parse(&header_bytes(&["SIMPLE  =                    T", "END"])).unwrap();
    assert!(matches!(
        h.bitpix(),
        Err(FitsError::MissingKeyword { name: "BITPIX" })
    ));
    assert!(matches!(
        h.naxis(),
        Err(FitsError::MissingKeyword { name: "NAXIS" })
    ));
}

#[test]
fn naxis_beyond_999_is_rejected() {
    // §4.4.1 caps NAXIS at 999; an absurd value must error rather than drive
    // `Vec::with_capacity(NAXIS)` in `axes()` (an allocation DoS from a tiny header).
    let mut h = Header::new();
    h.set_internal("NAXIS", 1000);
    assert!(matches!(
        h.naxis(),
        Err(FitsError::KeywordOutOfRange { name: "NAXIS" })
    ));
    h.set_internal("NAXIS", 3);
    assert_eq!(h.naxis().unwrap(), 3);
}
