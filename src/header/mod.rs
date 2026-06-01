pub(crate) mod card;
pub(crate) mod value;

use std::collections::HashMap;

use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::card::Card;
use crate::header::card::CardKind;
use crate::header::card::validate_keyword;
use crate::header::value::Value;
use crate::keyword::key;

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
    /// Invariant: every entry points at a [`CardKind::Value`] card in `cards`.
    /// `cards` is only ever appended/extended in place during `parse`, never
    /// reordered, so the index stays valid. Any future card-mutation API must
    /// rebuild this (or it must be made a method that maintains it) — do not
    /// expose raw mutation that can desynchronize the two.
    index: HashMap<String, usize>,
}

impl Header {
    /// Parse a header unit from its raw bytes (a whole number of 80-byte cards;
    /// the reader supplies block-aligned input). Stops at the `END` record.
    pub fn parse(bytes: &[u8]) -> Result<Header> {
        let mut cards: Vec<Card> = Vec::new();
        let mut index = HashMap::new();
        for chunk in bytes.chunks_exact(CARD_SIZE) {
            let card = Card::parse(
                chunk
                    .try_into()
                    .expect("chunks_exact yields CARD_SIZE slices"),
            )?;
            match card.kind {
                CardKind::End => return Ok(Header { cards, index }),
                CardKind::Continue if fold_continuation(&mut cards, &card) => {}
                _ => {
                    let mut card = card;
                    // A CONTINUE with no value card to extend is malformed; keep it
                    // readable by demoting it to a commentary card.
                    if card.kind == CardKind::Continue {
                        card.kind = CardKind::Commentary;
                        card.value = None;
                    }
                    if matches!(card.kind, CardKind::Value | CardKind::Hierarch) {
                        index.entry(card.keyword.clone()).or_insert(cards.len());
                    }
                    cards.push(card);
                }
            }
        }
        Err(FitsError::MissingEnd)
    }

    /// The value of the first card with this keyword, if it is a valued card.
    pub fn get(&self, keyword: &str) -> Option<&Value> {
        self.index
            .get(keyword)
            .and_then(|&i| self.cards[i].value.as_ref())
    }

    pub fn get_logical(&self, keyword: &str) -> Option<bool> {
        self.get(keyword)?.as_logical()
    }

    pub fn get_integer(&self, keyword: &str) -> Option<i64> {
        self.get(keyword)?.as_integer()
    }

    pub fn get_real(&self, keyword: &str) -> Option<f64> {
        self.get(keyword)?.as_real()
    }

    pub fn get_text(&self, keyword: &str) -> Option<&str> {
        self.get(keyword)?.as_text()
    }

    /// `BITPIX`, mapped to the typed element kind.
    pub fn bitpix(&self) -> Result<Bitpix> {
        let code = self
            .get_integer("BITPIX")
            .ok_or(FitsError::MissingKeyword { name: "BITPIX" })?;
        Bitpix::from_code(code)
    }

    /// `NAXIS` — the number of axes (0 means no data array).
    pub fn naxis(&self) -> Result<usize> {
        let n = self
            .get_integer("NAXIS")
            .ok_or(FitsError::MissingKeyword { name: "NAXIS" })?;
        // §4.4.1: `0 ≤ NAXIS ≤ 999`. Rejecting an out-of-range value is both
        // conformance and a guard — `axes()` reserves `Vec::with_capacity(NAXIS)`,
        // so an absurd `NAXIS` from an untrusted header would otherwise abort.
        match usize::try_from(n) {
            Ok(n) if n <= 999 => Ok(n),
            _ => Err(FitsError::KeywordOutOfRange { name: "NAXIS" }),
        }
    }

    /// The axis lengths `NAXIS1..NAXIS{NAXIS}`, in order.
    pub fn axes(&self) -> Result<Vec<usize>> {
        let naxis = self.naxis()?;
        let mut axes = Vec::with_capacity(naxis);
        for n in 1..=naxis {
            let len = self
                .get_integer(key!("NAXIS{n}").as_str())
                .ok_or(FitsError::MissingKeyword { name: "NAXISn" })?;
            axes.push(
                usize::try_from(len)
                    .map_err(|_| FitsError::KeywordOutOfRange { name: "NAXISn" })?,
            );
        }
        Ok(axes)
    }

