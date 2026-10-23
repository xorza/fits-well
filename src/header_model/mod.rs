//! The ordered header model, published as [`crate::header`].

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod card;
pub mod keywords;
pub(crate) mod value;

use std::collections::HashMap;

use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::header_model::card::Card;
use crate::header_model::card::ParsedRecord;
use crate::header_model::card::validate_ascii;
use crate::header_model::card::validate_valued_keyword;
use crate::header_model::value::Value;
use crate::image_model::scaling::Scaling;
use crate::keyword::Keyword;
use std::mem;
use std::slice;

/// A parsed header unit: an *ordered* list of content cards plus a side index
/// for O(1) keyword lookup.
///
/// Order and duplicates are preserved exactly (commentary cards repeat, and
/// record order is significant), so the model is a vector — never a map. The
/// terminating `END` record is implicit and not stored as a card. Long-string
/// values split across `CONTINUE` records are reassembled into a single value
/// card on read and re-emitted as a canonical `CONTINUE` chain on write, so the
/// round-trip preserves the logical model (not necessarily the original byte
/// split).
#[derive(Debug, Clone, Default)]
pub struct Header {
    pub(crate) cards: Vec<Card>,
    /// First occurrence of each valued keyword → index into `cards`.
    ///
    /// Invariant: every entry points at a [`Card::Value`] in `cards`.
    /// Parsing builds the index alongside the card list; every mutation method
    /// either maintains or rebuilds it. Raw card mutation stays private so the
    /// two cannot be desynchronized.
    index: HashMap<Keyword, usize>,
}

/// A read-only view of one stored header record, yielded by [`Header::iter`].
///
/// `value` is `None` for commentary (`COMMENT`/`HISTORY`/blank-keyword) cards and
/// `Some` for valued ones — that distinction is all a caller needs. `comment`
/// carries the inline `/`-comment of a valued card, or the whole free text of a
/// commentary card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Entry<'a> {
    pub keyword: &'a str,
    pub value: Option<&'a Value>,
    pub comment: Option<&'a str>,
}

impl<'a> Entry<'a> {
    fn of(card: &'a Card) -> Self {
        Entry {
            keyword: card.keyword(),
            value: card.value(),
            comment: card.comment(),
        }
    }
}

/// One owned logical header record, returned by ordered removal operations.
#[derive(Debug, Clone, PartialEq)]
pub struct Record {
    pub keyword: String,
    pub value: Option<Value>,
    pub comment: Option<String>,
}

