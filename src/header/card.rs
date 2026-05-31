use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::value::Value;

/// What role an 80-byte record plays in a header unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CardKind {
    /// `KEYWORD = value [/ comment]` — a value indicator sits in bytes 9–10.
    Value,
    /// `COMMENT`, `HISTORY`, or the blank keyword — free text in bytes 9–80,
    /// no value indicator. The text is carried in [`Card::comment`].
    Commentary,
    /// A `CONTINUE` record carrying a long-string substring (§4.2.1.2). Produced
    /// only transiently by [`Card::parse`]; [`Header::parse`] folds each one into
    /// the preceding value card and never stores it, so a `Continue` card never
    /// reaches the writer.
    Continue,
    /// The `END` record that terminates a header unit.
    End,
}

/// One logical keyword record (§4.1).
///
/// A header is an *ordered* list of these; duplicates and order are significant,
/// so the model never collapses cards into a map.
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// Keyword name, trailing spaces stripped. Empty for the blank keyword.
    pub(crate) keyword: String,
    /// Present only for [`CardKind::Value`] cards.
    pub(crate) value: Option<Value>,
    /// The `/`-comment for value cards, or the whole free text for commentary
    /// cards. Trailing spaces are not significant and are stripped.
    pub(crate) comment: Option<String>,
    pub(crate) kind: CardKind,
}

impl Card {
    /// Parse a single 80-byte record.
    pub fn parse(raw: &[u8; CARD_SIZE]) -> Result<Card> {
        // FITS header records are restricted ASCII (§4.1). Rejecting non-ASCII up
        // front both enforces that and guarantees every fixed-column slice below
        // lands on a char boundary — a valid UTF-8 *multibyte* card would
        // otherwise panic in `text[..8]`.
        if !raw.is_ascii() {
            return Err(FitsError::InvalidValue { card: label(raw) });
        }
        let text = std::str::from_utf8(raw).expect("ASCII bytes are valid UTF-8");
        let keyword = text[..8].trim_end_matches(' ').to_string();

        if keyword == "END" {
            return Ok(Card {
                keyword,
                value: None,
                comment: None,
                kind: CardKind::End,
            });
        }
        if keyword.is_empty() || keyword == "COMMENT" || keyword == "HISTORY" {
            return Ok(Card {
                kind: CardKind::Commentary,
                comment: free_text(&text[8..]),
                value: None,
                keyword,
            });
        }
        // A CONTINUE record (no value indicator; substring quoted from byte 11)
        // carries one piece of a long string. `Header::parse` folds it into the
        // preceding value card. A malformed CONTINUE falls through to commentary.
        if keyword == "CONTINUE" {
            let split = split_value_comment(&text[10..]);
            if split.value_token.starts_with('\'') {
                let substring = parse_string(split.value_token, raw)?;
                return Ok(Card {
                    keyword,
                    value: Some(Value::Text(substring)),
                    comment: split.comment,
                    kind: CardKind::Continue,
                });
            }
        }
        if raw[8] == b'=' {
            validate_keyword(&keyword)?;
            let split = split_value_comment(&text[10..]);
            let value = parse_value(split.value_token, raw)?;
            return Ok(Card {
                keyword,
                value: Some(value),
                comment: split.comment,
                kind: CardKind::Value,
            });
        }
        // Unknown no-value card (e.g. a HIERARCH record): treat as commentary so
        // the file stays readable, matching the "plain reader" fallback.
        Ok(Card {
            kind: CardKind::Commentary,
            comment: free_text(&text[8..]),
            value: None,
            keyword,
        })
    }

    /// Serialize back to an 80-byte record. Free-format: scalar values start at
    /// column 11. A fixed-format writer (mandatory keywords right-justified) is a
    /// later concern handled by the writer layer.
    pub fn render(&self) -> [u8; CARD_SIZE] {
        let mut buf = [b' '; CARD_SIZE];
        let kw = self.keyword.as_bytes();
        let n = kw.len().min(8);
        buf[..n].copy_from_slice(&kw[..n]);

        match self.kind {
            CardKind::End => {}
            CardKind::Commentary => {
                if let Some(text) = &self.comment {
                    write_at(&mut buf, 8, text);
                }
            }
            CardKind::Value => {
                buf[8] = b'=';
                let body = format_value(self.value.as_ref().expect("value card carries a value"));
                write_at(&mut buf, 10, &body);
                if let Some(comment) = &self.comment {
                    let pos = 10 + body.len();
                    write_at(&mut buf, pos, &format!(" / {comment}"));
                }
            }
            // A lone CONTINUE record: the substring (already a string value) sits
            // at byte 11 with no value indicator. Normally folded away before the
            // writer sees it; rendered here only for a standalone round-trip.
            CardKind::Continue => {
                let s = self.value.as_ref().and_then(Value::as_text).unwrap_or("");
                write_at(&mut buf, 10, &format!("'{}'", s.replace('\'', "''")));
            }
        }
        buf
    }

