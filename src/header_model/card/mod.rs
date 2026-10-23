use std::fmt;
use std::fmt::Write as _;

use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header_model::defect::Defect;
use crate::header_model::hierarch_name::HierarchName;
use crate::header_model::repair::{Repair, RepairKind};
use crate::header_model::value::FitsInteger;
use crate::header_model::value::Value;
use crate::keyword::Keyword;
use std::borrow::Cow;
use std::iter;
use std::ops::Range;
use std::str;

/// One stored logical keyword record (§4.1).
///
/// A header is an *ordered* list of these; duplicates and order are significant,
/// so the model never collapses cards into a map. Keywords have trailing spaces
/// stripped; the blank keyword is empty.
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Card {
    /// `KEYWORD = value [/ comment]` — a value indicator sits in bytes 9–10.
    Value {
        keyword: Keyword,
        value: Value,
        /// Trailing spaces are not significant and are stripped.
        comment: Option<String>,
    },
    /// `COMMENT`, `HISTORY`, the blank keyword, or any other record without a
    /// value indicator — free text in bytes 9–80.
    Commentary {
        keyword: String,
        /// Leading spaces are content; trailing spaces are stripped.
        text: Option<String>,
    },
    /// `HIERARCH name = value [/ comment]` (ESO convention).
    Hierarch {
        name: HierarchName,
        value: Value,
        comment: Option<String>,
    },
    /// A record that does not parse, kept as its 80 bytes, with its keyword when the
    /// name is one.
    Malformed {
        raw: Box<[u8; CARD_SIZE]>,
        keyword: Option<Keyword>,
        defect: Defect,
    },
}

/// What one 80-byte record parses to.
#[derive(Debug, Clone, PartialEq)]
pub(super) enum ParsedRecord {
    Card(Card),
    /// A `CONTINUE` record carrying a long-string substring (§4.2.1.2), which
    /// [`Header::parse`](crate::header_model::Header::parse) folds into the
    /// preceding value card.
    Continue {
        substring: String,
        comment: Option<String>,
    },
    /// The `END` record that terminates a header unit.
    End,
}

