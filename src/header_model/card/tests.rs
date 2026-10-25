use crate::header_model::card::*;
use crate::header_model::{Header, RecordView};
use crate::internals::cards;
use std::assert_matches;

use num_complex::Complex;

use crate::fits_error::ValueKind;

fn parse(text: &str) -> Record {
    reparse(&cards::card_bytes(text))
}

fn reparse(bytes: &[u8; CARD_SIZE]) -> Record {
    match record(bytes) {
        ParsedRecord::Record(card) => card,
        other => panic!("expected a stored card, got {other:?}"),
    }
}

/// The record `bytes` parse to, as the first of a header.
fn record(bytes: &[u8; CARD_SIZE]) -> ParsedRecord {
    ParsedRecord::parse(bytes, 1, &mut Vec::new())
}

/// The repairs the parse of `text` makes.
fn repairs(text: &str) -> Vec<RepairKind> {
    let mut repairs = Vec::new();
    ParsedRecord::parse(&cards::card_bytes(text), 1, &mut repairs);
    repairs.into_iter().map(|repair| repair.kind).collect()
}

impl Record {
    const fn value(&self) -> Option<&Value> {
        match self {
            Self::Valued { value, .. } => Some(value),
            Self::Hierarch { value, .. } => Some(value),
            Self::Commentary { .. } | Self::Malformed { .. } => None,
        }
    }

    /// The record's name as text, `""` for none.
    fn name_text(&self) -> String {
        self.view()
            .name()
            .map_or_else(String::new, |name| name.to_string())
    }

    /// A valued record's comment, or a commentary record's text.
    fn text(&self) -> Option<&str> {
        match self {
            Self::Commentary { text, .. } => text.as_deref(),
            _ => self.comment(),
        }
    }
}

/// One record: the card's checks, then its single-record rendering.
fn render(card: &Record) -> Result<[u8; CARD_SIZE]> {
    validate_contents(card)?;
    render_one(card)
}

fn render_records(card: &Record) -> Vec<[u8; CARD_SIZE]> {
    let mut bytes = Vec::new();
    render_into(card, &mut bytes).unwrap();
    bytes.as_chunks::<CARD_SIZE>().0.to_vec()
}

/// Complex integer parts past `i64` keep their exact decimals. A header reads both
/// complex forms as a pair, the integer parts as reals, and refuses another type by name;
/// an absent one is absent, unless it is required.
#[test]
fn complex_values_keep_exact_parts_and_read_as_pairs() {
    let exact = parse("CPLXBIG = (9223372036854775808, -9223372036854775809)");
    let Some(Value::ComplexInteger { re, im }) = exact.value() else {
        panic!("large complex integer components lost their exact representation");
    };
    assert_eq!(re.to_string(), "9223372036854775808");
    assert_eq!(im.to_string(), "-9223372036854775809");
    assert_eq!(reparse(&render(&exact).unwrap()), exact);

    let header = cards::header_of(&[
        (
            "CPLXI",
            Value::ComplexInteger {
                re: 3_i64.into(),
                im: 4_i64.into(),
            },
        ),
        ("CPLXR", Value::ComplexReal { re: 1.0, im: -2.5 }),
        ("REAL", 1.5.into()),
    ]);
    assert_eq!(
        header.get_complex("CPLXI").unwrap(),
        Some(Complex::new(3.0, 4.0))
    );
    assert_eq!(
        header.require_complex("CPLXR").unwrap(),
        Complex::new(1.0, -2.5)
    );
    assert_eq!(header.get_complex("NONE").unwrap(), None);
    assert_matches!(
        header.get_complex("REAL"),
        Err(FitsError::ValueTypeMismatch {
            expected: ValueKind::Complex,
            ..
        })
    );
    assert_matches!(
        header.require_complex("NONE"),
        Err(FitsError::MissingKeyword { .. })
    );
    assert_eq!(Value::Real(1.5).as_complex().unwrap(), None);
}

#[test]
fn classifies_end_and_commentary_cards() {
    assert_eq!(record(&cards::card_bytes("END")), ParsedRecord::End);

    let comment = parse("COMMENT  this file is great");
    assert_matches!(comment, Record::Commentary { .. });
    assert_eq!(comment.name_text(), "COMMENT");
    assert_eq!(comment.text(), Some(" this file is great"));

    let history = parse("HISTORY processed 2026-05-31");
    assert_matches!(history, Record::Commentary { .. });
    assert_eq!(history.name_text(), "HISTORY");

    // Blank-keyword commentary card.
    let blank = parse("         free annotation");
    assert_matches!(blank, Record::Commentary { .. });
    assert_eq!(blank.name_text(), "");
}

