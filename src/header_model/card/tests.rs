use crate::header_model::Header;

use crate::header_model::card::*;

/// Build an 80-byte card from a left-justified text snippet.
fn raw(text: &str) -> [u8; CARD_SIZE] {
    assert!(text.len() <= CARD_SIZE);
    let mut buf = [b' '; CARD_SIZE];
    buf[..text.len()].copy_from_slice(text.as_bytes());
    buf
}

fn parse(text: &str) -> Card {
    reparse(&raw(text))
}

fn reparse(bytes: &[u8; CARD_SIZE]) -> Card {
    match Record::parse(bytes).unwrap() {
        Record::Card(card) => card,
        other => panic!("expected a stored card, got {other:?}"),
    }
}

/// One record: the card's checks, then its single-record rendering.
fn render(card: &Card) -> Result<[u8; CARD_SIZE]> {
    card.validate_contents()?;
    card.render_one()
}

fn render_records(card: &Card) -> Vec<[u8; CARD_SIZE]> {
    let mut bytes = Vec::new();
    card.render_into(&mut bytes).unwrap();
    bytes.as_chunks::<CARD_SIZE>().0.to_vec()
}

#[test]
fn parses_a_logical_card_with_comment() {
    let card = parse("SIMPLE  =                    T / file does conform");
    assert_eq!(card.keyword(), "SIMPLE");
    assert!(matches!(card, Card::Value { .. }));
    assert_eq!(card.value(), Some(&Value::Logical(true)));
    assert_eq!(card.comment(), Some("file does conform"));
}

#[test]
fn parses_integers_reals_and_fortran_double_exponent() {
    assert_eq!(
        parse("BITPIX  =                   16").value(),
        Some(&Value::from(16_i64))
    );
    assert_eq!(
        parse("NEG     =                   -5").value(),
        Some(&Value::from(-5_i64))
    );
    assert_eq!(
        parse("EQUINOX =              1950.00").value(),
        Some(&Value::Real(1950.0))
    );
    assert_eq!(
        parse("UVCVOLT =                 -5.0").value(),
        Some(&Value::Real(-5.0))
    );
    assert_eq!(
        parse("SCALED  =                2.0D3").value(),
        Some(&Value::Real(2000.0))
    );
    assert_eq!(
        parse("EXP     =               3.14E2").value(),
        Some(&Value::Real(314.0))
    );
}

#[test]
fn string_unescapes_quotes_and_trims_only_trailing_spaces() {
    assert_eq!(
        parse("OBJECT  = 'Cygnus X-1'").value(),
        Some(&Value::Text("Cygnus X-1".into()))
    );
    assert_eq!(
        parse("NAME    = 'O''Brien  '").value(),
        Some(&Value::Text("O'Brien".into()))
    );
    assert_eq!(
        parse("LEAD    = '   keep'").value(),
        Some(&Value::Text("   keep".into()))
    );
    // §4.2.1.1: `''` is the null string (length 0); an all-blank string keeps one
    // significant space (length 1), and the two must compare unequal.
    let null = parse("EMPTY   = ''").value().cloned();
    assert_eq!(null, Some(Value::Text(String::new())));
    let blank = parse("BLANKS  = '      '").value().cloned();
    assert_eq!(blank, Some(Value::Text(" ".into())));
    assert_ne!(null, blank);
}

#[test]
fn large_magnitude_real_renders_with_exponent_and_round_trips() {
    // Display would expand 1e300 to 301 digits and overflow the 80-byte card;
    // format_real must use the §4.2.4 uppercase-`E` form instead (no truncation).
    for &r in &[1e300_f64, -1e300, 1e-300, 2.5e123] {
        let card = Card::Value {
            keyword: "BIG".into(),
            value: Value::Real(r),
            comment: None,
        };
        let rendered = render(&card).unwrap();
        let text = std::str::from_utf8(&rendered).unwrap();
        assert!(
            text.contains('E') && !text.contains('e'),
            "expected uppercase exponent, got {text:?}"
        );
        let reparsed = reparse(&rendered);
        assert_eq!(reparsed.value(), Some(&Value::Real(r)), "round-trip {r}");
    }
}

#[test]
fn slash_inside_a_string_is_not_a_comment_boundary() {
    let card = parse("PATH    = 'a/b/c' / the real comment");
    assert_eq!(card.value(), Some(&Value::Text("a/b/c".into())));
    assert_eq!(card.comment(), Some("the real comment"));
}

#[test]
fn blank_value_field_is_undefined() {
    let card = parse("DARKCORR= ");
    assert_eq!(card.value(), Some(&Value::Undefined));
}