    /// Create an empty header. Build it up with [`Header::set`] and friends.
    pub fn new() -> Header {
        Header::default()
    }

    /// Insert a valued keyword, or replace the value of an existing one, keeping
    /// the keyword index in sync. Returns `&mut self` for chaining. The keyword
    /// must be a valid FITS keyword name (≤ 8 chars of `A–Z`, `0–9`, `-`, `_`).
    pub fn set(&mut self, keyword: &str, value: impl Into<Value>) -> &mut Self {
        assert!(
            validate_keyword(keyword).is_ok(),
            "Header::set: invalid FITS keyword {keyword:?}"
        );
        let value = value.into();
        if let Some(&i) = self.index.get(keyword) {
            self.cards[i].value = Some(value);
        } else {
            self.index.insert(keyword.to_string(), self.cards.len());
            self.cards.push(Card::value(keyword, value));
        }
        self
    }

    /// Remove every card with this keyword and rebuild the index. A no-op if the
    /// keyword is absent. Used when transforming headers (e.g. stripping the `Z*`
    /// keywords when uncompressing a tiled table).
    #[cfg(feature = "compression")]
    pub(crate) fn remove(&mut self, keyword: &str) -> &mut Self {
        if self.index.contains_key(keyword) {
            self.cards.retain(|c| c.keyword != keyword);
            self.reindex();
        }
        self
    }

    /// Rebuild the keyword → first-card index after a structural edit.
    #[cfg(feature = "compression")]
    fn reindex(&mut self) {
        self.index.clear();
        for (i, card) in self.cards.iter().enumerate() {
            if matches!(card.kind, CardKind::Value | CardKind::Hierarch) {
                self.index.entry(card.keyword.clone()).or_insert(i);
            }
        }
    }

    /// Attach (or replace) the inline comment of an existing valued keyword;
    /// a no-op if the keyword is absent.
    pub fn comment(&mut self, keyword: &str, text: &str) -> &mut Self {
        if let Some(&i) = self.index.get(keyword) {
            self.cards[i].comment = Some(text.to_string());
        }
        self
    }

    /// Append a `COMMENT` card.
    pub fn push_comment(&mut self, text: &str) -> &mut Self {
        self.cards.push(Card::commentary("COMMENT", text));
        self
    }

    /// Append a `HISTORY` card.
    pub fn push_history(&mut self, text: &str) -> &mut Self {
        self.cards.push(Card::commentary("HISTORY", text));
        self
    }
}

/// Fold a `CONTINUE` substring into the preceding long-string value card,
/// returning `false` when the previous card is not a value awaiting continuation
/// (i.e. a [`Value::Text`] whose text ends with the `&` continuation flag).
fn fold_continuation(cards: &mut [Card], cont: &Card) -> bool {
    let Some(prev) = cards.last_mut() else {
        return false;
    };
    let Some(Value::Text(acc)) = prev.value.as_mut() else {
        return false;
    };
    if !acc.ends_with('&') {
        return false;
    }
    acc.pop(); // drop the continuation flag
    if let Some(Value::Text(sub)) = &cont.value {
        acc.push_str(sub);
    }
    // The convention carries any comment on the final CONTINUE record.
    if cont.comment.is_some() {
        prev.comment = cont.comment.clone();
    }
    true
}

/// Build a header from left-justified 80-column card text lines, appending the
/// `END` record. Shared test helper for modules that exercise parsed headers.
#[cfg(test)]
pub(crate) fn from_card_lines(lines: &[&str]) -> Header {
    let mut buf = Vec::with_capacity((lines.len() + 1) * CARD_SIZE);
    for line in lines {
        let mut card = [b' '; CARD_SIZE];
        card[..line.len()].copy_from_slice(line.as_bytes());
        buf.extend_from_slice(&card);
    }
    let mut end = [b' '; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    Header::parse(&buf).unwrap()
}

#[cfg(test)]
mod tests;
