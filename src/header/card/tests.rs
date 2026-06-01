use super::*;

/// Build an 80-byte card from a left-justified text snippet.
fn raw(text: &str) -> [u8; CARD_SIZE] {
    assert!(text.len() <= CARD_SIZE);
    let mut buf = [b' '; CARD_SIZE];
    buf[..text.len()].copy_from_slice(text.as_bytes());
    buf
}

fn parse(text: &str) -> Card {
    Card::parse(&raw(text)).unwrap()
}

#[test]
fn parses_a_logical_card_with_comment() {
    let card = parse("SIMPLE  =                    T / file does conform");
    assert_eq!(card.keyword, "SIMPLE");
    assert_eq!(card.kind, CardKind::Value);
    assert_eq!(card.value, Some(Value::Logical(true)));
    assert_eq!(card.comment.as_deref(), Some("file does conform"));
}

#[test]
fn parses_integers_reals_and_fortran_double_exponent() {
    assert_eq!(
        parse("BITPIX  =                   16").value,
        Some(Value::Integer(16))
    );
    assert_eq!(
        parse("NEG     =                   -5").value,
        Some(Value::Integer(-5))
    );
    assert_eq!(
        parse("EQUINOX =              1950.00").value,
        Some(Value::Real(1950.0))
    );
    assert_eq!(
        parse("UVCVOLT =                 -5.0").value,
        Some(Value::Real(-5.0))
    );
    assert_eq!(
        parse("SCALED  =                2.0D3").value,
        Some(Value::Real(2000.0))
    );
    assert_eq!(
        parse("EXP     =               3.14E2").value,
        Some(Value::Real(314.0))
    );
}

#[test]
fn string_unescapes_quotes_and_trims_only_trailing_spaces() {
    assert_eq!(
        parse("OBJECT  = 'Cygnus X-1'").value,
        Some(Value::Text("Cygnus X-1".into()))
    );
    assert_eq!(
        parse("NAME    = 'O''Brien  '").value,
        Some(Value::Text("O'Brien".into()))
    );
    assert_eq!(
        parse("LEAD    = '   keep'").value,
        Some(Value::Text("   keep".into()))
    );
    // §4.2.1.1: `''` is the null string (length 0); an all-blank string keeps one
    // significant space (length 1), and the two must compare unequal.
    let null = parse("EMPTY   = ''").value;
    assert_eq!(null, Some(Value::Text(String::new())));
    let blank = parse("BLANKS  = '      '").value;
    assert_eq!(blank, Some(Value::Text(" ".into())));
    assert_ne!(null, blank);
}

#[test]
fn large_magnitude_real_renders_with_exponent_and_round_trips() {
    // Display would expand 1e300 to 301 digits and overflow the 80-byte card;
    // format_real must use the §4.2.4 uppercase-`E` form instead (no truncation).
    for &r in &[1e300_f64, -1e300, 1e-300, 2.5e123] {
        let card = Card {
            keyword: "BIG".into(),
            value: Some(Value::Real(r)),
            comment: None,
            kind: CardKind::Value,
        };
        let rendered = card.render();
        let text = std::str::from_utf8(&rendered).unwrap();
        assert!(
            text.contains('E') && !text.contains('e'),
            "expected uppercase exponent, got {text:?}"
        );
        let reparsed = Card::parse(&rendered).unwrap();
        assert_eq!(reparsed.value, Some(Value::Real(r)), "round-trip {r}");
    }
}

#[test]
fn slash_inside_a_string_is_not_a_comment_boundary() {
    let card = parse("PATH    = 'a/b/c' / the real comment");
    assert_eq!(card.value, Some(Value::Text("a/b/c".into())));
    assert_eq!(card.comment.as_deref(), Some("the real comment"));
}

#[test]
fn blank_value_field_is_undefined() {
    let card = parse("DARKCORR= ");
    assert_eq!(card.value, Some(Value::Undefined));
}

#[test]
fn parses_complex_integer_and_real() {
    assert_eq!(
        parse("CPLXI   = (3, 4)").value,
        Some(Value::ComplexInteger { re: 3, im: 4 })
    );
    assert_eq!(
        parse("CPLXR   = (1.0, -2.5)").value,
        Some(Value::ComplexReal { re: 1.0, im: -2.5 })
    );
}

#[test]
fn classifies_end_and_commentary_cards() {
    assert_eq!(parse("END").kind, CardKind::End);

    let comment = parse("COMMENT  this file is great");
    assert_eq!(comment.kind, CardKind::Commentary);
    assert_eq!(comment.keyword, "COMMENT");
    assert_eq!(comment.comment.as_deref(), Some(" this file is great"));

    let history = parse("HISTORY processed 2026-05-31");
    assert_eq!(history.kind, CardKind::Commentary);
    assert_eq!(history.keyword, "HISTORY");

    // Blank-keyword commentary card.
    let blank = parse("         free annotation");
    assert_eq!(blank.kind, CardKind::Commentary);
    assert_eq!(blank.keyword, "");
}

#[test]
fn commentary_text_starting_with_equals_is_not_misread_as_a_value() {
    let card = parse("COMMENT = not a value indicator");
    assert_eq!(card.kind, CardKind::Commentary);
    assert!(card.value.is_none());
}