#[test]
fn parses_complex_integer_and_real() {
    assert_eq!(
        parse("CPLXI   = (3, 4)").value(),
        Some(&Value::ComplexInteger {
            re: 3_i64.into(),
            im: 4_i64.into()
        })
    );
    assert_eq!(
        parse("CPLXR   = (1.0, -2.5)").value(),
        Some(&Value::ComplexReal { re: 1.0, im: -2.5 })
    );

    let exact = parse("CPLXBIG = (9223372036854775808, -9223372036854775809)");
    let Some(Value::ComplexInteger { re, im }) = exact.value() else {
        panic!("large complex integer components lost their exact representation");
    };
    assert_eq!(re.to_string(), "9223372036854775808");
    assert_eq!(im.to_string(), "-9223372036854775809");
    assert_eq!(reparse(&render(&exact).unwrap()), exact);
}

#[test]
fn classifies_end_and_commentary_cards() {
    assert_eq!(Record::parse(&raw("END")).unwrap(), Record::End);

    let comment = parse("COMMENT  this file is great");
    assert!(matches!(comment, Card::Commentary { .. }));
    assert_eq!(comment.keyword(), "COMMENT");
    assert_eq!(comment.comment(), Some(" this file is great"));

    let history = parse("HISTORY processed 2026-05-31");
    assert!(matches!(history, Card::Commentary { .. }));
    assert_eq!(history.keyword(), "HISTORY");

    // Blank-keyword commentary card.
    let blank = parse("         free annotation");
    assert!(matches!(blank, Card::Commentary { .. }));
    assert_eq!(blank.keyword(), "");
}