impl Header {
    /// Parse a header unit from its raw bytes (a whole number of 80-byte cards;
    /// the reader supplies block-aligned input). Stops at the `END` record.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        // One record per card is the upper bound (CONTINUE folding only merges,
        // never adds; commentary cards skip the index), so reserve both once and
        // let parsing fill them without the grow-reallocations a small header would
        // otherwise pay on every push.
        let ncards = bytes.len() / CARD_SIZE;
        let mut cards: Vec<Card> = Vec::with_capacity(ncards);
        let mut index = HashMap::with_capacity(ncards);
        for chunk in bytes.as_chunks::<CARD_SIZE>().0 {
            let card = match ParsedRecord::parse(chunk)? {
                ParsedRecord::End => return Ok(Self { cards, index }),
                ParsedRecord::Continue { substring, comment } => {
                    if fold_continuation(&mut cards, &substring, comment.as_deref()) {
                        continue;
                    }
                    demoted_continuation(&substring, comment.as_deref())
                }
                ParsedRecord::Card(card) => card,
            };
            if let Card::Value { keyword, .. } = &card {
                index.entry(*keyword).or_insert(cards.len());
            }
            cards.push(card);
        }
        Err(FitsError::MissingEnd)
    }

    /// The value of the first valued card with this keyword. Every method that names a
    /// keyword takes a [`Keyword`], such as a [`keywords`] preset, or a `&str`, which
    /// errors with [`FitsError::InvalidKeyword`] when it is no keyword.
    pub fn get(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<&Value>> {
        Ok(self.value(checked(keyword)?))
    }

    fn value(&self, keyword: Keyword) -> Option<&Value> {
        let &i = self.index.get(&keyword)?;
        self.cards[i].value()
    }

    /// The first valued card with this keyword: its value and its comment.
    pub fn entry(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<Entry<'_>>> {
        let keyword = checked(keyword)?;
        Ok(self.index.get(&keyword).map(|&i| Entry::of(&self.cards[i])))
    }

    /// Read an optional logical keyword without conflating absence with a wrong type.
    pub fn get_logical(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<bool>> {
        self.typed_optional(checked(keyword)?, "logical", |value| Ok(value.as_logical()))
    }

    /// Read an optional integer keyword without conflating absence with a wrong type.
    pub fn get_integer(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<i64>> {
        self.typed_optional(checked(keyword)?, "integer", Value::as_integer)
    }

    /// Read an optional real keyword without conflating absence with a wrong type.
    pub fn get_real(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<f64>> {
        self.typed_optional(checked(keyword)?, "real", Value::as_real)
    }

    /// Read an optional text keyword without conflating absence with a wrong type.
    pub fn get_text(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<&str>> {
        self.typed_optional(checked(keyword)?, "text", |value| Ok(value.as_text()))
    }

    fn typed_optional<'a, T>(
        &'a self,
        keyword: Keyword,
        expected: &'static str,
        convert: impl FnOnce(&'a Value) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        let Some(value) = self.value(keyword) else {
            return Ok(None);
        };
        convert(value)?
            .map(Some)
            .ok_or_else(|| FitsError::TypeMismatch {
                name: keyword.to_string(),
                expected,
            })
    }

    /// A mandatory logical keyword: [`FitsError::MissingKeyword`] when it is absent,
    /// [`FitsError::TypeMismatch`] when it holds another type.
    pub fn require_logical(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<bool> {
        let keyword = checked(keyword)?;
        self.get_logical(keyword)?
            .ok_or(FitsError::MissingKeyword { keyword })
    }

    /// A mandatory integer keyword, as [`Header::require_logical`].
    pub fn require_integer(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<i64> {
        let keyword = checked(keyword)?;
        self.get_integer(keyword)?
            .ok_or(FitsError::MissingKeyword { keyword })
    }

    /// A mandatory real keyword, as [`Header::require_logical`].
    pub fn require_real(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<f64> {
        let keyword = checked(keyword)?;
        self.get_real(keyword)?
            .ok_or(FitsError::MissingKeyword { keyword })
    }

    /// A mandatory text keyword, as [`Header::require_logical`].
    pub fn require_text(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<&str> {
        let keyword = checked(keyword)?;
        self.get_text(keyword)?
            .ok_or(FitsError::MissingKeyword { keyword })
    }

    /// A mandatory integer keyword narrowed to `usize`. A negative or oversized
    /// value is [`FitsError::KeywordOutOfRange`], keeping untrusted sizes from
    /// wrapping into a plausible-looking length.
    pub(crate) fn require_usize(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<usize> {
        let keyword = checked(keyword)?;
        usize::try_from(self.require_integer(keyword)?)
            .map_err(|_| FitsError::KeywordOutOfRange { keyword })
    }

    /// An optional integer keyword narrowed to `usize`, or `default` when the card is
    /// absent. A present but out-of-range value still errors.
    pub(crate) fn optional_usize(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        default: usize,
    ) -> Result<usize> {
        let keyword = checked(keyword)?;
        match self.get_integer(keyword)? {
            Some(value) => {
                usize::try_from(value).map_err(|_| FitsError::KeywordOutOfRange { keyword })
            }
            None => Ok(default),
        }
    }

    /// The number of records, commentary ones included.
    pub const fn len(&self) -> usize {
        self.cards.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.cards.is_empty()
    }

    /// Whether a record, valued or commentary, has this keyword.
    pub fn contains(&self, keyword: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<bool> {
        Ok(self.index_of(keyword)?.is_some())
    }

    /// The position of the first record, valued or commentary, with this keyword.
    pub fn index_of(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<usize>> {
        let keyword = checked(keyword)?;
        Ok(self
            .cards
            .iter()
            .position(|card| card.keyword() == keyword.as_str()))
    }

    /// Every stored record in file order, as [`Entry`] views — duplicates and
    /// order preserved (the whole point of the ordered model), so `COMMENT`/`HISTORY`
    /// runs and repeated keywords come through intact. The implicit `END` is not a
    /// record and is never yielded. For valued keywords only, filter on
    /// `e.value`: `header.iter().filter_map(|e| Some((e.keyword, e.value?)))`.
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            cards: self.cards.iter(),
        }
    }

    /// The text of each `HISTORY` record, in order; an empty record gives `""`.
    pub fn history(&self) -> impl Iterator<Item = &str> {
        self.commentary("HISTORY")
    }

    /// The text of each `COMMENT` record, in order; an empty record gives `""`.
    pub fn comments(&self) -> impl Iterator<Item = &str> {
        self.commentary("COMMENT")
    }

    fn commentary(&self, keyword: &'static str) -> impl Iterator<Item = &str> {
        self.cards.iter().filter_map(move |card| match card {
            Card::Commentary {
                keyword: name,
                text,
            } if name == keyword => Some(text.as_deref().unwrap_or("")),
            _ => None,
        })
    }

    /// `BITPIX`, mapped to the typed element kind.
    pub fn bitpix(&self) -> Result<Bitpix> {
        let code = self.require_integer(keywords::BITPIX)?;
        Bitpix::from_code(code)
    }

    /// `NAXIS` — the number of axes (0 means no data array).
    pub fn naxis(&self) -> Result<usize> {
        let n = self.require_usize(keywords::NAXIS)?;
        // §4.4.1: `0 ≤ NAXIS ≤ 999`. Rejecting an out-of-range value is both
        // conformance and a guard — `axes()` reserves `Vec::with_capacity(NAXIS)`,
        // so an absurd `NAXIS` from an untrusted header would otherwise abort.
        if n > 999 {
            return Err(FitsError::KeywordOutOfRange {
                keyword: keywords::NAXIS,
            });
        }
        Ok(n)
    }

    /// The axis lengths `NAXIS1..NAXIS{NAXIS}`, in order.
    pub fn axes(&self) -> Result<Vec<usize>> {
        let naxis = self.naxis()?;
        let mut axes = Vec::with_capacity(naxis);
        for n in 1..=naxis {
            axes.push(self.require_usize(keywords::NAXIS.indexed(n)?)?);
        }
        Ok(axes)
    }

    /// `PCOUNT` (default 0), rejecting a negative value (§4.4.1). Primary-image
    /// checks and random-groups decode use this defaulted view; role-aware extension
    /// sizing requires the keyword explicitly.
    pub(crate) fn pcount(&self) -> Result<u64> {
        match self.get_integer(keywords::PCOUNT)? {
            Some(p) if p < 0 => Err(FitsError::KeywordOutOfRange {
                keyword: keywords::PCOUNT,
            }),
            Some(p) => Ok(p.cast_unsigned()),
            None => Ok(0),
        }
    }

    /// `GCOUNT` (default 1), rejecting a negative value (§4.4.1) — see [`Header::pcount`].
    pub(crate) fn gcount(&self) -> Result<u64> {
        match self.get_integer(keywords::GCOUNT)? {
            Some(g) if g < 0 => Err(FitsError::KeywordOutOfRange {
                keyword: keywords::GCOUNT,
            }),
            Some(g) => Ok(g.cast_unsigned()),
            None => Ok(1),
        }
    }

    /// The physical-value scaling (`BSCALE`/`BZERO`/`BLANK`) declared by this header.
    pub fn scaling(&self) -> Result<Scaling> {
        Scaling::from_header(self)
    }

    /// Create an empty header. Build it with [`Header::set`] and friends.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a valued keyword, or replace the value of an existing one, keeping
    /// the keyword index in sync. The keyword must not be a control or commentary
    /// keyword. Text must use restricted ASCII and numeric values must have a
    /// finite FITS representation. An error leaves the header unchanged.
    pub fn set(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        value: impl Into<Value>,
    ) -> Result<&mut Self> {
        let keyword = checked(keyword)?;
        let value = value.into();
        validate_valued_keyword(keyword)?;
        if let Some(&i) = self.index.get(&keyword) {
            let Card::Value { value: slot, .. } = &mut self.cards[i] else {
                unreachable!("the keyword index holds value cards only");
            };
            let previous = mem::replace(slot, value);
            if let Err(error) = self.cards[i].validate() {
                let Card::Value { value: slot, .. } = &mut self.cards[i] else {
                    unreachable!("the card was a value card a moment ago");
                };
                *slot = previous;
                return Err(error);
            }
        } else {
            let card = Card::Value {
                keyword,
                value,
                comment: None,
            };
            card.validate()?;
            self.index.insert(keyword, self.cards.len());
            self.cards.push(card);
        }
        Ok(self)
    }

    /// [`Header::set`] for `Some`; for `None`, remove every record with this keyword,
    /// so the header holds `value` or nothing.
    pub fn set_optional(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        value: Option<impl Into<Value>>,
    ) -> Result<&mut Self> {
        let keyword = checked(keyword)?;
        validate_valued_keyword(keyword)?;
        let Some(value) = value else {
            self.remove_all(keyword)?;
            return Ok(self);
        };
        self.set(keyword, value)
    }

    #[track_caller]
    pub(crate) fn set_internal(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        value: impl Into<Value>,
    ) -> &mut Self {
        self.set(keyword, value)
            .expect("internal FITS header metadata must be valid")
    }

    /// Append a duplicate valued card without replacing an earlier occurrence.
    /// Lookup continues to return the first occurrence, matching parsed headers.
    pub fn append(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        value: impl Into<Value>,
    ) -> Result<&mut Self> {
        self.insert(self.cards.len(), keyword, value)
    }

    /// Insert a valued card at a logical record position. Unlike [`Header::set`],
    /// this never replaces an existing card and can deliberately create duplicates.
    pub fn insert(
        &mut self,
        index: usize,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        value: impl Into<Value>,
    ) -> Result<&mut Self> {
        let keyword = checked(keyword)?;
        validate_valued_keyword(keyword)?;
        self.insert_card(
            index,
            Card::Value {
                keyword,
                value: value.into(),
                comment: None,
            },
        )
    }

    fn insert_card(&mut self, index: usize, card: Card) -> Result<&mut Self> {
        if index > self.cards.len() {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::HeaderRecord,
                index,
                len: self.cards.len(),
            });
        }
        card.validate()?;
        self.cards.insert(index, card);
        self.reindex();
        Ok(self)
    }

    /// Remove the first logical record with `keyword`.
    pub fn remove(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<Record>> {
        let Some(index) = self.index_of(keyword)? else {
            return Ok(None);
        };
        self.remove_at(index).map(Some)
    }

    /// Remove one logical record by zero-based position.
    pub fn remove_at(&mut self, index: usize) -> Result<Record> {
        if index >= self.cards.len() {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::HeaderRecord,
                index,
                len: self.cards.len(),
            });
        }
        let card = self.cards.remove(index);
        self.reindex();
        Ok(match card {
            Card::Value {
                keyword,
                value,
                comment,
            } => Record {
                keyword: keyword.to_string(),
                value: Some(value),
                comment,
            },
            Card::Commentary { keyword, text } => Record {
                keyword,
                value: None,
                comment: text,
            },
        })
    }

    /// Remove every logical record with `keyword`, returning the count removed.
    ///
    /// Each call that removes anything rebuilds the keyword index, so dropping
    /// several keywords is one pass' worth of work per call.
    pub fn remove_all(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<usize> {
        let keyword = checked(keyword)?;
        let old_len = self.cards.len();
        self.remove_where(|card| card == keyword.as_str());
        Ok(old_len - self.cards.len())
    }

    pub(crate) fn append_filtered_from(
        &mut self,
        source: &Self,
        mut keep: impl FnMut(&str) -> bool,
    ) {
        self.cards.extend(
            source
                .cards
                .iter()
                .filter(|card| keep(card.keyword()))
                .cloned(),
        );
        self.reindex();
    }

    /// Remove all cards matching `should_remove`, then rebuild the keyword index
    /// once after the bulk structural edit.
    pub(crate) fn remove_where(
        &mut self,
        mut should_remove: impl FnMut(&str) -> bool,
    ) -> &mut Self {
        let old_len = self.cards.len();
        self.cards.retain(|card| !should_remove(card.keyword()));
        if self.cards.len() != old_len {
            self.reindex();
        }
        self
    }

    /// Rename valued cards in place so their order, values, and comments survive
    /// a reversible metadata transformation.
    #[cfg(feature = "compression")]
    pub(crate) fn rename_keywords(&mut self, renames: &[(Keyword, Keyword)]) {
        let mut changed = false;
        for card in &mut self.cards {
            let Card::Value { keyword, .. } = card else {
                continue;
            };
            if let Some(&(_, to)) = renames.iter().find(|(from, _)| keyword == from) {
                *keyword = to;
                changed = true;
            }
        }
        if changed {
            self.reindex();
        }
    }

    /// Rebuild the keyword → first-card index after a structural edit.
    fn reindex(&mut self) {
        self.index.clear();
        for (i, card) in self.cards.iter().enumerate() {
            if let Card::Value { keyword, .. } = card {
                self.index.entry(*keyword).or_insert(i);
            }
        }
    }

    /// Attach (or replace) the inline comment of an existing valued keyword. Errors
    /// with [`FitsError::KeywordNotFound`] for a keyword the header does not hold.
    /// An error leaves the header unchanged.
    pub fn set_comment(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        text: &str,
    ) -> Result<&mut Self> {
        let keyword = checked(keyword)?;
        validate_ascii(text, "header comment")?;
        let Some(&i) = self.index.get(&keyword) else {
            return Err(FitsError::KeywordNotFound { keyword });
        };
        let Card::Value { comment, .. } = &mut self.cards[i] else {
            unreachable!("the keyword index holds value cards only");
        };
        let previous = comment.replace(text.to_string());
        if let Err(error) = self.cards[i].validate() {
            let Card::Value { comment, .. } = &mut self.cards[i] else {
                unreachable!("the card was a value card a moment ago");
            };
            *comment = previous;
            return Err(error);
        }
        Ok(self)
    }

    #[track_caller]
    pub(crate) fn set_comment_internal(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        text: &str,
    ) -> &mut Self {
        self.set_comment(keyword, text)
            .expect("internal FITS header comments must be valid")
    }

    /// Append one restricted-ASCII `COMMENT` record. Text that cannot fit the
    /// record returns an error without changing the header.
    pub fn push_comment(&mut self, text: &str) -> Result<&mut Self> {
        let card = Card::commentary("COMMENT", text);
        card.validate()?;
        self.cards.push(card);
        Ok(self)
    }

    /// Append one restricted-ASCII `HISTORY` record. Text that cannot fit the
    /// record returns an error without changing the header.
    pub fn push_history(&mut self, text: &str) -> Result<&mut Self> {
        let card = Card::commentary("HISTORY", text);
        card.validate()?;
        self.cards.push(card);
        Ok(self)
    }

    /// Append a blank-keyword commentary record. `text = ""` produces a physically
    /// blank card; nonempty text uses the same restricted-ASCII rules.
    pub fn push_blank(&mut self, text: &str) -> Result<&mut Self> {
        let card = Card::commentary("", text);
        card.validate()?;
        self.cards.push(card);
        Ok(self)
    }
}

/// The keyword `name` names: a [`Keyword`] as it is, a `&str` checked into one.
fn checked(name: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Keyword> {
    name.try_into().map_err(Into::into)
}

impl<'a> IntoIterator for &'a Header {
    type Item = Entry<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// The records of a [`Header`], in order, as [`Entry`] views.
#[derive(Debug, Clone)]
pub struct Iter<'a> {
    cards: slice::Iter<'a, Card>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = Entry<'a>;

    fn next(&mut self) -> Option<Entry<'a>> {
        self.cards.next().map(Entry::of)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.cards.size_hint()
    }
}

impl ExactSizeIterator for Iter<'_> {}

/// A `CONTINUE` record with no value card to extend, as the commentary record its
/// bytes 9–80 spell.
fn demoted_continuation(substring: &str, comment: Option<&str>) -> Card {
    let mut text = format!("  '{}'", substring.replace('\'', "''"));
    if let Some(comment) = comment {
        text.push_str(" / ");
        text.push_str(comment);
    }
    Card::Commentary {
        keyword: "CONTINUE".to_string(),
        text: Some(text),
    }
}

/// Fold a `CONTINUE` substring into the preceding long-string value card,
/// returning `false` when the previous card is not a value awaiting continuation
/// (i.e. a [`Value::Text`] whose text ends with the `&` continuation flag).
fn fold_continuation(cards: &mut [Card], substring: &str, fragment: Option<&str>) -> bool {
    let Some(Card::Value {
        value: Value::Text(acc),
        comment,
        ..
    }) = cards.last_mut()
    else {
        return false;
    };
    if !acc.ends_with('&') {
        return false;
    }
    acc.pop(); // drop the continuation flag
    acc.push_str(substring);
    if let Some(fragment) = fragment {
        match comment {
            Some(comment) => {
                if !comment.is_empty() {
                    comment.push(' ');
                }
                comment.push_str(fragment);
            }
            None => *comment = Some(fragment.to_string()),
        }
    }
    true
}

/// Card builders for the tests of every module that parses headers.
#[cfg(test)]
pub(crate) mod internals {
    use crate::block::CARD_SIZE;
    use crate::header_model::Header;

    /// One record: `text` left-justified in 80 blank-filled columns.
    pub(crate) fn card_bytes(text: &str) -> [u8; CARD_SIZE] {
        assert!(text.len() <= CARD_SIZE, "{text:?} is longer than a card");
        let mut card = [b' '; CARD_SIZE];
        card[..text.len()].copy_from_slice(text.as_bytes());
        card
    }

    /// The records of `lines`, in order.
    pub(crate) fn records(lines: &[&str]) -> Vec<u8> {
        lines.iter().flat_map(|line| card_bytes(line)).collect()
    }

    /// The header `lines` and an appended `END` record parse to.
    pub(crate) fn from_card_lines(lines: &[&str]) -> Header {
        let mut bytes = records(lines);
        bytes.extend_from_slice(&card_bytes("END"));
        Header::parse(&bytes).unwrap()
    }
}

#[cfg(test)]
mod tests;
