pub(crate) mod card;
pub(crate) mod value;

use std::collections::HashMap;

use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::data::Scaling;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::card::Card;
use crate::header::card::CardKind;
use crate::header::card::validate_ascii;
#[cfg(feature = "compression")]
use crate::header::card::validate_keyword;
use crate::header::card::validate_valued_keyword;
use crate::header::value::Value;
use crate::keyword::key;
use crate::time::{FitsTime, PhaseAxis, TimeBounds, TimeCoordinate};
use crate::wcs::Wcs;

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
    /// Invariant: every entry points at a card that carries a value — a
    /// [`CardKind::Value`] or [`CardKind::Hierarch`] — in `cards`.
    /// Parsing builds the index alongside the card list; every mutation method
    /// either maintains or rebuilds it. Raw card mutation stays private so the
    /// two cannot be desynchronized.
    index: HashMap<String, usize>,
}

/// A read-only view of one stored header record, yielded by [`Header::iter`].
///
/// `value` is `None` for commentary (`COMMENT`/`HISTORY`/blank-keyword) cards and
/// `Some` for valued ones — that distinction is all a caller needs, so the internal
/// `CardKind` (which also tags transient parse states) stays private. `comment`
/// carries the inline `/`-comment of a valued card, or the whole free text of a
/// commentary card.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HeaderEntry<'a> {
    pub keyword: &'a str,
    pub value: Option<&'a Value>,
    pub comment: Option<&'a str>,
}

impl Header {
    /// Parse a header unit from its raw bytes (a whole number of 80-byte cards;
    /// the reader supplies block-aligned input). Stops at the `END` record.
    pub fn parse(bytes: &[u8]) -> Result<Header> {
        // One record per card is the upper bound (CONTINUE folding only merges,
        // never adds; commentary cards skip the index), so reserve both once and
        // let parsing fill them without the grow-reallocations a small header would
        // otherwise pay on every push.
        let ncards = bytes.len() / CARD_SIZE;
        let mut cards: Vec<Card> = Vec::with_capacity(ncards);
        let mut index = HashMap::with_capacity(ncards);
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
                        demote_continuation(&mut card);
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

    /// Read an optional logical keyword without conflating absence with a wrong type.
    pub fn get_logical(&self, keyword: &str) -> Result<Option<bool>> {
        self.typed_optional(keyword, "logical", |value| Ok(value.as_logical()))
    }

    /// Read an optional integer keyword without conflating absence with a wrong type.
    pub fn get_integer(&self, keyword: &str) -> Result<Option<i64>> {
        self.typed_optional(keyword, "integer", Value::as_integer)
    }

    /// Read an optional real keyword without conflating absence with a wrong type.
    pub fn get_real(&self, keyword: &str) -> Result<Option<f64>> {
        self.typed_optional(keyword, "real", Value::as_real)
    }

    /// Read an optional text keyword without conflating absence with a wrong type.
    pub fn get_text(&self, keyword: &str) -> Result<Option<&str>> {
        self.typed_optional(keyword, "text", |value| Ok(value.as_text()))
    }

    fn typed_optional<'a, T>(
        &'a self,
        keyword: &str,
        expected: &'static str,
        convert: impl FnOnce(&'a Value) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        let Some(value) = self.get(keyword) else {
            return Ok(None);
        };
        convert(value)?
            .map(Some)
            .ok_or_else(|| FitsError::TypeMismatch {
                name: keyword.to_string(),
                expected,
            })
    }

    fn required_integer(&self, keyword: &str, missing_name: &'static str) -> Result<i64> {
        self.get_integer(keyword)?
            .ok_or(FitsError::MissingKeyword { name: missing_name })
    }