impl ParsedRecord {
    /// Parse the 80-byte record at 1-based position `record`, pushing each repair it
    /// makes to `repairs`. A record that does not parse is a [`Card::Malformed`], so a
    /// parse never fails.
    pub(super) fn parse(raw: &[u8; CARD_SIZE], record: usize, repairs: &mut Vec<Repair>) -> Self {
        let mut repair = |kind| repairs.push(Repair { record, kind });
        if is_end_record(raw) {
            return Self::End;
        }
        let malformed = |keyword, defect| {
            Self::Card(Card::Malformed {
                raw: Box::new(*raw),
                keyword,
                defect,
            })
        };
        // FITS header records are restricted ASCII (§4.1). A name with another byte
        // names nothing; elsewhere each such byte reads as `?`, so every fixed-column
        // slice below lands on a char boundary, and its place decides whether it is a
        // repaired comment or a malformed value.
        if !raw[..8].is_ascii() {
            return malformed(None, Defect::Name);
        }
        let text = ascii_text(raw);
        let non_ascii_in = |range: Range<usize>| !raw[range].is_ascii();
        let keyword = text[..8].trim_end_matches(' ');
        if keyword == "END" {
            return malformed(None, Defect::Name);
        }
        let mut commentary = || {
            if non_ascii_in(8..CARD_SIZE) {
                repair(RepairKind::NonAsciiText);
            }
            Self::Card(Card::Commentary {
                keyword: keyword.to_string(),
                text: free_text(&text[8..]),
            })
        };
        if keyword.is_empty() || keyword == "COMMENT" || keyword == "HISTORY" {
            return commentary();
        }
        // A CONTINUE record (no value indicator; substring quoted from byte 11)
        // carries one piece of a long string. A malformed CONTINUE reads as
        // commentary.
        if keyword == "CONTINUE" {
            let split = split_value_comment(&text[10..]);
            if &raw[8..10] == b"  " && split.value_token.starts_with('\'') {
                let value_end = 10 + split.comment_start.unwrap_or(CARD_SIZE - 10);
                let substring = parse_string(split.value_token);
                let Some(substring) = substring.filter(|_| !non_ascii_in(10..value_end)) else {
                    return malformed(None, Defect::Value);
                };
                if non_ascii_in(value_end..CARD_SIZE) {
                    repair(RepairKind::NonAsciiText);
                }
                return Self::Continue {
                    substring,
                    comment: split.comment,
                };
            }
            return commentary();
        }
        // `HIERARCH name = value` (ESO convention): the name runs from byte 10 to the
        // `=`, and the value and comment follow in free format.
        if text[..9].eq_ignore_ascii_case("HIERARCH ")
            && let Some(at) = text[9..].find('=')
        {
            let indicator = 9 + at;
            if &text[..8] != "HIERARCH" {
                repair(RepairKind::LowercaseName);
            }
            let name = text[9..indicator].parse::<HierarchName>().ok();
            let Some(name) = name.filter(|_| !non_ascii_in(9..indicator)) else {
                return malformed(None, Defect::Name);
            };
            let value_start = indicator + 1;
            let split = split_value_comment(&text[value_start..]);
            let value_end = value_start + split.comment_start.unwrap_or(CARD_SIZE - value_start);
            let value =
                parse_value(split.value_token).filter(|_| !non_ascii_in(value_start..value_end));
            let Some(value) = value else {
                return malformed(None, Defect::Value);
            };
            if non_ascii_in(value_end..CARD_SIZE) {
                repair(RepairKind::NonAsciiText);
            }
            return Self::Card(Card::Hierarch {
                name,
                value,
                comment: split.comment,
            });
        }
        // The value indicator belongs in byte 9, followed by a blank (§4.2.1); cfitsio
        // and astropy also read an `=` earlier in the name field, or one with no blank
        // after it.
        let Some(indicator) = raw[..9].iter().position(|&byte| byte == b'=') else {
            // Unknown no-value cards remain readable as commentary.
            return commentary();
        };
        let value_start = if indicator == 8 && raw[9] == b' ' {
            10
        } else {
            repair(RepairKind::ValueIndicator);
            indicator + 1
        };
        let mut name = Cow::Borrowed(text[..indicator].trim_end_matches(' '));
        if name.bytes().any(|byte| byte.is_ascii_lowercase()) {
            repair(RepairKind::LowercaseName);
            name.to_mut().make_ascii_uppercase();
        }
        let Ok(keyword) = Keyword::try_from(&*name) else {
            return malformed(None, Defect::Name);
        };
        let split = split_value_comment(&text[value_start..]);
        let value_end = value_start + split.comment_start.unwrap_or(CARD_SIZE - value_start);
        if non_ascii_in(value_start..value_end) {
            return malformed(Some(keyword), Defect::Value);
        }
        let Some(value) = parse_value(split.value_token) else {
            return malformed(Some(keyword), Defect::Value);
        };
        if non_ascii_in(value_end..CARD_SIZE) {
            repair(RepairKind::NonAsciiText);
        }
        Self::Card(Card::Value {
            keyword,
            value,
            comment: split.comment,
        })
    }
}

/// The record's text, each non-ASCII byte read as `?`; borrowed when it is ASCII.
fn ascii_text(raw: &[u8; CARD_SIZE]) -> Cow<'_, str> {
    match str::from_utf8(raw) {
        Ok(text) if raw.is_ascii() => Cow::Borrowed(text),
        _ => Cow::Owned(
            raw.iter()
                .map(|&byte| {
                    if byte.is_ascii() {
                        char::from(byte)
                    } else {
                        '?'
                    }
                })
                .collect(),
        ),
    }
}

impl Card {
    /// A commentary card (`COMMENT`/`HISTORY`/blank keyword) carrying free text; an
    /// empty `text` is a physically blank record.
    pub(super) fn commentary(keyword: &str, text: &str) -> Self {
        Self::Commentary {
            keyword: keyword.to_string(),
            text: (!text.is_empty()).then(|| text.to_string()),
        }
    }