#[test]
fn commentary_text_starting_with_equals_is_not_misread_as_a_value() {
    let card = parse("COMMENT = not a value indicator");
    assert!(matches!(card, Card::Commentary { .. }));
    assert!(card.value().is_none());
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
        Record::parse(&bytes),
        Err(FitsError::InvalidValue { .. })
    ));
    // A high byte elsewhere in the record is likewise rejected, not decoded.
    let mut in_value = raw("OBJECT  = 'x'");
    in_value[11] = 0xFF;
    assert!(matches!(
        Record::parse(&in_value),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn rejects_lowercase_keyword_on_a_value_card() {
    assert!(matches!(
        Record::parse(&raw("object  = 'x'")),
        Err(FitsError::InvalidKeyword { .. })
    ));
}

#[test]
fn preserves_a_hierarch_record_as_opaque_commentary() {
    let card = parse("HIERARCH ESO DET CHIP1 NAME = 'CCD-44' / detector");
    assert!(matches!(card, Card::Commentary { .. }));
    assert_eq!(card.keyword(), "HIERARCH");
    assert_eq!(card.value(), None);
    assert_eq!(
        card.comment(),
        Some(" ESO DET CHIP1 NAME = 'CCD-44' / detector")
    );
    let reparsed = reparse(&render(&card).unwrap());
    assert_eq!(reparsed, card);
}

#[test]
fn integer_boundaries_round_trip_without_real_coercion() {
    for decimal in [
        "-9223372036854775809",
        "-9223372036854775808",
        "9223372036854775807",
        "9223372036854775808",
    ] {
        let card = parse(&format!("EXACT   = {decimal}"));
        let Value::Integer(value) = card.value().unwrap() else {
            panic!("{decimal} was not parsed as an exact integer");
        };
        assert_eq!(value.to_string(), decimal);
        let rendered = render(&card).unwrap();
        assert!(
            std::str::from_utf8(&rendered).unwrap().contains(decimal),
            "rendered card changed {decimal}"
        );
        assert_eq!(reparse(&rendered), card);
    }
}

#[test]
fn parses_a_continue_record() {
    assert_eq!(
        Record::parse(&raw("CONTINUE  'ollowed by more text&'")).unwrap(),
        Record::Continue {
            substring: "ollowed by more text&".into(),
            comment: None
        }
    );
}

#[test]
fn end_requires_the_canonical_blank_record() {
    assert_eq!(Record::parse(&raw("END")).unwrap(), Record::End);
    assert!(matches!(
        Record::parse(&raw("END     =                    T")),
        Err(FitsError::ReservedKeyword { name }) if name == "END"
    ));
}

#[test]
fn long_string_splits_into_a_continue_chain() {
    // A value too long for one record (with an embedded quote that must not be
    // split across a record boundary) renders to multiple records.
    let value = format!("{}'q'{}", "a".repeat(60), "b".repeat(60));
    let card = Card::Value {
        keyword: "LONGSTR".into(),
        value: Value::Text(value.clone()),
        comment: Some("trailing note".into()),
    };
    let records = render_records(&card);
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
    let h = Header::parse(&with_end).unwrap();
    assert_eq!(h.get_text("LONGSTR").unwrap(), Some(value.as_str()));
}

#[test]
fn long_string_comment_boundary_is_lossless_or_rejected() {
    let exact_comment = "c".repeat(65);
    let exact = Card::Value {
        keyword: "TEXT".into(),
        value: Value::Text("x".into()),
        comment: Some(exact_comment.clone()),
    };
    let records = render_records(&exact);
    assert_eq!(records.len(), 2);
    assert_eq!(&records[1][..15], b"CONTINUE  '' / ");
    let mut bytes: Vec<u8> = records.iter().flatten().copied().collect();
    bytes.extend_from_slice(&raw("END"));
    let parsed = Header::parse(&bytes).unwrap();
    let entry = parsed.iter().next().unwrap();
    assert_eq!(entry.value.and_then(Value::as_text), Some("x"));
    assert_eq!(entry.comment, Some(exact_comment.as_str()));

    let overflow = Card::Value {
        keyword: "TEXT".into(),
        value: Value::Text("x".into()),
        comment: Some("c".repeat(66)),
    };
    let mut output = vec![1, 2, 3];
    assert!(matches!(
        overflow.render_into(&mut output),
        Err(FitsError::HeaderCardTooLong {
            keyword,
            length: 81,
        }) if keyword == "TEXT"
    ));
    assert_eq!(output, vec![1, 2, 3]);
}

#[test]
fn short_string_renders_to_a_single_record() {
    let card = parse("OBJECT  = 'Cygnus X-1'");
    assert_eq!(render_records(&card).len(), 1);
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
        "COMMENT  some words here",
    ];
    for text in originals {
        let card = parse(text);
        let reparsed = reparse(&render(&card).unwrap());
        assert_eq!(card, reparsed, "round-trip failed for {text:?}");
    }
}

#[test]
fn non_finite_reals_are_rejected_on_read() {
    // §4.2.4 has no inf/NaN value form; Rust's float parser would accept them (and
    // an overflowing magnitude yields inf), so the reader must reject them.
    for token in ["inf", "Infinity", "nan", "-inf", "1E400"] {
        let card = format!("BADREAL = {token}");
        assert!(
            Record::parse(&raw(&card)).is_err(),
            "expected {token:?} to be rejected, not parsed as a real"
        );
    }
    assert_eq!(parse("OK      = 1.5").value(), Some(&Value::Real(1.5)));
}

#[test]
fn rendering_a_non_finite_real_returns_an_error() {
    let card = Card::Value {
        keyword: "BAD".into(),
        value: Value::Real(f64::INFINITY),
        comment: None,
    };
    assert!(matches!(
        render(&card),
        Err(FitsError::InvalidHeaderValue { keyword, reason })
            if keyword == "BAD" && reason == "real values must be finite"
    ));
}

/// The value field of each kind, exactly: right-justified to column 30 except
/// strings, an integral real gains `.0`, an extreme real takes the `E` form, and a
/// string pads to 8 characters once its quotes are doubled.
#[test]
fn every_value_kind_renders_its_fixed_format_field() {
    let right = |text: &str| format!("{text:>20}");
    let cases = [
        (Value::Logical(true), right("T")),
        (Value::Integer((-5_i64).into()), right("-5")),
        (Value::Real(5.0), right("5.0")),
        (Value::Real(-0.0), right("-0.0")),
        (Value::Real(0.1), right("0.1")),
        (Value::Real(1e300), right("1E300")),
        (
            Value::ComplexReal { re: 1.0, im: -2.5 },
            right("(1.0, -2.5)"),
        ),
        (Value::Text("ab".into()), "'ab      '".to_string()),
        (Value::Text("O'Brien".into()), "'O''Brien'".to_string()),
        (Value::Undefined, String::new()),
    ];
    for (value, field) in cases {
        let card = Card::Value {
            keyword: "KEY".into(),
            value: value.clone(),
            comment: None,
        };
        let record = render(&card).unwrap();
        assert_eq!(
            std::str::from_utf8(&record).unwrap(),
            format!("KEY     = {field:<70}"),
            "{value:?}"
        );
        assert_eq!(reparse(&record), card, "{value:?}");
    }
}

/// Each substring holds at most 67 characters once quotes are doubled, and a
/// doubled quote never straddles two: 66 letters and a quote need 68, so the quote
/// opens the next substring.
#[test]
fn a_long_string_splits_at_67_escaped_characters() {
    let pieces = |text: &str| -> Vec<String> {
        LongString {
            text,
            comment: None,
        }
        .pieces()
        .map(str::to_string)
        .collect()
    };
    let a = |n| "a".repeat(n);
    assert_eq!(pieces(""), [""]);
    assert_eq!(pieces(&a(67)), [a(67)]);
    assert_eq!(pieces(&a(68)), [a(67), a(1)]);
    assert_eq!(pieces(&(a(66) + "'")), [a(66), "'".to_string()]);
    assert_eq!(pieces(&(a(65) + "'")), [a(65) + "'"]);
}