    /// Every stored record in file order, as [`HeaderEntry`] views — duplicates and
    /// order preserved (the whole point of the ordered model), so `COMMENT`/`HISTORY`
    /// runs and repeated keywords come through intact. The implicit `END` is not a
    /// record and is never yielded. For valued keywords only, filter on
    /// `e.value`: `header.iter().filter_map(|e| Some((e.keyword, e.value?)))`.
    pub fn iter(&self) -> impl Iterator<Item = HeaderEntry<'_>> {
        self.cards.iter().map(|c| HeaderEntry {
            keyword: c.keyword.as_str(),
            value: c.value.as_ref(),
            comment: c.comment.as_deref(),
        })
    }

    /// `BITPIX`, mapped to the typed element kind.
    pub fn bitpix(&self) -> Result<Bitpix> {
        let code = self.required_integer("BITPIX", "BITPIX")?;
        Bitpix::from_code(code)
    }

    /// `NAXIS` — the number of axes (0 means no data array).
    pub fn naxis(&self) -> Result<usize> {
        let n = self.required_integer("NAXIS", "NAXIS")?;
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
            let len = self.required_integer(key!("NAXIS{n}").as_str(), "NAXISn")?;
            axes.push(
                usize::try_from(len)
                    .map_err(|_| FitsError::KeywordOutOfRange { name: "NAXISn" })?,
            );
        }
        Ok(axes)
    }

    /// `PCOUNT` (default 0), rejecting a negative value (§4.4.1). Primary-image
    /// checks and random-groups decode use this defaulted view; role-aware extension
    /// sizing requires the keyword explicitly.
    pub(crate) fn pcount(&self) -> Result<u64> {
        match self.get_integer("PCOUNT")? {
            Some(p) if p < 0 => Err(FitsError::KeywordOutOfRange { name: "PCOUNT" }),
            Some(p) => Ok(p as u64),
            None => Ok(0),
        }
    }

    /// `GCOUNT` (default 1), rejecting a value `< 1` (§4.4.1) — see [`Header::pcount`].
    pub(crate) fn gcount(&self) -> Result<u64> {
        match self.get_integer("GCOUNT")? {
            Some(g) if g < 1 => Err(FitsError::KeywordOutOfRange { name: "GCOUNT" }),
            Some(g) => Ok(g as u64),
            None => Ok(1),
        }
    }

    /// The physical-value scaling (`BSCALE`/`BZERO`/`BLANK`) declared by this header.
    pub fn scaling(&self) -> Result<Scaling> {
        Scaling::from_header(self)
    }

    /// Parse the World Coordinate System (FITS §8) described by this header: the
    /// primary description (`alt = None`) or an alternate (`alt = Some('A'..='Z')`).
    pub fn wcs(&self, alt: Option<char>) -> Result<Wcs> {
        Wcs::from_header(self, alt)
    }

    /// WCS for a *pixel-list* table (§8.4.2), where the given `columns` hold the
    /// coordinate axes; `alt` selects the primary (`None`) or an alternate system.
    pub fn wcs_pixel_list(&self, columns: &[usize], alt: Option<char>) -> Result<Wcs> {
        Wcs::from_pixel_list(self, columns, alt)
    }

    /// WCS attached to a single array-valued table `column` (§8.4.1).
    pub fn wcs_array_column(&self, column: usize, alt: Option<char>) -> Result<Wcs> {
        Wcs::from_array_column(self, column, alt)
    }

    /// The time-coordinate frame (FITS §9) parsed from this header — reference
    /// epoch/scale, units, and any time WCS axis.
    pub fn time(&self) -> Result<FitsTime> {
        FitsTime::from_header(self)
    }

    /// The time-coordinate frame for a binary-table `column` (1-based).
    /// `TRPOSn` overrides global `TREFPOS`; both default to `TOPOCENTER`.
    pub fn time_for_column(&self, column: usize) -> Result<FitsTime> {
        FitsTime::from_header_for_column(self, Some(column))
    }

    /// The observation Modified Julian Date — `MJD-OBS`, else `DATE-OBS`, else the
    /// `JEPOCH`/`BEPOCH` epoch, else `None`.
    pub fn obs_mjd(&self) -> Result<Option<f64>> {
        FitsTime::obs_mjd(self)
    }

    /// The Julian (`JEPOCH`) or Besselian (`BEPOCH`) epoch keyword, if present.
    pub fn epoch(&self) -> Result<Option<TimeCoordinate>> {
        FitsTime::epoch(self)
    }

    /// The observation time bounds (start/end/duration, §9.2.3) from this header.
    pub fn time_bounds(&self) -> Result<TimeBounds> {
        FitsTime::bounds(self)
    }

    /// The §9.6 `'PHASE'` parameters for an image WCS `axis` (1-based).
    pub fn phase_axis(&self, axis: usize, alt: Option<char>) -> Result<Option<PhaseAxis>> {
        FitsTime::phase_axis(self, axis, alt)
    }

    /// The §9.6 `'PHASE'` parameters for a binary-table pixel-list `column`.
    pub fn phase_axis_pixel_list(
        &self,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        FitsTime::phase_axis_pixel_list(self, column, alt)
    }

    /// The §9.6 `'PHASE'` parameters for an axis in an array-valued table column.
    pub fn phase_axis_array_column(
        &self,
        axis: usize,
        column: usize,
        alt: Option<char>,
    ) -> Result<Option<PhaseAxis>> {
        FitsTime::phase_axis_array_column(self, axis, column, alt)
    }

    /// Create an empty header. Build it with [`Header::try_set`] and friends.
    pub fn new() -> Header {
        Header::default()
    }

    /// Insert a valued keyword, or replace the value of an existing one, keeping
    /// the keyword index in sync. The keyword must be a valid FITS keyword name
    /// (≤ 8 chars of `A–Z`, `0–9`, `-`, `_`) and must not be a control or
    /// commentary keyword. Text must use restricted ASCII and numeric values must
    /// have a finite FITS representation. An error leaves the header unchanged.
    pub fn try_set(&mut self, keyword: &str, value: impl Into<Value>) -> Result<&mut Self> {
        validate_valued_keyword(keyword)?;
        self.try_set_card(Card::value(keyword, value.into()))
    }

    /// Insert or replace an ESO HIERARCH convention keyword. `keyword` is the
    /// effective compound name without the `HIERARCH ` prefix. It must be
    /// nonempty restricted ASCII without leading/trailing spaces or `=`;
    /// internal spaces separate hierarchy tokens. An error leaves the header
    /// unchanged.
    pub fn try_set_hierarch(
        &mut self,
        keyword: &str,
        value: impl Into<Value>,
    ) -> Result<&mut Self> {
        self.try_set_card(Card::hierarch(keyword, value.into()))
    }

    fn try_set_card(&mut self, card: Card) -> Result<&mut Self> {
        let keyword = card.keyword.clone();
        if let Some(&i) = self.index.get(&keyword) {
            let mut replacement = self.cards[i].clone();
            replacement.value = card.value;
            replacement.kind = card.kind;
            replacement.validate()?;
            self.cards[i] = replacement;
        } else {
            card.validate()?;
            self.index.insert(keyword, self.cards.len());
            self.cards.push(card);
        }
        Ok(self)
    }

    #[track_caller]
    pub(crate) fn set(&mut self, keyword: &str, value: impl Into<Value>) -> &mut Self {
        self.try_set(keyword, value)
            .expect("internal FITS header metadata must be valid")
    }

    /// Remove all cards matching `should_remove`, then rebuild the keyword index
    /// once after the bulk structural edit.
    #[cfg(feature = "compression")]
    pub(crate) fn remove_where(
        &mut self,
        mut should_remove: impl FnMut(&str) -> bool,
    ) -> &mut Self {
        let old_len = self.cards.len();
        self.cards.retain(|card| !should_remove(&card.keyword));
        if self.cards.len() != old_len {
            self.reindex();
        }
        self
    }

    /// Rename valued cards in place so their order, values, and comments survive
    /// a reversible metadata transformation.
    #[cfg(feature = "compression")]
    pub(crate) fn rename_keywords(&mut self, renames: &[(&str, &str)]) {
        let mut changed = false;
        for card in &mut self.cards {
            if let Some((_, to)) = renames.iter().find(|(from, _)| card.keyword == *from) {
                debug_assert!(validate_keyword(to).is_ok());
                card.keyword = (*to).to_string();
                changed = true;
            }
        }
        if changed {
            self.reindex();
        }
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

    /// Attach (or replace) the inline comment of an existing valued keyword. A
    /// missing valid keyword remains a no-op. An error leaves the header unchanged.
    pub fn try_comment(&mut self, keyword: &str, text: &str) -> Result<&mut Self> {
        validate_ascii(text, "header comment")?;
        let Some(&i) = self.index.get(keyword) else {
            validate_valued_keyword(keyword)?;
            return Ok(self);
        };
        let mut replacement = self.cards[i].clone();
        replacement.comment = Some(text.to_string());
        replacement.validate()?;
        self.cards[i] = replacement;
        Ok(self)
    }

    #[track_caller]
    pub(crate) fn comment(&mut self, keyword: &str, text: &str) -> &mut Self {
        self.try_comment(keyword, text)
            .expect("internal FITS header comments must be valid")
    }

    /// Append one restricted-ASCII `COMMENT` record. Text that cannot fit the
    /// record returns an error without changing the header.
    pub fn try_push_comment(&mut self, text: &str) -> Result<&mut Self> {
        let card = Card::commentary("COMMENT", text);
        card.validate()?;
        self.cards.push(card);
        Ok(self)
    }

    #[track_caller]
    #[cfg(test)]
    pub(crate) fn push_comment(&mut self, text: &str) -> &mut Self {
        self.try_push_comment(text)
            .expect("internal FITS COMMENT text must be valid")
    }

    /// Append one restricted-ASCII `HISTORY` record. Text that cannot fit the
    /// record returns an error without changing the header.
    pub fn try_push_history(&mut self, text: &str) -> Result<&mut Self> {
        let card = Card::commentary("HISTORY", text);
        card.validate()?;
        self.cards.push(card);
        Ok(self)
    }

    #[track_caller]
    #[cfg(test)]
    pub(crate) fn push_history(&mut self, text: &str) -> &mut Self {
        self.try_push_history(text)
            .expect("internal FITS HISTORY text must be valid")
    }
}

fn demote_continuation(card: &mut Card) {
    let substring = card
        .value
        .as_ref()
        .and_then(Value::as_text)
        .expect("parsed CONTINUE card must carry a text substring");
    let mut commentary = format!("  '{}'", substring.replace('\'', "''"));
    if let Some(comment) = &card.comment {
        commentary.push_str(" / ");
        commentary.push_str(comment);
    }
    card.kind = CardKind::Commentary;
    card.value = None;
    card.comment = Some(commentary);
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
    if let Some(fragment) = &cont.comment {
        if let Some(comment) = &mut prev.comment {
            if !comment.is_empty() {
                comment.push(' ');
            }
            comment.push_str(fragment);
        } else {
            prev.comment = Some(fragment.clone());
        }
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