    /// The record's name: a valued record's keyword, a commentary record's name, or a
    /// malformed record's keyword, or `""` when its name is none.
    pub(crate) fn keyword(&self) -> &str {
        match self {
            Self::Value { keyword, .. } => keyword.as_str(),
            Self::Commentary { keyword, .. } => keyword,
            Self::Hierarch { .. } => "HIERARCH",
            Self::Malformed { keyword, .. } => keyword.as_ref().map_or("", Keyword::as_str),
        }
    }

    /// The keyword the header's index finds this card by: a valued card's, or a
    /// malformed card's, whose lookup then fails.
    pub(crate) const fn indexed_keyword(&self) -> Option<Keyword> {
        match self {
            Self::Value { keyword, .. } => Some(*keyword),
            Self::Malformed { keyword, .. } => *keyword,
            Self::Commentary { .. } | Self::Hierarch { .. } => None,
        }
    }

    /// A valued card's value; [`FitsError::InvalidValue`] for a malformed card.
    pub(crate) fn value_or_defect(&self) -> Result<&Value> {
        match self {
            Self::Value { value, .. } => Ok(value),
            Self::Malformed { raw, .. } => Err(FitsError::InvalidValue { card: label(raw) }),
            Self::Commentary { .. } | Self::Hierarch { .. } => {
                unreachable!("the keyword index holds valued and malformed cards only")
            }
        }
    }

    /// The inline `/`-comment of a value card, or the free text of a commentary
    /// card.
    pub(crate) fn comment(&self) -> Option<&str> {
        match self {
            Self::Value { comment, .. } | Self::Hierarch { comment, .. } => comment.as_deref(),
            Self::Commentary { text, .. } => text.as_deref(),
            Self::Malformed { .. } => None,
        }
    }

    /// Append one or more 80-byte records to `out`. A string value too long for a
    /// single record is split into a `CONTINUE` chain (§4.2.1.2) instead of being
    /// silently truncated; every other card renders to exactly one record.
    pub(crate) fn render_into(&self, out: &mut Vec<u8>) -> Result<()> {
        self.validate_contents()?;
        if let Some(chain) = self.continuation_chain() {
            let start = out.len();
            let result = chain.render_into(out);
            if result.is_err() {
                out.truncate(start);
            }
            result
        } else {
            out.extend_from_slice(&self.render_one()?);
            Ok(())
        }
    }

    /// A card is valid exactly when it renders. This runs the same checks without
    /// a heap allocation: a single record renders into a stack array, and a
    /// `CONTINUE` chain is valid when its final record holds the comment.
    pub(super) fn validate(&self) -> Result<()> {
        self.validate_contents()?;
        match self.continuation_chain() {
            Some(chain) => chain.validate(),
            None => self.render_one().map(|_| ()),
        }
    }

    /// Serialize to one 80-byte record in fixed format (§4.2): logical, integer,
    /// real, and complex values are right-justified ending at column 30; character
    /// strings keep their opening quote at column 11.
    fn render_one(&self) -> Result<[u8; CARD_SIZE]> {
        if let Self::Hierarch {
            name,
            value,
            comment,
        } = self
        {
            return render_hierarch(name, value, comment.as_deref());
        }
        let mut record = RecordBuf::new(self.keyword());
        match self {
            Self::Commentary { text, .. } => {
                if let Some(text) = text {
                    record.write_str_at(8, text)?;
                }
            }
            Self::Malformed { .. } => unreachable!("validate_contents refuses a malformed card"),
            Self::Hierarch { .. } => unreachable!("rendered above"),
            Self::Value { value, comment, .. } => {
                record.bytes[8] = b'=';
                // astropy and cfitsio both warn on a non-fixed-format mandatory
                // keyword (§4.2.3–4.2.4).
                let end = match value {
                    Value::Text(_) | Value::Undefined => record.write_value_at(10, value)?,
                    _ => {
                        let len = rendered_len(value);
                        let end = (10 + len).max(30);
                        record.write_value_at(end - len, value)?
                    }
                };
                if let Some(comment) = comment {
                    let end = record.write_str_at(end, " / ")?;
                    record.write_str_at(end, comment)?;
                }
            }
        }
        Ok(record.bytes)
    }