#[test]
fn commentary_text_starting_with_equals_is_not_misread_as_a_value() {
    let card = parse("COMMENT = not a value indicator");
    assert_matches!(card, Record::Commentary { .. });
    assert!(card.value().is_none());
}

/// A non-ASCII byte in the name names nothing, and one in a value spoils it: both
/// keep the record malformed, with no panic in slicing a multibyte character. One in
/// a comment or commentary text reads as `?`, a repair.
#[test]
fn non_ascii_bytes_mark_or_repair_a_record_by_their_place() {
    // A valid-UTF-8 multibyte é (0xC3 0xA9) straddling the name field's end.
    let mut bytes = [b' '; CARD_SIZE];
    bytes[..7].copy_from_slice(b"OBJECT ");
    bytes[7] = 0xC3;
    bytes[8] = 0xA9;
    assert_matches!(
        reparse(&bytes),
        Record::Malformed {
            name: None,
            defect: Defect::Name,
            ..
        }
    );
    let mut in_value = cards::card_bytes("OBJECT  = 'x'");
    in_value[11] = 0xFF;
    assert_matches!(
        reparse(&in_value),
        Record::Malformed { name: Some(RecordName::Keyword(keyword)), defect: Defect::Value, .. } if keyword == "OBJECT"
    );
    let mut in_comment = cards::card_bytes("OBJECT  = 'x' / caf");
    in_comment[19] = 0xE9;
    let mut log = Vec::new();
    let ParsedRecord::Record(card) = ParsedRecord::parse(&in_comment, 7, &mut log) else {
        panic!("a card");
    };
    assert_eq!(card.text(), Some("caf?"));
    assert_eq!(
        log,
        [Repair {
            record: 7,
            kind: RepairKind::NonAsciiText
        }]
    );
    let mut in_history = cards::card_bytes("HISTORY reduced");
    in_history[15] = 0x80;
    assert_eq!(reparse(&in_history).text(), Some("reduced?"));
}

/// cfitsio and astropy read a lowercase name as its keyword, and a value after an
/// `=` placed early or with no blank after it; each is a repair.
#[test]
fn repairs_a_lowercase_name_and_a_misplaced_value_indicator() {
    assert_eq!(parse("object  = 'x'"), parse("OBJECT  = 'x'"));
    assert_eq!(repairs("object  = 'x'"), [RepairKind::LowercaseName]);
    assert_eq!(parse("OBJECT  ='M31'"), parse("OBJECT  = 'M31'"));
    assert_eq!(repairs("OBJECT  ='M31'"), [RepairKind::ValueIndicator]);
    assert_eq!(parse("EXPOS= 5"), parse("EXPOS   = 5"));
    assert_eq!(
        repairs("expos= 5 / s"),
        [RepairKind::ValueIndicator, RepairKind::LowercaseName]
    );
    assert_eq!(repairs("OBJECT  = 'M31'"), []);
    // A name with a character no keyword takes, and a value that does not parse.
    assert_matches!(
        parse("GA!N    = 1"),
        Record::Malformed {
            name: None,
            defect: Defect::Name,
            ..
        }
    );
    assert_matches!(
        parse("GAIN    = 1.2.3"),
        Record::Malformed { name: Some(RecordName::Keyword(keyword)), defect: Defect::Value, .. } if keyword == "GAIN"
    );
    assert_matches!(
        parse("OBJECT  = 'open"),
        Record::Malformed {
            defect: Defect::Value,
            ..
        }
    );
    // Only blanks may follow a closing quote: a second token is no part of the string.
    assert_matches!(
        parse("OBJECT  = 'abc' 123 / c"),
        Record::Malformed {
            defect: Defect::Value,
            ..
        }
    );
    assert_matches!(
        parse("OBJECT  = 'abc'     / c"),
        Record::Valued { value: Value::Text(text), .. } if text == "abc"
    );
}

