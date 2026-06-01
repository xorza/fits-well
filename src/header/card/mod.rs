use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::value::Value;

/// What role an 80-byte record plays in a header unit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum CardKind {
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
    /// An ESO `HIERARCH` record (registered convention): `HIERARCH a b c = value`.
    /// [`Card::keyword`] holds the space-joined hierarchical key (no `HIERARCH`
    /// prefix); it is value-indexed like a [`CardKind::Value`] card.
    Hierarch,
    /// The `END` record that terminates a header unit.
    End,
}

/// One logical keyword record (§4.1).
///
/// A header is an *ordered* list of these; duplicates and order are significant,
/// so the model never collapses cards into a map.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Card {
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
    /// A valued keyword card (`KEYWORD = value`), comment optional.
    pub(crate) fn value(keyword: &str, value: Value) -> Card {
        Card {
            keyword: keyword.to_string(),
            value: Some(value),
            comment: None,
            kind: CardKind::Value,
        }
    }

    /// A commentary card (`COMMENT`/`HISTORY`/blank keyword) carrying free text.
    pub(crate) fn commentary(keyword: &str, text: &str) -> Card {
        Card {
            keyword: keyword.to_string(),
            value: None,
            comment: Some(text.to_string()),
            kind: CardKind::Commentary,
        }
    }

    /// Parse a single 80-byte record.
    pub(crate) fn parse(raw: &[u8; CARD_SIZE]) -> Result<Card> {
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
        // A HIERARCH record: `HIERARCH key path = value`. The value indicator is
        // the first `=` (hierarchical keys never contain one). Store the
        // space-joined key without the prefix.
        if keyword == "HIERARCH"
            && let Some(eq) = text.find('=')
        {
            let key = text[8..eq].trim();
            if !key.is_empty() {
                let split = split_value_comment(&text[eq + 1..]);
                let value = parse_value(split.value_token, raw)?;
                return Ok(Card {
                    keyword: key.to_string(),
                    value: Some(value),
                    comment: split.comment,
                    kind: CardKind::Hierarch,
                });
            }
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

    /// Serialize back to an 80-byte record in fixed format (§4.2): logical,
    /// integer, real, and complex values are right-justified ending at column 30;
    /// character strings keep their opening quote at column 11.
    pub(crate) fn render(&self) -> [u8; CARD_SIZE] {
        let mut buf = [b' '; CARD_SIZE];
        // HIERARCH lays out the whole card itself ("HIERARCH key = value"); the
        // key has spaces and is not an 8-byte field, so it bypasses the layout below.
        if self.kind == CardKind::Hierarch {
            let value = format_value(self.value.as_ref().expect("HIERARCH card carries a value"));
            let body = format!("HIERARCH {} = {value}", self.keyword);
            write_at(&mut buf, 0, &body);
            if let Some(comment) = &self.comment {
                write_at(&mut buf, body.len(), &format!(" / {comment}"));
            }
            return buf;
        }
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
                let value = self.value.as_ref().expect("value card carries a value");
                let body = format_value(value);
                // Fixed format (§4.2.3–4.2.4): logical/integer/real/complex values
                // are right-justified ending at column 30; character strings keep
                // their opening quote at column 11 (left-justified). astropy and
                // cfitsio both warn on a non-fixed-format mandatory keyword.
                let end = match value {
                    Value::Text(_) | Value::Undefined => {
                        write_at(&mut buf, 10, &body);
                        10 + body.len()
                    }
                    _ => {
                        let end = (10 + body.len()).max(30);
                        write_at(&mut buf, end - body.len(), &body);
                        end
                    }
                };
                if let Some(comment) = &self.comment {
                    write_at(&mut buf, end, &format!(" / {comment}"));
                }
            }
            // A lone CONTINUE record: the substring (already a string value) sits
            // at byte 11 with no value indicator. Normally folded away before the
            // writer sees it; rendered here only for a standalone round-trip.
            CardKind::Continue => {
                let s = self.value.as_ref().and_then(Value::as_text).unwrap_or("");
                write_at(&mut buf, 10, &format!("'{}'", s.replace('\'', "''")));
            }
            CardKind::Hierarch => unreachable!("HIERARCH is rendered before this match"),
        }
        buf
    }

    /// Serialize to one or more 80-byte records. A string value too long for a
    /// single record is split into a `CONTINUE` chain (§4.2.1.2) instead of being
    /// silently truncated; every other card renders to exactly one record.
    pub(crate) fn render_records(&self) -> Vec<[u8; CARD_SIZE]> {
        // A string value that overflows one record emits a CONTINUE chain (§4.2.1.2)
        // instead of being truncated — for both plain value cards (value field at
        // byte 11) and HIERARCH cards (whose `HIERARCH key = ` prefix is longer).
        if let Some(Value::Text(s)) = &self.value {
            let value_len = 2 + s.len() + s.bytes().filter(|&b| b == b'\'').count();
            let comment_len = self.comment.as_ref().map_or(0, |c| 3 + c.len());
            let prefix_len = match self.kind {
                CardKind::Value => 10,
                CardKind::Hierarch => "HIERARCH ".len() + self.keyword.len() + " = ".len(),
                _ => usize::MAX, // other kinds never use the chain
            };
            if prefix_len != usize::MAX && prefix_len + value_len + comment_len > CARD_SIZE {
                let hierarch = self.kind == CardKind::Hierarch;
                return render_long_string(&self.keyword, s, self.comment.as_deref(), hierarch);
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
    let had_content = !out.is_empty();
    while out.ends_with(' ') {
        out.pop();
    }
    // §4.2.1.1: trailing blanks are insignificant, but an all-blank (non-null)
    // string keeps one significant space — that single space is what distinguishes
    // `'   '` (empty string, length 1) from `''` (null string, length 0).
    if out.is_empty() && had_content {
        out.push(' ');
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
/// Non-finite results (`inf`/`NaN`, which Rust's parser accepts and which an
/// overflowing magnitude produces) are rejected — §4.2.4 has no such value form.
fn parse_real(token: &str) -> Option<f64> {
    // Only the Fortran `D`/`d` exponent needs rewriting; skip the allocation for
    // the common `E`/`e`/plain decimal forms.
    let parsed = if token.bytes().any(|b| b == b'd' || b == b'D') {
        token.replace(['d', 'D'], "E").parse::<f64>()
    } else {
        token.parse::<f64>()
    };
    parsed.ok().filter(|v| v.is_finite())
}

pub(crate) fn validate_keyword(name: &str) -> Result<()> {
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
fn render_long_string(
    keyword: &str,
    value: &str,
    comment: Option<&str>,
    hierarch: bool,
) -> Vec<[u8; CARD_SIZE]> {
    // Bytes 12–79 hold the quoted substring (68 chars); reserve one for the
    // continuation `&`, leaving 67 escaped characters per continuation record.
    const PER_RECORD: usize = 67;
    // A HIERARCH first record is `HIERARCH key = '…&'`, so its prefix shrinks the
    // budget for that record's substring; otherwise the first record matches the
    // 67-char continuation budget.
    let prefix = hierarch.then(|| format!("HIERARCH {keyword} = "));
    let first_budget = prefix
        .as_ref()
        .map_or(PER_RECORD, |p| CARD_SIZE.saturating_sub(p.len() + 3));
    let subs = split_escaped(value, first_budget, PER_RECORD);
    let last = subs.len() - 1;
    subs.iter()
        .enumerate()
        .map(|(i, sub)| {
            let mut buf = [b' '; CARD_SIZE];
            let body = if i == last {
                format!("'{sub}'")
            } else {
                format!("'{sub}&'")
            };
            let body_start = match (i, &prefix) {
                (0, Some(p)) => {
                    write_at(&mut buf, 0, p);
                    p.len()
                }
                (0, None) => {
                    let kw = keyword.as_bytes();
                    let n = kw.len().min(8);
                    buf[..n].copy_from_slice(&kw[..n]);
                    buf[8] = b'=';
                    10
                }
                _ => {
                    buf[..8].copy_from_slice(b"CONTINUE");
                    10
                }
            };
            write_at(&mut buf, body_start, &body);
            if i == last
                && let Some(c) = comment
            {
                write_at(&mut buf, body_start + body.len(), &format!(" / {c}"));
            }
            buf
        })
        .collect()
}

/// Split `value` into substrings whose *escaped* form (`'` → `''`) is at most
/// `budget` characters. Splitting on source characters keeps an escaped quote
/// pair atomic, so a `''` never straddles a record boundary.
fn split_escaped(value: &str, first_budget: usize, rest_budget: usize) -> Vec<String> {
    let mut subs = Vec::new();
    let mut cur = String::new();
    let mut len = 0;
    for ch in value.chars() {
        let w = if ch == '\'' { 2 } else { 1 };
        // The first record may hold fewer chars (a HIERARCH prefix eats into it).
        let budget = if subs.is_empty() {
            first_budget
        } else {
            rest_budget
        };
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
    assert!(
        r.is_finite(),
        "FITS keyword reals must be finite — §4.2.4 has no inf/NaN value form (got {r})"
    );
    // Rust's `Display` never uses exponent notation, so an extreme magnitude (e.g.
    // `1e300`) balloons to hundreds of digits and overflows the value field. Fall
    // back to the §4.2.4 uppercase-`E` exponent form, which always fits, when the
    // plain decimal grows long.
    let plain = format!("{r}");
    let s = if plain.len() > 20 && format!("{r:E}").len() < plain.len() {
        format!("{r:E}")
    } else {
        plain
    };
    if looks_real(&s) { s } else { format!("{s}.0") }
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
mod tests;