    /// The long text value that cannot fit one 80-byte record and so must be
    /// emitted as a `CONTINUE` chain (§4.2.1.2).
    fn continuation_chain(&self) -> Option<LongString<'_>> {
        let (lead, text, comment) = match self {
            Self::Value {
                keyword,
                value: Value::Text(text),
                comment,
            } => (Lead::Keyword(keyword.as_str()), text, comment),
            Self::Hierarch {
                name,
                value: Value::Text(text),
                comment,
            } => (Lead::Hierarch(name), text, comment),
            _ => return None,
        };
        // The rendered value is the quoted text with each embedded quote doubled; the
        // comment adds its ` / ` separator. A HIERARCH record first drops its blank
        // before the `=`.
        let value_len = 2 + escaped_len(text);
        let comment_len = comment.as_ref().map_or(0, |c| 3 + c.len());
        let tightest = match lead {
            Lead::Keyword(_) => lead.value_start(),
            Lead::Hierarch(_) => lead.value_start() - 1,
        };
        (tightest + value_len + comment_len > CARD_SIZE).then_some(LongString {
            lead,
            text,
            comment: comment.as_deref(),
        })
    }

    fn validate_contents(&self) -> Result<()> {
        if let Self::Malformed { raw, .. } = self {
            return Err(FitsError::InvalidValue { card: label(raw) });
        }
        if let Some(comment) = self.comment() {
            validate_ascii(comment, "header comment")?;
        }
        let (keyword, value) = match self {
            Self::Value { keyword, value, .. } => {
                validate_valued_keyword(*keyword)?;
                (*keyword, value)
            }
            Self::Hierarch { value, .. } => (Keyword::new("HIERARCH"), value),
            Self::Commentary { .. } | Self::Malformed { .. } => return Ok(()),
        };
        match value {
            Value::Text(text) => validate_ascii(text, "header text value"),
            Value::Real(value) if !value.is_finite() => Err(FitsError::InvalidHeaderValue {
                keyword,
                reason: "real values must be finite",
            }),
            Value::ComplexReal { re, im } if !re.is_finite() || !im.is_finite() => {
                Err(FitsError::InvalidHeaderValue {
                    keyword,
                    reason: "complex real components must be finite",
                })
            }
            _ => Ok(()),
        }
    }
}

/// Result of splitting a value field into its value token and trailing comment.
#[derive(Debug)]
struct Split<'a> {
    value_token: &'a str,
    comment: Option<String>,
    /// Where the comment's `/` sits in the field; `None` for no comment.
    comment_start: Option<usize>,
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
                    comment_start: Some(i),
                };
            }
            _ => {}
        }
        i += 1;
    }
    Split {
        value_token: field.trim(),
        comment: None,
        comment_start: None,
    }
}

/// The value a value token spells; `None` for one that spells none.
fn parse_value(token: &str) -> Option<Value> {
    if token.is_empty() {
        Some(Value::Undefined)
    } else if token.starts_with('\'') {
        parse_string(token).map(Value::Text)
    } else if token == "T" {
        Some(Value::Logical(true))
    } else if token == "F" {
        Some(Value::Logical(false))
    } else if token.starts_with('(') {
        parse_complex(token)
    } else {
        parse_scalar(token)
    }
}