/// `HIERARCH name = value` reads its name to the `=` and its value and comment in free
/// format, of every value type; one with no `=` stays commentary.
#[test]
fn reads_hierarch_records() {
    let name = |text: &str| text.parse::<HierarchName>().unwrap();
    for (text, value) in [
        (
            "HIERARCH ESO DET CHIP1 NAME = 'CCD-44' / detector",
            Value::Text("CCD-44".into()),
        ),
        (
            "HIERARCH ESO DET CHIP1 NX = 2048",
            Value::Integer(2048.into()),
        ),
        (
            "HIERARCH ESO TEL FOCU SCALE = 1.489 / (deg/m)",
            Value::Real(1.489),
        ),
        ("HIERARCH ESO INS OPTI-3 ON = T", Value::Logical(true)),
    ] {
        let Record::Hierarch { value: read, .. } = parse(text) else {
            panic!("{text:?} is no HIERARCH card");
        };
        assert_eq!(read, value, "{text:?}");
    }
    let card = parse("HIERARCH ESO  DET   CHIP1 NAME = 'CCD-44' / detector");
    assert_eq!(
        card,
        Record::Hierarch {
            name: name("ESO DET CHIP1 NAME"),
            value: Value::Text("CCD-44".into()),
            comment: Some("detector".into()),
        }
    );
    assert_eq!(card.name_text(), "HIERARCH ESO DET CHIP1 NAME");
    assert_matches!(
        parse("HIERARCH LONGKEYWORDNAME.X = 1"),
        Record::Hierarch { .. }
    );
    assert_matches!(
        parse("HIERARCH without a value indicator"),
        Record::Commentary { .. }
    );
    assert_eq!(repairs("hierarch ESO X = 1"), [RepairKind::LowercaseName]);
    assert_matches!(
        parse("HIERARCH  = 1"),
        Record::Malformed {
            defect: Defect::Name,
            ..
        }
    );
    assert_matches!(
        parse("HIERARCH ESO X = 1.2.3"),
        Record::Malformed {
            defect: Defect::Value,
            ..
        }
    );
}

/// A HIERARCH record keeps the blank before its `=` unless the record needs that byte,
/// as cfitsio and astropy write it, and refuses a comment that does not fit.
#[test]
fn writes_hierarch_records() {
    let card = |name: &str, value: Value, comment: Option<&str>| Record::Hierarch {
        name: name.parse().unwrap(),
        value,
        comment: comment.map(str::to_string),
    };
    let short = card(
        "ESO DET CHIP1 NAME",
        Value::Text("CCD-44".into()),
        Some("detector"),
    );
    let record = render(&short).unwrap();
    assert_eq!(
        str::from_utf8(&record).unwrap().trim_end(),
        "HIERARCH ESO DET CHIP1 NAME = 'CCD-44  ' / detector"
    );
    assert_eq!(reparse(&record), short);

    // 9 + 67 + 3 + 1 = 80 bytes keep the blank; 9 + 68 + 3 + 1 = 81 drop it.
    let name67 = format!("ESO {}", "A".repeat(63));
    let name68 = format!("ESO {}", "A".repeat(64));
    let fits = render(&card(&name67, Value::Integer(1.into()), None)).unwrap();
    assert_eq!(
        str::from_utf8(&fits).unwrap(),
        format!("HIERARCH {name67} = 1")
    );
    let tight = render(&card(&name68, Value::Integer(1.into()), None)).unwrap();
    assert_eq!(
        str::from_utf8(&tight).unwrap(),
        format!("HIERARCH {name68}= 1")
    );
    assert_eq!(
        reparse(&tight),
        card(&name68, Value::Integer(1.into()), None)
    );

    let long_comment = card("ESO X", Value::Integer(1.into()), Some(&"c".repeat(70)));
    assert_matches!(
        render(&long_comment),
        Err(FitsError::HeaderCardTooLong { name, .. }) if name == RecordName::Hierarch("ESO X".parse().unwrap())
    );
}

#[test]
fn parses_a_continue_record() {
    assert_eq!(
        record(&cards::card_bytes("CONTINUE  'ollowed by more text&'")),
        ParsedRecord::Continue {
            substring: "ollowed by more text&".into(),
            comment: None,
            tail: Some("  'ollowed by more text&'".into()),
        }
    );
}

/// A record named `END` ends the header; bytes other than blanks after the name are a
/// repair, as cfitsio and astropy read it.
#[test]
fn end_ends_the_header_and_its_stray_bytes_are_a_repair() {
    assert_eq!(record(&cards::card_bytes("END")), ParsedRecord::End);
    assert_eq!(repairs("END"), []);
    assert_eq!(
        record(&cards::card_bytes("END     =                    T")),
        ParsedRecord::End
    );
    assert_eq!(
        repairs("END     =                    T"),
        [RepairKind::EndText]
    );
    // A longer name is another keyword.
    assert_matches!(parse("ENDX    = 1"), Record::Valued { .. });
}