    /// Serialize to one or more 80-byte records. A string value too long for a
    /// single record is split into a `CONTINUE` chain (§4.2.1.2) instead of being
    /// silently truncated; every other card renders to exactly one record.
    pub(crate) fn render_records(&self) -> Vec<[u8; CARD_SIZE]> {
        if self.kind == CardKind::Value
            && let Some(Value::Text(s)) = &self.value
        {
            // The value field spans bytes 11–80 (70 bytes). If the quoted (escaped)
            // value plus any " / comment" overflows it, emit a CONTINUE chain.
            // Short-string padding is ignored — padded strings never overflow.
            let value_len = 2 + s.replace('\'', "''").len();
            let comment_len = self.comment.as_ref().map_or(0, |c| 3 + c.len());
            if 10 + value_len + comment_len > CARD_SIZE {
                return render_long_string(&self.keyword, s, self.comment.as_deref());
            }
        }
        vec![self.render()]
    }
}

/// Result of splitting a value field into its value token and trailing comment.
struct Split<'a> {
    value_token: &'a str,
    comment: Option<String>,
}

/// Split `field` (bytes 11–80) on the first `/` that is not inside a string
/// literal, tracking the `''` escape so an embedded quote never ends the string.
fn split_value_comment(field: &str) -> Split<'_> {
    let bytes = field.as_bytes();
    let mut in_string = false;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\'' => {
                if in_string && bytes.get(i + 1) == Some(&b'\'') {
                    i += 2;
                    continue;
                }
                in_string = !in_string;
            }
            b'/' if !in_string => {
                return Split {
                    value_token: field[..i].trim(),
                    comment: comment_text(&field[i + 1..]),
                };
            }
            _ => {}
        }
        i += 1;
    }
    Split {
        value_token: field.trim(),
        comment: None,
    }
}

fn parse_value(token: &str, raw: &[u8; CARD_SIZE]) -> Result<Value> {
    let invalid = || FitsError::InvalidValue { card: label(raw) };
    if token.is_empty() {
        Ok(Value::Undefined)
    } else if token.starts_with('\'') {
        Ok(Value::Text(parse_string(token, raw)?))
    } else if token == "T" {
        Ok(Value::Logical(true))
    } else if token == "F" {
        Ok(Value::Logical(false))
    } else if token.starts_with('(') {
        parse_complex(token).ok_or_else(invalid)
    } else {
        parse_scalar(token).ok_or_else(invalid)
    }
}

/// Parse a `'...'` literal: unescape `''` → `'` and drop insignificant trailing
/// spaces (leading spaces are significant and kept).
fn parse_string(token: &str, raw: &[u8; CARD_SIZE]) -> Result<String> {
    let bytes = token.as_bytes();
    let mut out = String::new();
    let mut i = 1; // skip the opening quote
    loop {
        match bytes.get(i) {
            None => return Err(FitsError::InvalidValue { card: label(raw) }),
            Some(&b'\'') => {
                if bytes.get(i + 1) == Some(&b'\'') {
                    out.push('\'');
                    i += 2;
                } else {
                    break; // closing quote
                }
            }
            Some(&c) => {
                out.push(c as char);
                i += 1;
            }
        }
    }
    while out.ends_with(' ') {
        out.pop();
    }
    Ok(out)
}

fn parse_complex(token: &str) -> Option<Value> {
    let inner = token.strip_prefix('(')?.strip_suffix(')')?;
    let (re, im) = inner.split_once(',')?;
    match (parse_scalar(re.trim())?, parse_scalar(im.trim())?) {
        (Value::Integer(re), Value::Integer(im)) => Some(Value::ComplexInteger { re, im }),
        (re, im) => Some(Value::ComplexReal {
            re: re.as_real()?,
            im: im.as_real()?,
        }),
    }
}

fn parse_scalar(token: &str) -> Option<Value> {
    if looks_real(token) {
        parse_real(token).map(Value::Real)
    } else {
        token
            .parse::<i64>()
            .ok()
            .map(Value::Integer)
            .or_else(|| parse_real(token).map(Value::Real))
    }
}

fn looks_real(token: &str) -> bool {
    token
        .bytes()
        .any(|b| matches!(b, b'.' | b'e' | b'E' | b'd' | b'D'))
}

/// Parse a FITS real, accepting the Fortran `D`/`d` double-precision exponent.
fn parse_real(token: &str) -> Option<f64> {
    token.replace(['d', 'D'], "E").parse().ok()
}