/// Parse a `'...'` literal: unescape `''` → `'` and drop insignificant trailing
/// spaces (leading spaces are significant and kept); `None` for an unterminated one.
fn parse_string(token: &str) -> Option<String> {
    let bytes = token.as_bytes();
    let mut out = String::new();
    let mut i = 1; // skip the opening quote
    loop {
        match bytes.get(i) {
            None => return None,
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
    Some(out)
}

/// A `(re, im)` complex value; `None` for a token of another form. An integer part
/// beside a real one widens to `f64`.
fn parse_complex(token: &str) -> Option<Value> {
    let inner = token.strip_prefix('(')?.strip_suffix(')')?;
    let (re, im) = inner.split_once(',')?;
    let real = |part: &Value| {
        part.as_real()
            .expect("a value field of 70 bytes holds no integer past the range of f64")
            .expect("a scalar is a real or an integer")
    };
    match (parse_scalar(re.trim())?, parse_scalar(im.trim())?) {
        (Value::Integer(re), Value::Integer(im)) => Some(Value::ComplexInteger { re, im }),
        (re, im) => Some(Value::ComplexReal {
            re: real(&re),
            im: real(&im),
        }),
    }
}

fn parse_scalar(token: &str) -> Option<Value> {
    if looks_real(token) {
        parse_real(token).map(Value::Real)
    } else {
        FitsInteger::parse(token)
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

/// Errors when `keyword` is a control or commentary keyword, which a valued card
/// cannot carry.
pub(super) fn validate_valued_keyword(keyword: Keyword) -> Result<()> {
    if matches!(keyword.as_str(), "END" | "CONTINUE" | "COMMENT" | "HISTORY") {
        Err(FitsError::ReservedKeyword { keyword })
    } else {
        Ok(())
    }
}

pub(crate) fn validate_ascii(text: &str, context: &'static str) -> Result<()> {
    if text.bytes().all(|byte| (0x20..=0x7e).contains(&byte)) {
        Ok(())
    } else {
        Err(FitsError::InvalidAscii { context })
    }
}

pub(crate) fn is_end_record(raw: &[u8]) -> bool {
    raw.len() == CARD_SIZE && &raw[..3] == b"END" && raw[3..].iter().all(|&byte| byte == b' ')
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

/// One `HIERARCH name = value [/ comment]` record. The blank before the `=` goes only
/// when the record needs that byte, as cfitsio and astropy write it.
fn render_hierarch(
    name: &HierarchName,
    value: &Value,
    comment: Option<&str>,
) -> Result<[u8; CARD_SIZE]> {
    let label = Lead::Hierarch(name).label();
    let mut record = RecordBuf::new(&label);
    let comment_len = comment.map_or(0, |comment| 3 + comment.len());
    let spaced = Lead::Hierarch(name).value_start();
    let indicator = if spaced + rendered_len(value) + comment_len <= CARD_SIZE {
        " = "
    } else {
        "= "
    };
    let end = record.write_str_at(9, name.as_str())?;
    let end = record.write_str_at(end, indicator)?;
    let end = record.write_value_at(end, value)?;
    if let Some(comment) = comment {
        let end = record.write_str_at(end, " / ")?;
        record.write_str_at(end, comment)?;
    }
    Ok(record.bytes)
}

/// What a long string's first record holds before its value.
#[derive(Debug, Clone, Copy)]
enum Lead<'a> {
    /// `KEYWORD = ` in bytes 1–10.
    Keyword(&'a str),
    /// `HIERARCH name = `.
    Hierarch(&'a HierarchName),
}

impl Lead<'_> {
    /// Where the value starts in the first record.
    fn value_start(self) -> usize {
        match self {
            Self::Keyword(_) => 10,
            Self::Hierarch(name) => 9 + name.as_str().len() + 3,
        }
    }

    /// The name an error reports.
    fn label(self) -> String {
        match self {
            Self::Keyword(keyword) => keyword.to_string(),
            Self::Hierarch(name) => format!("HIERARCH {name}"),
        }
    }
}

/// A text value rendered as a `CONTINUE` chain (§4.2.1.2): the first record holds
/// `KEYWORD= 'sub&'` (or `HIERARCH name = 'sub&'`), each following one
/// `CONTINUE  'sub&'`, and the final substring drops the `&` and carries any comment.
/// The value is never lost; an over-long comment is rejected instead of clipped.
#[derive(Debug, Clone, Copy)]
struct LongString<'a> {
    lead: Lead<'a>,
    text: &'a str,
    comment: Option<&'a str>,
}

impl<'a> LongString<'a> {
    /// Bytes 12–79 of a `CONTINUE` record hold the quoted substring (68 characters);
    /// one goes to the continuation `&`, leaving 67 escaped characters per record.
    const PER_RECORD: usize = 67;

    /// The escaped characters the first record holds: its bytes after the lead, less
    /// the two quotes and the `&`.
    fn first_width(self) -> usize {
        CARD_SIZE.saturating_sub(self.lead.value_start() + 3)
    }

    /// The substrings of the text, in order, each at most its record's width once its
    /// quotes are doubled. A doubled quote never straddles two records, and the null
    /// string is one empty substring.
    fn pieces(self) -> impl Iterator<Item = &'a str> {
        let text = self.text;
        let mut start = Some(0);
        let mut limit = self.first_width();
        iter::from_fn(move || {
            let from = start?;
            let mut width = 0;
            let mut end = from;
            for byte in text[from..].bytes() {
                let byte_width = if byte == b'\'' { 2 } else { 1 };
                if width + byte_width > limit {
                    break;
                }
                width += byte_width;
                end += 1;
            }
            limit = LongString::PER_RECORD;
            start = (end < text.len()).then_some(end);
            Some(&text[from..end])
        })
    }

    /// Whether the comment needs a record of its own: when it does not fit after
    /// the last substring. Errors when it fits no record at all, or when the lead
    /// leaves the first record no room for a substring.
    fn comment_needs_own_record(self) -> Result<bool> {
        if self.first_width() == 0 {
            return Err(FitsError::HeaderCardTooLong {
                keyword: self.lead.label(),
                length: self.lead.value_start() + 3,
            });
        }
        let Some(comment) = self.comment else {
            return Ok(false);
        };
        let mut pieces = self.pieces();
        let first = pieces.next().expect("one substring");
        let (start, last) = match pieces.last() {
            Some(last) => (10, last),
            None => (self.lead.value_start(), first),
        };
        if start + 2 + escaped_len(last) + 3 + comment.len() <= CARD_SIZE {
            return Ok(false);
        }
        let alone = 10 + 2 + 3 + comment.len();
        if alone > CARD_SIZE {
            return Err(FitsError::HeaderCardTooLong {
                keyword: self.lead.label(),
                length: alone,
            });
        }
        Ok(true)
    }

    fn validate(self) -> Result<()> {
        self.comment_needs_own_record().map(|_| ())
    }

    fn render_into(self, out: &mut Vec<u8>) -> Result<()> {
        let own_record = self.comment_needs_own_record()?;
        let label = self.lead.label();
        let mut pieces = self.pieces().chain(own_record.then_some("")).peekable();
        let mut first = true;
        while let Some(piece) = pieces.next() {
            let (mut record, pos) = if first {
                let mut record = RecordBuf::new(&label);
                let pos = match self.lead {
                    Lead::Keyword(_) => {
                        record.bytes[8] = b'=';
                        10
                    }
                    Lead::Hierarch(name) => {
                        let end = record.write_str_at(9, name.as_str())?;
                        record.write_str_at(end, " = ")?
                    }
                };
                (record, pos)
            } else {
                (RecordBuf::new("CONTINUE"), 10)
            };
            first = false;
            let last = pieces.peek().is_none();
            let mut cursor = Cursor {
                record: &mut record,
                pos,
            };
            let written = write_escaped(&mut cursor, piece, if last { "'" } else { "&'" });
            let end = cursor.pos;
            written.expect("a substring fits its record");
            if last && let Some(comment) = self.comment {
                let end = record.write_str_at(end, " / ")?;
                record.write_str_at(end, comment)?;
            }
            out.extend_from_slice(&record.bytes);
        }
        Ok(())
    }
}

/// An 80-byte record under construction, blank-filled with the keyword in bytes
/// 1–8.
#[derive(Debug)]
struct RecordBuf<'k> {
    bytes: [u8; CARD_SIZE],
    keyword: &'k str,
}

impl<'k> RecordBuf<'k> {
    fn new(keyword: &'k str) -> Self {
        let mut bytes = [b' '; CARD_SIZE];
        let name = keyword.as_bytes();
        let n = name.len().min(8);
        bytes[..n].copy_from_slice(&name[..n]);
        RecordBuf { bytes, keyword }
    }

    fn too_long(&self, length: usize) -> FitsError {
        FitsError::HeaderCardTooLong {
            keyword: self.keyword.to_string(),
            length,
        }
    }

    /// Write `text` from byte `pos`, returning where it ends. Errors when it would
    /// run past the record.
    fn write_str_at(&mut self, pos: usize, text: &str) -> Result<usize> {
        let end = pos + text.len();
        if end > CARD_SIZE {
            return Err(self.too_long(end));
        }
        self.bytes[pos..end].copy_from_slice(text.as_bytes());
        Ok(end)
    }

    /// Write the fixed-format text of `value` from byte `pos`, returning where it
    /// ends.
    fn write_value_at(&mut self, pos: usize, value: &Value) -> Result<usize> {
        let mut cursor = Cursor { record: self, pos };
        if write_value(&mut cursor, value).is_err() {
            return Err(self.too_long(pos + rendered_len(value)));
        }
        Ok(cursor.pos)
    }
}

/// A [`fmt::Write`] sink over a record from a byte position. Running past the
/// record is a [`fmt::Error`].
#[derive(Debug)]
struct Cursor<'r, 'k> {
    record: &'r mut RecordBuf<'k>,
    pos: usize,
}

impl fmt::Write for Cursor<'_, '_> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.pos + text.len();
        if end > CARD_SIZE {
            return Err(fmt::Error);
        }
        self.record.bytes[self.pos..end].copy_from_slice(text.as_bytes());
        self.pos = end;
        Ok(())
    }
}