#[test]
fn long_string_splits_into_a_continue_chain() {
    // A value too long for one record, with embedded quotes that must not be split
    // across a record boundary.
    let value = format!("{}'q'{}", "a".repeat(60), "b".repeat(60));
    let card = Record::Valued {
        keyword: Keyword::new("LONGSTR"),
        value: Value::Text(value.clone()),
        comment: Some("trailing note".into()),
    };
    let records = render_records(&card);
    // The 123 characters escape to 125. The first record takes 67 of them, ending
    // after the doubled quotes and two b's, and the second the other 58. The
    // comment does not fit beside those, so an empty third piece carries it.
    let records: Vec<&str> = records
        .iter()
        .map(|record| str::from_utf8(record).unwrap().trim_end())
        .collect();
    assert_eq!(
        records,
        [
            format!("LONGSTR = '{}''q''bb&'", "a".repeat(60)),
            format!("CONTINUE  '{}&'", "b".repeat(58)),
            "CONTINUE  '' / trailing note".to_string(),
        ]
    );

    // The chain reassembles to the original value (comment on the last record).
    let mut with_end: Vec<u8> = render_records(&card).into_iter().flatten().collect();
    with_end.extend_from_slice(&cards::card_bytes("END"));
    let h = Header::parse(&with_end).unwrap();
    assert_eq!(h.get_text("LONGSTR").unwrap(), Some(value.as_str()));
}

#[test]
fn long_string_comment_boundary_is_lossless_or_refused() {
    let exact_comment = "c".repeat(65);
    let exact = Record::Valued {
        keyword: Keyword::new("TEXT"),
        value: Value::Text("x".into()),
        comment: Some(exact_comment.clone()),
    };
    let records = render_records(&exact);
    assert_eq!(records.len(), 2);
    assert_eq!(&records[1][..15], b"CONTINUE  '' / ");
    let mut bytes: Vec<u8> = records.iter().flatten().copied().collect();
    bytes.extend_from_slice(&cards::card_bytes("END"));
    let parsed = Header::parse(&bytes).unwrap();
    let Some(RecordView::Valued { value, comment, .. }) = parsed.iter().next() else {
        panic!("the folded card reads back as one valued record");
    };
    assert_eq!(value.as_text(), Some("x"));
    assert_eq!(comment, Some(exact_comment.as_str()));

    let overflow = Record::Valued {
        keyword: Keyword::new("TEXT"),
        value: Value::Text("x".into()),
        comment: Some("c".repeat(66)),
    };
    let mut output = vec![1, 2, 3];
    assert_matches!(
        render_into(&overflow, &mut output),
        Err(FitsError::HeaderCardTooLong {
            name,
            length: 81,
        }) if name == RecordName::Keyword(Keyword::new("TEXT"))
    );
    assert_eq!(output, vec![1, 2, 3]);
}

#[test]
fn short_string_renders_to_a_single_record() {
    let card = parse("OBJECT  = 'Cygnus X-1'");
    assert_eq!(render_records(&card).len(), 1);
}

#[test]
fn non_finite_reals_are_refused_on_read() {
    // §4.2.4 has no inf/NaN value form; Rust's float parser would accept them (and
    // an overflowing magnitude yields inf), so the reader must reject them.
    for token in ["inf", "Infinity", "nan", "-inf", "1E400"] {
        let card = format!("BADREAL = {token}");
        assert_matches!(
            parse(&card),
            Record::Malformed {
                defect: Defect::Value,
                ..
            },
            "expected {token:?} to be rejected, not parsed as a real"
        );
    }
    assert_eq!(parse("OK      = 1.5").value(), Some(&Value::Real(1.5)));
}

#[test]
fn render_refuses_a_non_finite_real() {
    let card = Record::Valued {
        keyword: Keyword::new("BAD"),
        value: Value::Real(f64::INFINITY),
        comment: None,
    };
    assert_matches!(
        render(&card),
        Err(FitsError::InvalidHeaderValue { name, reason: NonFinite::Real })
            if name == RecordName::Keyword(Keyword::new("BAD"))
    );
}