fn validate_keyword(name: &str) -> Result<()> {
    let ok = name.len() <= 8
        && !name.is_empty()
        && name
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(FitsError::InvalidKeyword {
            name: name.to_string(),
        })
    }
}

/// Free text of a commentary card (bytes 9–80): leading spaces are content and
/// kept; only insignificant trailing spaces are stripped.
fn free_text(field: &str) -> Option<String> {
    let trimmed = field.trim_end_matches(' ');
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// The `/`-comment of a value card: the separator space is not part of the
/// comment, so both ends are trimmed to a canonical form.
fn comment_text(field: &str) -> Option<String> {
    let trimmed = field.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

/// Render a long string value as a `CONTINUE` chain (§4.2.1.2): the first record
/// holds `KEYWORD= 'sub&'`, each following one `CONTINUE  'sub&'`, and the final
/// substring drops the `&` and carries any comment. The string value is never
/// lost; only an over-long comment on the final record may be clipped.
fn render_long_string(keyword: &str, value: &str, comment: Option<&str>) -> Vec<[u8; CARD_SIZE]> {
    // Bytes 12–79 hold the quoted substring (68 chars); reserve one for the
    // continuation `&`, leaving 67 escaped characters per record.
    const PER_RECORD: usize = 67;
    let subs = split_escaped(value, PER_RECORD);
    let last = subs.len() - 1;
    subs.iter()
        .enumerate()
        .map(|(i, sub)| {
            let mut buf = [b' '; CARD_SIZE];
            if i == 0 {
                let kw = keyword.as_bytes();
                let n = kw.len().min(8);
                buf[..n].copy_from_slice(&kw[..n]);
                buf[8] = b'=';
            } else {
                buf[..8].copy_from_slice(b"CONTINUE");
            }
            let body = if i == last {
                format!("'{sub}'")
            } else {
                format!("'{sub}&'")
            };
            write_at(&mut buf, 10, &body);
            if i == last
                && let Some(c) = comment
            {
                write_at(&mut buf, 10 + body.len(), &format!(" / {c}"));
            }
            buf
        })
        .collect()
}

/// Split `value` into substrings whose *escaped* form (`'` → `''`) is at most
/// `budget` characters. Splitting on source characters keeps an escaped quote
/// pair atomic, so a `''` never straddles a record boundary.
fn split_escaped(value: &str, budget: usize) -> Vec<String> {
    let mut subs = Vec::new();
    let mut cur = String::new();
    let mut len = 0;
    for ch in value.chars() {
        let w = if ch == '\'' { 2 } else { 1 };
        if len + w > budget {
            subs.push(std::mem::take(&mut cur));
            len = 0;
        }
        if ch == '\'' {
            cur.push_str("''");
        } else {
            cur.push(ch);
        }
        len += w;
    }
    subs.push(cur); // always ≥1 substring, even for the null string
    subs
}

fn format_value(value: &Value) -> String {
    match value {
        Value::Logical(true) => "T".to_string(),
        Value::Logical(false) => "F".to_string(),
        Value::Integer(i) => i.to_string(),
        Value::Real(r) => format_real(*r),
        Value::Text(s) => format!("'{}'", pad_string(&s.replace('\'', "''"))),
        Value::ComplexInteger { re, im } => format!("({re}, {im})"),
        Value::ComplexReal { re, im } => format!("({}, {})", format_real(*re), format_real(*im)),
        Value::Undefined => String::new(),
    }
}

/// Render a real so it always reads back as [`Value::Real`] (never a bare integer).
fn format_real(r: f64) -> String {
    let s = format!("{r}");
    if looks_real(&s) || !r.is_finite() {
        s
    } else {
        format!("{s}.0")
    }
}

/// Pad a string value to the 8-character minimum many writers emit; the extra
/// trailing spaces are insignificant and parse away.
fn pad_string(s: &str) -> String {
    if s.len() >= 8 {
        s.to_string()
    } else {
        format!("{s:<8}")
    }
}

fn write_at(buf: &mut [u8; CARD_SIZE], pos: usize, text: &str) {
    let bytes = text.as_bytes();
    let end = (pos + bytes.len()).min(CARD_SIZE);
    if pos < end {
        buf[pos..end].copy_from_slice(&bytes[..end - pos]);
    }
}

fn label(raw: &[u8; CARD_SIZE]) -> String {
    String::from_utf8_lossy(raw).trim_end().to_string()
}

#[cfg(test)]
mod tests {
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
        assert_eq!(
            parse("EMPTY   = ''").value,
            Some(Value::Text(String::new()))
        );
        assert_eq!(
            parse("BLANKS  = '      '").value,
            Some(Value::Text(String::new()))
        );
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
}