/// A [`fmt::Write`] sink that only counts the bytes written through it.
#[derive(Debug, Default)]
struct Counter(usize);

impl fmt::Write for Counter {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        self.0 += text.len();
        Ok(())
    }
}

fn counted_len(args: fmt::Arguments<'_>) -> usize {
    let mut counter = Counter::default();
    counter.write_fmt(args).expect("counting never fails");
    counter.0
}

fn rendered_len(value: &Value) -> usize {
    let mut counter = Counter::default();
    write_value(&mut counter, value).expect("counting never fails");
    counter.0
}

/// The fixed-format text of `value` (§4.2).
fn write_value(out: &mut impl fmt::Write, value: &Value) -> fmt::Result {
    match value {
        Value::Logical(true) => out.write_str("T"),
        Value::Logical(false) => out.write_str("F"),
        Value::Integer(integer) => write!(out, "{integer}"),
        Value::Real(real) => write_real(out, *real),
        Value::Text(text) => {
            // Many writers pad a string to 8 characters; the padding parses away.
            let padding = 8usize.saturating_sub(escaped_len(text));
            write_escaped(out, text, "")?;
            for _ in 0..padding {
                out.write_char(' ')?;
            }
            out.write_char('\'')
        }
        Value::ComplexInteger { re, im } => write!(out, "({re}, {im})"),
        Value::ComplexReal { re, im } => {
            out.write_char('(')?;
            write_real(out, *re)?;
            out.write_str(", ")?;
            write_real(out, *im)?;
            out.write_char(')')
        }
        Value::Undefined => Ok(()),
    }
}