/// The value field of each kind as a render spells it, exactly: right-justified to column
/// 30 except strings, an integral real gains `.0`, an extreme real takes the `E` form, an
/// integer past `i64` keeps its exact decimal, a string pads to 8 characters once its quotes
/// are doubled, and the null string and a string of blanks keep apart.
fn canonical_fields() -> Vec<(Value, String)> {
    let right = |text: &str| format!("{text:>20}");
    let integer = |text: &str| Value::Integer(FitsInteger::parse(text).unwrap());
    let mut fields = vec![
        (Value::Logical(true), right("T")),
        (Value::Logical(false), right("F")),
        (integer("-5"), right("-5")),
        (Value::Real(5.0), right("5.0")),
        (Value::Real(-0.0), right("-0.0")),
        (Value::Real(0.1), right("0.1")),
        (Value::Real(1e300), right("1E300")),
        (Value::Real(-1e300), right("-1E300")),
        (Value::Real(1e-300), right("1E-300")),
        (Value::Real(-1e-300), right("-1E-300")),
        (Value::Real(2.5e123), right("2.5E123")),
        (
            Value::ComplexReal { re: 1.0, im: -2.5 },
            right("(1.0, -2.5)"),
        ),
        (
            Value::ComplexInteger {
                re: 3_i64.into(),
                im: (-4_i64).into(),
            },
            right("(3, -4)"),
        ),
        (Value::Text("ab".into()), "'ab      '".to_string()),
        (Value::Text("O'Brien".into()), "'O''Brien'".to_string()),
        (Value::Text("a/b/c".into()), "'a/b/c   '".to_string()),
        (Value::Text(String::new()), "''".to_string()),
        (Value::Text(" ".into()), "'        '".to_string()),
        (Value::Undefined, String::new()),
    ];
    for decimal in [
        "-9223372036854775809",
        "-9223372036854775808",
        "9223372036854775807",
        "9223372036854775808",
    ] {
        fields.push((integer(decimal), right(decimal)));
    }
    fields
}

/// A render writes each value in its canonical field, which parses back to the record.
#[test]
fn every_value_kind_renders_its_fixed_format_field() {
    for (value, field) in canonical_fields() {
        let card = Record::Valued {
            keyword: Keyword::new("KEY"),
            value: value.clone(),
            comment: None,
        };
        let record = render(&card).unwrap();
        assert_eq!(
            str::from_utf8(&record).unwrap(),
            format!("KEY     = {field:<70}"),
            "{value:?}"
        );
        assert_eq!(reparse(&record), card, "{value:?}");
    }
}

/// Each canonical field parses to its value, and so does each other spelling the standard
/// allows: a real's trailing zeros, a `D` or `E` exponent, an unpadded string, a string's
/// leading blanks, and a complex pair of an integer and a real, which widens. A slash in a
/// string is no comment boundary, and the comment after the value is kept.
#[test]
fn every_value_field_parses_to_its_value() {
    let other_spellings = [
        ("1950.00", Value::Real(1950.0)),
        ("2.0D3", Value::Real(2000.0)),
        ("3.14E2", Value::Real(314.0)),
        ("'Cygnus X-1'", Value::Text("Cygnus X-1".into())),
        ("'   keep'", Value::Text("   keep".into())),
        ("(3, -2.5)", Value::ComplexReal { re: 3.0, im: -2.5 }),
        ("(1.5, -4)", Value::ComplexReal { re: 1.5, im: -4.0 }),
    ];
    let fields = canonical_fields()
        .into_iter()
        .map(|(value, field)| (field, value))
        .chain(other_spellings.map(|(field, value)| (field.to_string(), value)));
    for (field, value) in fields {
        let card = parse(&format!("KEY     = {field} / the comment"));
        assert_eq!(card.value(), Some(&value), "{field:?}");
        assert_eq!(card.text(), Some("the comment"), "{field:?}");
    }
    let commentary = parse("COMMENT  some words here");
    assert_eq!(reparse(&render(&commentary).unwrap()), commentary);
}

/// Each substring holds at most 67 characters once quotes are doubled, and a
/// doubled quote never straddles two: 66 letters and a quote need 68, so the quote
/// opens the next substring.
#[test]
fn long_string_splits_at_67_escaped_characters() {
    let pieces = |text: &str| -> Vec<String> {
        LongString {
            lead: Lead::Keyword(Keyword::new("LONGSTR")),
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
