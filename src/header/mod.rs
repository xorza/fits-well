pub(crate) mod card;
pub(crate) mod value;

use std::collections::HashMap;

use crate::bitpix::Bitpix;
use crate::block::CARD_SIZE;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::card::Card;
use crate::header::card::CardKind;
use crate::header::value::Value;

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
                    if card.kind == CardKind::Value {
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
        usize::try_from(n).map_err(|_| FitsError::WrongValueType { name: "NAXIS" })
    }

    /// The axis lengths `NAXIS1..NAXIS{NAXIS}`, in order.
    pub fn axes(&self) -> Result<Vec<usize>> {
        let naxis = self.naxis()?;
        let mut axes = Vec::with_capacity(naxis);
        for n in 1..=naxis {
            let len = self
                .get_integer(&format!("NAXIS{n}"))
                .ok_or(FitsError::MissingKeyword { name: "NAXISn" })?;
            axes.push(
                usize::try_from(len).map_err(|_| FitsError::WrongValueType { name: "NAXISn" })?,
            );
        }
        Ok(axes)
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

#[cfg(test)]
mod tests {
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
}