/// `'`, then `text` with each quote doubled, then `close`.
fn write_escaped(out: &mut impl fmt::Write, text: &str, close: &str) -> fmt::Result {
    out.write_char('\'')?;
    for (i, part) in text.split('\'').enumerate() {
        if i > 0 {
            out.write_str("''")?;
        }
        out.write_str(part)?;
    }
    out.write_str(close)
}

/// The length of `text` once each quote is doubled.
fn escaped_len(text: &str) -> usize {
    text.len() + text.bytes().filter(|&byte| byte == b'\'').count()
}

/// A real that always reads back as [`Value::Real`], never a bare integer.
/// `Display` never uses an exponent, so an extreme magnitude (`1e300`) would
/// balloon to hundreds of digits and overflow the value field; the §4.2.4
/// uppercase-`E` form takes over when the plain decimal passes 20 characters and
/// the exponent form is shorter. A plain decimal shows a point exactly when the
/// value has a fraction, so an integral one gains `.0`.
fn write_real(out: &mut impl fmt::Write, real: f64) -> fmt::Result {
    debug_assert!(real.is_finite());
    let plain = counted_len(format_args!("{real}"));
    if plain > 20 && counted_len(format_args!("{real:E}")) < plain {
        return write!(out, "{real:E}");
    }
    write!(out, "{real}")?;
    if real.fract() == 0.0 {
        out.write_str(".0")?;
    }
    Ok(())
}

fn label(raw: &[u8; CARD_SIZE]) -> String {
    String::from_utf8_lossy(raw).trim_end().to_string()
}

#[cfg(test)]
mod tests;
