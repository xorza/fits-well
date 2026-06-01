use super::*;

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
    assert_eq!(h.get_text("OBJECT"), Some("Cygnus X-1"));
    assert_eq!(h.get("MISSING"), None);
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
        h.get_text("WEATHER"),
        Some(
            "Partly cloudy during the evening followed by cloudy skies overnight. \
             Low 21C. Winds NNE at 5 to 10 mph."
        )
    );
    // The two CONTINUE records are folded into the value card, not stored.
    assert_eq!(h.cards.len(), 1);
}

#[test]
fn trailing_ampersand_without_a_continue_is_a_literal() {
    let h = Header::parse(&header_bytes(&["NOTE    = 'ends with amp &'", "END"])).unwrap();
    assert_eq!(h.get_text("NOTE"), Some("ends with amp &"));
}

#[test]
fn orphan_continue_is_demoted_to_commentary() {
    let h = Header::parse(&header_bytes(&["CONTINUE  'no predecessor'", "END"])).unwrap();
    assert_eq!(h.cards.len(), 1);
    assert_eq!(h.cards[0].kind, CardKind::Commentary);
    assert_eq!(h.get("CONTINUE"), None);
}

#[test]
fn missing_end_record_is_an_error() {
    let bytes = header_bytes(&["SIMPLE  =                    T"]);
    assert!(matches!(Header::parse(&bytes), Err(FitsError::MissingEnd)));
}

#[test]
fn builder_sets_replaces_and_indexes_keywords() {
    let mut h = Header::new();
    h.set("SIMPLE", true)
        .comment("SIMPLE", "conforms")
        .set("BITPIX", 16)
        .set("OBJECT", "NGC4151");
    assert_eq!(h.get_logical("SIMPLE"), Some(true));
    assert_eq!(h.get_integer("BITPIX"), Some(16));
    assert_eq!(h.get_text("OBJECT"), Some("NGC4151"));
    assert_eq!(h.cards.len(), 3);

    // Re-setting a keyword replaces in place — no duplicate card, index stable.
    h.set("BITPIX", -32);
    assert_eq!(h.get_integer("BITPIX"), Some(-32));
    assert_eq!(h.cards.len(), 3);
    // The attached comment survives on its card.
    assert_eq!(h.cards[0].comment.as_deref(), Some("conforms"));
}

#[test]
fn builder_appends_commentary_cards() {
    let mut h = Header::new();
    h.set("SIMPLE", true)
        .push_comment("made by fits")
        .push_history("step 1");
    assert_eq!(h.cards.len(), 3);
    assert_eq!(h.cards[1].kind, CardKind::Commentary);
    assert_eq!(h.cards[1].keyword, "COMMENT");
    assert_eq!(h.cards[2].keyword, "HISTORY");
    // Commentary cards are not keyword-indexed.
    assert_eq!(h.get("COMMENT"), None);
}

#[test]
fn built_header_round_trips_through_render_and_parse() {
    let mut h = Header::new();
    h.set("SIMPLE", true)
        .set("BITPIX", 8)
        .set("NAXIS", 0)
        .set("OBJECT", "test");
    let bytes = crate::writer::render_header(&h);
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
    h.set("NAXIS", 1000);
    assert!(matches!(
        h.naxis(),
        Err(FitsError::WrongValueType { name: "NAXIS" })
    ));
    h.set("NAXIS", 3);
    assert_eq!(h.naxis().unwrap(), 3);
}
