//! The ordered header model, published as [`crate::header`].

#[cfg(feature = "internals")]
pub(crate) mod bench;
pub(crate) mod card;
pub(crate) mod commentary_name;
pub(crate) mod defect;
pub(crate) mod hierarch_name;
pub mod keywords;
pub(crate) mod record;
pub(crate) mod record_name;
pub(crate) mod repair;
pub(crate) mod value;

use std::collections::HashMap;

use crate::bitpix::Bitpix;
use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::RECORDS_PER_BLOCK;
use crate::checksum;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Result;
use crate::error::{TextOrigin, ValueKind};
use crate::header_model::card::ParsedRecord;
use crate::header_model::card::validate_ascii;
use crate::header_model::card::validate_valued_keyword;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::hierarch_name::HierarchName;
use crate::header_model::record::{Record, RecordView};
use crate::header_model::record_name::RecordName;
use crate::header_model::repair::Repair;
use crate::header_model::value::Value;
use crate::keyword::Keyword;
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
    records: Vec<Record>,
    /// First occurrence of each valued keyword → index into `records`.
    ///
    /// Invariant: every entry points at the first [`Record::Valued`] with that keyword,
    /// or a [`Record::Malformed`] so named, whose lookup fails. Parsing builds the index
    /// alongside the records; an edit of one record maintains it, and a bulk edit
    /// rebuilds it. The records stay private so the two cannot part.
    index: HashMap<Keyword, usize>,
    /// First occurrence of each `HIERARCH` name → index into `records`, under the same
    /// invariant.
    hierarch_index: HashMap<HierarchName, usize>,
    /// The repairs the parse made, a report of the input that edits leave as it is.
    repairs: Vec<Repair>,
}

impl Header {
    /// Parse a header unit from its raw bytes (a whole number of 80-byte cards;
    /// the reader supplies block-aligned input). Stops at the `END` record.
    pub fn parse(bytes: &[u8]) -> Result<Self> {
        // One record per card is the upper bound (CONTINUE folding only merges,
        // never adds; commentary records skip the index), so reserve both once and
        // let parsing fill them without the grow-reallocations a small header would
        // otherwise pay on every push.
        let ncards = bytes.len() / CARD_SIZE;
        let mut header = Self {
            records: Vec::with_capacity(ncards),
            index: HashMap::with_capacity(ncards),
            hierarch_index: HashMap::new(),
            repairs: Vec::new(),
        };
        for (position, chunk) in bytes.as_chunks::<CARD_SIZE>().0.iter().enumerate() {
            let record = match ParsedRecord::parse(chunk, position + 1, &mut header.repairs) {
                ParsedRecord::End => {
                    // The blank records before `END` are space for later records, not
                    // content (cfitsio `ffhdef`, astropy `useblanks`).
                    while matches!(
                        header.records.last(),
                        Some(Record::Commentary { name, text: None }) if *name == CommentaryName::BLANK
                    ) {
                        header.records.pop();
                    }
                    return Ok(header);
                }
                ParsedRecord::Continue {
                    substring,
                    comment,
                    tail,
                } => {
                    if fold_continuation(&mut header.records, &substring, comment.as_deref()) {
                        continue;
                    }
                    Record::Commentary {
                        name: CommentaryName::CONTINUE,
                        text: tail,
                    }
                }
                ParsedRecord::Record(record) => record,
            };
            header.push_record(record);
        }
        Err(FitsError::MissingEnd)
    }

    /// The repairs the parse made, in record order. A header built in code has none.
    pub fn repairs(&self) -> &[Repair] {
        &self.repairs
    }