#[test]
fn rejects_non_ascii_card_without_panicking() {
    // A valid-UTF-8 *multibyte* byte (é = 0xC3 0xA9) straddling the column-8
    // keyword boundary must be rejected, not panic in str slicing.
    let mut bytes = [b' '; CARD_SIZE];
    bytes[..7].copy_from_slice(b"OBJECT ");
    bytes[7] = 0xC3;
    bytes[8] = 0xA9;
    assert!(matches!(
        Card::parse(&bytes),
        Err(FitsError::InvalidValue { .. })
    ));
    // A high byte elsewhere in the record is likewise rejected, not decoded.
    let mut in_value = raw("OBJECT  = 'x'");
    in_value[11] = 0xFF;
    assert!(matches!(
        Card::parse(&in_value),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn rejects_lowercase_keyword_on_a_value_card() {
    assert!(matches!(
        Card::parse(&raw("object  = 'x'")),
        Err(FitsError::InvalidKeyword { .. })
    ));
}

#[test]
fn parses_and_round_trips_a_hierarch_record() {
    let card = parse("HIERARCH ESO DET CHIP1 NAME = 'CCD-44' / detector");
    assert_eq!(card.kind, CardKind::Hierarch);
    assert_eq!(card.keyword, "ESO DET CHIP1 NAME");
    assert_eq!(card.value, Some(Value::Text("CCD-44".into())));
    assert_eq!(card.comment.as_deref(), Some("detector"));
    // Render → parse round-trips the compound key and value.
    let reparsed = Card::parse(&card.render()).unwrap();
    assert_eq!(reparsed, card);

    // A numeric HIERARCH value too.
    let n = parse("HIERARCH ESO DET EXPTIME = 1200");
    assert_eq!(n.keyword, "ESO DET EXPTIME");
    assert_eq!(n.value, Some(Value::Integer(1200)));
}

#[test]
fn parses_a_continue_record() {
    let card = parse("CONTINUE  'ollowed by more text&'");
    assert_eq!(card.kind, CardKind::Continue);
    assert_eq!(
        card.value,
        Some(Value::Text("ollowed by more text&".into()))
    );
}

#[test]
fn long_string_splits_into_a_continue_chain() {
    // A value too long for one record (with an embedded quote that must not be
    // split across a record boundary) renders to multiple records.
    let value = format!("{}'q'{}", "a".repeat(60), "b".repeat(60));
    let card = Card {
        keyword: "LONGSTR".into(),
        value: Some(Value::Text(value.clone())),
        comment: Some("trailing note".into()),
        kind: CardKind::Value,
    };
    let records = card.render_records();
    assert!(records.len() >= 2, "expected a CONTINUE chain");
    assert_eq!(&records[0][..8], b"LONGSTR ");
    assert_eq!(records[0][8], b'='); // first record carries the value indicator
    assert_eq!(&records[1][..8], b"CONTINUE");
    // Non-final records end their quoted substring with the '&' flag.
    let first = std::str::from_utf8(&records[0]).unwrap();
    assert!(first.trim_end().ends_with("&'"));

    // The chain reassembles to the original value (comment on the last record).
    let bytes: Vec<u8> = records.iter().flatten().copied().collect();
    let mut with_end = bytes;
    with_end.extend_from_slice(&raw("END"));
    let h = crate::header::Header::parse(&with_end).unwrap();
    assert_eq!(h.get_text("LONGSTR"), Some(value.as_str()));
}

#[test]
fn long_hierarch_string_splits_into_a_continue_chain() {
    // A HIERARCH card whose key + long string value overflows one record must
    // continue, not silently truncate at 80 bytes.
    let value = "x".repeat(120);
    let card = Card {
        keyword: "ESO DET CHIP1 LONGNAME".into(),
        value: Some(Value::Text(value.clone())),
        comment: Some("note".into()),
        kind: CardKind::Hierarch,
    };
    let records = card.render_records();
    assert!(records.len() >= 2, "expected a CONTINUE chain");
    // First record carries the HIERARCH prefix and a continued ('&) substring.
    let first = std::str::from_utf8(&records[0]).unwrap();
    assert!(first.starts_with("HIERARCH ESO DET CHIP1 LONGNAME = '"));
    assert!(first.trim_end().ends_with("&'"));
    assert_eq!(&records[1][..8], b"CONTINUE");
    // Reassembles to the full value — nothing truncated.
    let bytes: Vec<u8> = records.iter().flatten().copied().collect();
    let mut with_end = bytes;
    with_end.extend_from_slice(&raw("END"));
    let h = crate::header::Header::parse(&with_end).unwrap();
    assert_eq!(h.get_text("ESO DET CHIP1 LONGNAME"), Some(value.as_str()));
}

#[test]
fn short_string_renders_to_a_single_record() {
    let card = parse("OBJECT  = 'Cygnus X-1'");
    assert_eq!(card.render_records().len(), 1);
}

#[test]
fn render_then_parse_round_trips_the_model() {
    let originals = [
        "SIMPLE  =                    T / file does conform",
        "BITPIX  =                  -32 / bits per pixel",
        "NAXIS   =                    2",
        "EQUINOX =              1950.00 / epoch",
        "OBJECT  = 'O''Brien' / observer",
        "DARKCORR= ",
        "CPLXR   = (1.0, -2.5)",
        "END",
        "COMMENT  some words here",
    ];
    for text in originals {
        let card = parse(text);
        let reparsed = Card::parse(&card.render()).unwrap();
        assert_eq!(card, reparsed, "round-trip failed for {text:?}");
    }
}