    /// The value of the first valued card with this keyword. Every method that names a
    /// keyword takes a [`Keyword`], such as a [`keywords`] preset, or a `&str`, which
    /// errors with [`FitsError::InvalidKeyword`] when it is no keyword.
    pub fn get(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<&Value>> {
        self.value(checked(keyword)?)
    }

    /// The value of the first record with `keyword`; [`FitsError::InvalidValue`] when
    /// that record is malformed.
    fn value(&self, keyword: Keyword) -> Result<Option<&Value>> {
        let Some(&i) = self.index.get(&keyword) else {
            return Ok(None);
        };
        self.records[i].value_or_defect().map(Some)
    }

    /// The inline comment of the first valued card with this keyword; `None` when the
    /// card has none or the header holds no such card. The read twin of
    /// [`Header::set_comment`].
    pub fn get_comment(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<&str>> {
        let keyword = checked(keyword)?;
        let Some(&i) = self.index.get(&keyword) else {
            return Ok(None);
        };
        self.records[i].value_or_defect()?;
        Ok(self.records[i].comment())
    }

    /// Read an optional logical keyword without conflating absence with a wrong type.
    pub fn get_logical(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<bool>> {
        self.typed_optional(checked(keyword)?, ValueKind::Logical, |value| {
            Ok(value.as_logical())
        })
    }

    /// Read an optional integer keyword without conflating absence with a wrong type.
    pub fn get_integer(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<i64>> {
        self.typed_optional(checked(keyword)?, ValueKind::Integer, Value::as_integer)
    }

    /// Read an optional real keyword without conflating absence with a wrong type.
    pub fn get_real(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<f64>> {
        self.typed_optional(checked(keyword)?, ValueKind::Real, Value::as_real)
    }

    /// Read an optional text keyword without conflating absence with a wrong type.
    pub fn get_text(
        &self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<Option<&str>> {
        self.typed_optional(checked(keyword)?, ValueKind::Text, |value| {
            Ok(value.as_text())
        })
    }

    fn typed_optional<'a, T>(
        &'a self,
        keyword: Keyword,
        expected: ValueKind,
        convert: impl FnOnce(&'a Value) -> Result<Option<T>>,
    ) -> Result<Option<T>> {
        let Some(value) = self.value(keyword)? else {
            return Ok(None);
        };
        convert(value)
            .map_err(|error| match error {
                FitsError::IntegerOutOfRange { value, target, .. } => {
                    FitsError::IntegerOutOfRange {
                        keyword: Some(keyword),
                        value,
                        target,
                    }
                }
                other => other,
            })?
            .map(Some)
            .ok_or(FitsError::ValueTypeMismatch {
                name: RecordName::Keyword(keyword),
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
        self.records.len()
    }

    pub const fn is_empty(&self) -> bool {
        self.records.is_empty()
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
        // The index holds the first valued record; a commentary record of the name can
        // only come before it.
        let end = self
            .index
            .get(&keyword)
            .map_or(self.records.len(), |&i| i + 1);
        Ok(self.records[..end]
            .iter()
            .position(|record| record.is_named(keyword)))
    }

    /// Every stored record in file order, as a [`RecordView`] — duplicates and order
    /// preserved (the whole point of the ordered model), so `COMMENT`/`HISTORY` runs
    /// and repeated keywords come through intact. The implicit `END` is not a record
    /// and is never yielded. A valued record is [`RecordView::Valued`], with its
    /// [`Keyword`].
    pub fn iter(&self) -> Iter<'_> {
        Iter {
            records: self.records.iter(),
        }
    }

    /// The text of each `HISTORY` record, in order; an empty record gives `""`.
    pub fn history_records(&self) -> impl Iterator<Item = &str> {
        self.commentary(CommentaryName::HISTORY)
    }

    /// The text of each `COMMENT` record, in order; an empty record gives `""`.
    pub fn comment_records(&self) -> impl Iterator<Item = &str> {
        self.commentary(CommentaryName::COMMENT)
    }

    fn commentary(&self, wanted: CommentaryName) -> impl Iterator<Item = &str> {
        self.records.iter().filter_map(move |record| match record {
            Record::Commentary { name, text } if *name == wanted => {
                Some(text.as_deref().unwrap_or(""))
            }
            _ => None,
        })
    }

    /// `BITPIX`, mapped to the typed element kind.
    pub(crate) fn bitpix(&self) -> Result<Bitpix> {
        let code = self.require_integer(keywords::BITPIX)?;
        Bitpix::from_code(code)
    }

    /// `NAXIS` — the number of axes (0 means no data array).
    pub(crate) fn naxis(&self) -> Result<usize> {
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
    pub(crate) fn axes(&self) -> Result<Vec<usize>> {
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
        validate_valued_keyword(keyword)?;
        let record = Record::Valued {
            keyword,
            value: value.into(),
            comment: None,
        };
        let Some(&i) = self.index.get(&keyword) else {
            card::validate(&record)?;
            self.push_record(record);
            return Ok(self);
        };
        let Record::Valued { value, .. } = record else {
            unreachable!("built as a valued record above");
        };
        self.replace_value(i, value)?;
        Ok(self)
    }

    /// Give the indexed record at `i` the `value`: a valued or `HIERARCH` record keeps
    /// its comment, and a malformed one is replaced in its place by a record of its name.
    /// An error leaves the record as it was.
    fn replace_value(&mut self, i: usize, value: Value) -> Result<()> {
        let replacement = match &self.records[i] {
            Record::Valued {
                keyword, comment, ..
            } => Record::Valued {
                keyword: *keyword,
                value,
                comment: comment.clone(),
            },
            Record::Hierarch { name, comment, .. } => Record::Hierarch {
                name: name.clone(),
                value,
                comment: comment.clone(),
            },
            Record::Malformed {
                name: Some(RecordName::Keyword(keyword)),
                ..
            } => Record::Valued {
                keyword: *keyword,
                value,
                comment: None,
            },
            Record::Malformed {
                name: Some(RecordName::Hierarch(name)),
                ..
            } => Record::Hierarch {
                name: name.clone(),
                value,
                comment: None,
            },
            Record::Commentary { .. } | Record::Malformed { .. } => {
                unreachable!("the indexes hold valued, HIERARCH and named malformed records")
            }
        };
        card::validate(&replacement)?;
        self.records[i] = replacement;
        Ok(())
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
        self.insert(self.records.len(), keyword, value)
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
        self.insert_record(
            index,
            Record::Valued {
                keyword,
                value: value.into(),
                comment: None,
            },
        )
    }

    /// Insert `record` at the logical position `index`: the inverse of
    /// [`Header::remove_at`]. A malformed record goes in as it is, as a parse keeps one;
    /// any other must render.
    pub fn insert_record(&mut self, index: usize, record: Record) -> Result<&mut Self> {
        if index > self.records.len() {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::HeaderRecord,
                index,
                len: self.records.len(),
            });
        }
        if let Record::Valued { keyword, .. } = &record {
            validate_valued_keyword(*keyword)?;
        }
        if !matches!(record, Record::Malformed { .. }) {
            card::validate(&record)?;
        }
        self.insert_at(index, record);
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
        if index >= self.records.len() {
            return Err(FitsError::IndexOutOfBounds {
                indexed: Indexed::HeaderRecord,
                index,
                len: self.records.len(),
            });
        }
        Ok(self.take_at(index))
    }

    /// Remove every logical record with `keyword`, returning the count removed.
    pub fn remove_all(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
    ) -> Result<usize> {
        let keyword = checked(keyword)?;
        let old_len = self.records.len();
        self.records.retain(|record| !record.is_named(keyword));
        if self.records.len() != old_len {
            self.reindex();
        }
        Ok(old_len - self.records.len())
    }

    /// Append the records of `source` that are named by no keyword, and those whose
    /// keyword `keep` keeps.
    pub(crate) fn append_filtered_from(
        &mut self,
        source: &Self,
        mut keep: impl FnMut(Keyword) -> bool,
    ) {
        self.records.extend(
            source
                .records
                .iter()
                .filter(|record| record.keyword().is_none_or(&mut keep))
                .cloned(),
        );
        self.reindex();
    }

    /// Remove the records whose keyword `should_remove` names, then rebuild the indexes
    /// once after the bulk edit. A record named by no keyword stays.
    pub(crate) fn remove_where(
        &mut self,
        mut should_remove: impl FnMut(Keyword) -> bool,
    ) -> &mut Self {
        let old_len = self.records.len();
        self.records
            .retain(|record| !record.keyword().is_some_and(&mut should_remove));
        if self.records.len() != old_len {
            self.reindex();
        }
        self
    }

    /// Rename valued cards in place so their order, values, and comments survive
    /// a reversible metadata transformation.
    #[cfg(feature = "compression")]
    pub(crate) fn rename_keywords(&mut self, renames: &[(Keyword, Keyword)]) {
        let mut changed = false;
        for record in &mut self.records {
            let Record::Valued { keyword, .. } = record else {
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

    /// Append `record`, indexing it where it is the first of its name.
    fn push_record(&mut self, record: Record) {
        self.insert_at(self.records.len(), record);
    }

    /// Insert `record` at `i`, shifting the indexed positions after it, and index it
    /// where it is now the first of its name.
    fn insert_at(&mut self, i: usize, record: Record) {
        if i < self.records.len() {
            for position in self
                .index
                .values_mut()
                .chain(self.hierarch_index.values_mut())
            {
                if *position >= i {
                    *position += 1;
                }
            }
        }
        if let Some(keyword) = record.keyword() {
            let first = self.index.entry(keyword).or_insert(i);
            *first = (*first).min(i);
        }
        if let Some(name) = record.hierarch_name() {
            match self.hierarch_index.get_mut(name) {
                Some(first) => *first = (*first).min(i),
                None => {
                    self.hierarch_index.insert(name.clone(), i);
                }
            }
        }
        self.records.insert(i, record);
    }

    /// Remove the record at `i`, shifting the indexed positions after it, and index the
    /// next record of its name where it was the first.
    fn take_at(&mut self, i: usize) -> Record {
        let record = self.records.remove(i);
        for position in self
            .index
            .values_mut()
            .chain(self.hierarch_index.values_mut())
        {
            if *position > i {
                *position -= 1;
            }
        }
        if let Some(keyword) = record.keyword()
            && self.index.get(&keyword) == Some(&i)
        {
            match self.records[i..]
                .iter()
                .position(|next| next.keyword() == Some(keyword))
            {
                Some(offset) => {
                    self.index.insert(keyword, i + offset);
                }
                None => {
                    self.index.remove(&keyword);
                }
            }
        }
        if let Some(name) = record.hierarch_name()
            && self.hierarch_index.get(name) == Some(&i)
        {
            match self.records[i..]
                .iter()
                .position(|next| next.hierarch_name() == Some(name))
            {
                Some(offset) => {
                    self.hierarch_index.insert(name.clone(), i + offset);
                }
                None => {
                    self.hierarch_index.remove(name);
                }
            }
        }
        record
    }

    /// Rebuild both indexes after a bulk edit.
    fn reindex(&mut self) {
        self.index.clear();
        self.hierarch_index.clear();
        for (i, record) in self.records.iter().enumerate() {
            if let Some(keyword) = record.keyword() {
                self.index.entry(keyword).or_insert(i);
            }
            if let Some(name) = record.hierarch_name()
                && !self.hierarch_index.contains_key(name)
            {
                self.hierarch_index.insert(name.clone(), i);
            }
        }
    }

    /// The value of the first `HIERARCH` record with `name`, compared without regard to
    /// ASCII case; [`FitsError::InvalidValue`] when that record is malformed.
    pub fn get_hierarch(
        &self,
        name: impl TryInto<HierarchName, Error: Into<FitsError>>,
    ) -> Result<Option<&Value>> {
        let name = name.try_into().map_err(Into::into)?;
        self.hierarch_index
            .get(&name)
            .map(|&i| self.records[i].value_or_defect())
            .transpose()
    }

    /// The inline comment of the first `HIERARCH` record with `name`, as
    /// [`Header::get_comment`] reads a keyword's.
    pub fn get_hierarch_comment(
        &self,
        name: impl TryInto<HierarchName, Error: Into<FitsError>>,
    ) -> Result<Option<&str>> {
        let name = name.try_into().map_err(Into::into)?;
        let Some(&i) = self.hierarch_index.get(&name) else {
            return Ok(None);
        };
        self.records[i].value_or_defect()?;
        Ok(self.records[i].comment())
    }

    /// Set the value of the first `HIERARCH` record with `name`, or append one; a
    /// malformed record of the name is replaced in its place. Text must use restricted
    /// ASCII, and a long one continues on `CONTINUE` records. An error leaves the header
    /// unchanged.
    pub fn set_hierarch(
        &mut self,
        name: impl TryInto<HierarchName, Error: Into<FitsError>>,
        value: impl Into<Value>,
    ) -> Result<&mut Self> {
        let name = name.try_into().map_err(Into::into)?;
        let value = value.into();
        if let Some(&i) = self.hierarch_index.get(&name) {
            self.replace_value(i, value)?;
        } else {
            let record = Record::Hierarch {
                name,
                value,
                comment: None,
            };
            card::validate(&record)?;
            self.push_record(record);
        }
        Ok(self)
    }

    /// Remove the first `HIERARCH` record with `name`, malformed or not.
    pub fn remove_hierarch(
        &mut self,
        name: impl TryInto<HierarchName, Error: Into<FitsError>>,
    ) -> Result<Option<Record>> {
        let name = name.try_into().map_err(Into::into)?;
        Ok(self
            .hierarch_index
            .get(&name)
            .copied()
            .map(|i| self.take_at(i)))
    }

    /// Attach (or replace) the inline comment of an existing valued keyword. Errors
    /// with [`FitsError::MissingKeyword`] for a keyword the header does not hold.
    /// An error leaves the header unchanged.
    pub fn set_comment(
        &mut self,
        keyword: impl TryInto<Keyword, Error: Into<FitsError>>,
        text: &str,
    ) -> Result<&mut Self> {
        let keyword = checked(keyword)?;
        validate_ascii(text, || TextOrigin::Comment(RecordName::Keyword(keyword)))?;
        let Some(&i) = self.index.get(&keyword) else {
            return Err(FitsError::MissingKeyword { keyword });
        };
        let Record::Valued { value, .. } = &self.records[i] else {
            self.records[i].value_or_defect()?;
            unreachable!("the keyword index holds valued and malformed records only");
        };
        let record = Record::Valued {
            keyword,
            value: value.clone(),
            comment: Some(text.to_string()),
        };
        card::validate(&record)?;
        self.records[i] = record;
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
        self.push_commentary(CommentaryName::COMMENT, text)
    }

    /// Append one restricted-ASCII `HISTORY` record. Text that cannot fit the
    /// record returns an error without changing the header.
    pub fn push_history(&mut self, text: &str) -> Result<&mut Self> {
        self.push_commentary(CommentaryName::HISTORY, text)
    }

    /// Append a blank-keyword commentary record. `text = ""` produces a physically
    /// blank card; nonempty text uses the same restricted-ASCII rules.
    pub fn push_blank(&mut self, text: &str) -> Result<&mut Self> {
        self.push_commentary(CommentaryName::BLANK, text)
    }

    fn push_commentary(&mut self, name: CommentaryName, text: &str) -> Result<&mut Self> {
        let record = Record::commentary(name, text);
        card::validate(&record)?;
        self.records.push(record);
        Ok(self)
    }

    /// Serialize this header unit into `out`, cleared first: every record rendered to 80
    /// bytes, blank records to fill `space`, the `END` record, then blank fill to the next
    /// 2880-byte boundary. The blank records are space a later edit can fill in place, as
    /// cfitsio's `ffhdef` reserves it. `signing` sets `DATASUM` and `CHECKSUM` in their
    /// place, or after the records when the header has none, without changing `self`.
    pub(crate) fn render_into(
        &self,
        signing: Signing,
        space: Space,
        out: &mut Vec<u8>,
    ) -> Result<()> {
        out.clear();
        let checksum_at = self.render_records(signing, out)?;
        let records = out.len() / CARD_SIZE;
        let spare = match space {
            Space::Spare(spare) => spare,
            Space::Blocks(blocks) => {
                let needed = (records + 1).div_ceil(RECORDS_PER_BLOCK);
                if needed > blocks {
                    return Err(FitsError::HeaderDoesNotFit {
                        needed,
                        available: blocks,
                    });
                }
                blocks * RECORDS_PER_BLOCK - records - 1
            }
        };
        let len = records
            .checked_add(spare)
            .and_then(|records| records.checked_add(1))
            .and_then(|records| records.checked_mul(CARD_SIZE))
            .ok_or(FitsError::DataUnitOverflow)?;
        out.reserve(len.next_multiple_of(BLOCK_SIZE) - out.len());
        out.resize(out.len() + spare * CARD_SIZE, b' ');
        let mut end = [b' '; CARD_SIZE];
        end[..3].copy_from_slice(b"END");
        out.extend_from_slice(&end);
        out.resize(out.len().next_multiple_of(BLOCK_SIZE), b' ');
        if let (Signing::WithDataSum(data_sum), Some(at)) = (signing, checksum_at) {
            let hdu_sum = checksum::combine(checksum::accumulate(out, 0), data_sum);
            // The value of `CHECKSUM = '…'` sits in bytes 12–27 of its record.
            out[at + 11..at + 27].copy_from_slice(&checksum::encode(hdu_sum));
        }
        Ok(())
    }

    /// The records a render of this header writes before its free records and `END`,
    /// unsigned: a long string takes one record for each `CONTINUE` part.
    pub(crate) fn rendered_records(&self) -> Result<usize> {
        let mut out = Vec::new();
        self.render_records(Signing::Unsigned, &mut out)?;
        Ok(out.len() / CARD_SIZE)
    }

    /// Append every record rendered with `signing` to `out`; the offset of the `CHECKSUM`
    /// record when `signing` signs.
    fn render_records(&self, signing: Signing, out: &mut Vec<u8>) -> Result<Option<usize>> {
        let signed = |keyword| match signing {
            Signing::Unsigned => None,
            Signing::Placeholder => Some(if keyword == keywords::DATASUM {
                "0".to_string()
            } else {
                checksum::PLACEHOLDER.to_string()
            }),
            Signing::WithDataSum(sum) => Some(if keyword == keywords::DATASUM {
                sum.to_string()
            } else {
                checksum::PLACEHOLDER.to_string()
            }),
        };
        let mut checksum_at = None;
        let mut render_signed =
            |keyword: Keyword, comment: Option<&str>, out: &mut Vec<u8>| -> Result<()> {
                let value = signed(keyword).expect("only a signed render signs");
                if keyword == keywords::CHECKSUM {
                    checksum_at = Some(out.len());
                }
                card::render_into(
                    &Record::Valued {
                        keyword,
                        value: Value::Text(value),
                        comment: comment.map(str::to_string),
                    },
                    out,
                )
            };
        let signs = signing != Signing::Unsigned;
        let datasum = self.index.get(&keywords::DATASUM).copied();
        let checksum = self.index.get(&keywords::CHECKSUM).copied();
        for (i, record) in self.records.iter().enumerate() {
            match record.keyword() {
                Some(keyword @ (keywords::DATASUM | keywords::CHECKSUM))
                    if signs && [datasum, checksum].contains(&Some(i)) =>
                {
                    render_signed(keyword, record.comment(), out)?;
                }
                _ => card::render_into(record, out)?,
            }
        }
        if signs {
            for (keyword, at) in [(keywords::DATASUM, datasum), (keywords::CHECKSUM, checksum)] {
                if at.is_none() {
                    render_signed(keyword, None, out)?;
                }
            }
        }
        Ok(checksum_at)
    }
}

/// How a rendered header carries its checksums (Appendix J).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Signing {
    /// As the records hold them, or not at all.
    Unsigned,
    /// `DATASUM = '0'` and a placeholder `CHECKSUM`: the records a stream writes before
    /// its data, the size of the signed header that replaces them.
    Placeholder,
    /// `DATASUM` is the data's sum, and `CHECKSUM` makes the HDU sum to negative zero.
    WithDataSum(u32),
}

/// The blank records a rendered header holds before `END`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Space {
    /// This many.
    Spare(usize),
    /// As many as fill exactly this many blocks; `HeaderDoesNotFit` when the records
    /// need more.
    Blocks(usize),
}

/// Two headers are equal when they hold the same records in the same order; the
/// repairs that a parse reports are not content.
impl PartialEq for Header {
    fn eq(&self, other: &Self) -> bool {
        self.records == other.records
    }
}

/// Every record of `iter`, appended as [`Header::insert_record`] appends one.
///
/// # Panics
/// When a record is not malformed and does not render: an invalid value or name, which
/// a record taken from a header never has.
impl Extend<Record> for Header {
    fn extend<I: IntoIterator<Item = Record>>(&mut self, iter: I) {
        for record in iter {
            let len = self.records.len();
            self.insert_record(len, record)
                .expect("an appended record renders");
        }
    }
}

/// The keyword `name` names: a [`Keyword`] as it is, a `&str` checked into one.
fn checked(name: impl TryInto<Keyword, Error: Into<FitsError>>) -> Result<Keyword> {
    name.try_into().map_err(Into::into)
}

impl<'a> IntoIterator for &'a Header {
    type Item = RecordView<'a>;
    type IntoIter = Iter<'a>;

    fn into_iter(self) -> Iter<'a> {
        self.iter()
    }
}

/// The records of a [`Header`], in order, as [`RecordView`] views.
#[derive(Debug, Clone)]
pub struct Iter<'a> {
    records: slice::Iter<'a, Record>,
}

impl<'a> Iterator for Iter<'a> {
    type Item = RecordView<'a>;

    fn next(&mut self) -> Option<RecordView<'a>> {
        self.records.next().map(Record::view)
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        self.records.size_hint()
    }
}

impl ExactSizeIterator for Iter<'_> {}

/// Fold a `CONTINUE` substring into the preceding long-string value card,
/// returning `false` when the previous card is not a value awaiting continuation
/// (i.e. a [`Value::Text`] whose text ends with the `&` continuation flag).
fn fold_continuation(records: &mut [Record], substring: &str, fragment: Option<&str>) -> bool {
    let Some(
        Record::Valued {
            value: Value::Text(acc),
            comment,
            ..
        }
        | Record::Hierarch {
            value: Value::Text(acc),
            comment,
            ..
        },
    ) = records.last_mut()
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

#[cfg(test)]
mod tests;
