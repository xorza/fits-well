//! Header records for tests: cards from text, files from cards, and lookups in rendered bytes.

use std::ops::Range;

use crate::block::{BLOCK_SIZE, CARD_SIZE};
use crate::checksum::checksum_report::{ChecksumReport, ChecksumStatus};
use crate::header_model::value::Value;
use crate::header_model::{Header, Signing, Space, keywords};

/// The report of an HDU whose `DATASUM` and `CHECKSUM` both verify.
pub(crate) const VALID: ChecksumReport = report(ChecksumStatus::Valid, ChecksumStatus::Valid);

/// The report of `DATASUM` and `CHECKSUM` statuses `datasum` and `checksum`.
pub(crate) const fn report(datasum: ChecksumStatus, checksum: ChecksumStatus) -> ChecksumReport {
    ChecksumReport { datasum, checksum }
}

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

/// The mandatory records (§4.4.1): `SIMPLE = T` of a primary or the extension's
/// `XTENSION`, `BITPIX`, `NAXIS` and one `NAXISn` for each of `axes`, and for an extension
/// `PCOUNT = 0` and `GCOUNT = 1`.
pub(crate) fn structural(xtension: Option<&str>, bitpix: i64, axes: &[i64]) -> Header {
    let mut header = Header::new();
    match xtension {
        Some(xtension) => header.set_internal(keywords::XTENSION, xtension),
        None => header.set_internal(keywords::SIMPLE, true),
    };
    header
        .set_internal(keywords::BITPIX, bitpix)
        .set_internal(keywords::NAXIS, i64::try_from(axes.len()).unwrap());
    for (index, &length) in axes.iter().enumerate() {
        header.set_internal(keywords::NAXIS.indexed(index + 1).unwrap(), length);
    }
    if xtension.is_some() {
        header
            .set_internal(keywords::PCOUNT, 0)
            .set_internal(keywords::GCOUNT, 1);
    }
    header
}

/// The header of `cards`, in order.
pub(crate) fn header_of(cards: &[(&str, Value)]) -> Header {
    let mut header = Header::new();
    for (keyword, value) in cards {
        header.set_internal(*keyword, value.clone());
    }
    header
}

/// Assert that `header` holds each of `cards`.
#[track_caller]
pub(crate) fn assert_cards(header: &Header, cards: &[(&str, Value)]) {
    for (keyword, value) in cards {
        assert_eq!(header.get(*keyword).unwrap(), Some(value), "{keyword}");
    }
}

/// Assert that `header` holds none of `keywords`.
#[track_caller]
pub(crate) fn assert_absent(header: &Header, keywords: &[&str]) {
    for keyword in keywords {
        assert_eq!(header.get(*keyword).unwrap(), None, "{keyword}");
    }
}

/// A one-HDU file: the records of `cards` and `END`, blank-filled to the block, then `data`,
/// zero-filled to the block.
pub(crate) fn fits_file(cards: &[&str], data: &[u8]) -> Vec<u8> {
    let mut file = records(cards);
    file.extend_from_slice(&card_bytes("END"));
    file.resize(file.len().next_multiple_of(BLOCK_SIZE), b' ');
    file.extend_from_slice(data);
    file.resize(file.len().next_multiple_of(BLOCK_SIZE), 0);
    file
}

/// The records `header` renders to, with no checksum and no spare record.
pub(crate) fn rendered(header: &Header) -> Vec<u8> {
    let mut bytes = Vec::new();
    header
        .render_into(Signing::Unsigned, Space::Spare(0), &mut bytes)
        .unwrap();
    bytes
}

/// The records of rendered `bytes` as text, trailing blanks dropped.
pub(crate) fn record_lines(bytes: &[u8]) -> impl Iterator<Item = &str> {
    bytes
        .as_chunks::<CARD_SIZE>()
        .0
        .iter()
        .map(|record| str::from_utf8(record).unwrap().trim_end())
}

/// The index of the first record of `bytes` whose name field is `keyword`.
pub(crate) fn card_index(bytes: &[u8], keyword: &str) -> usize {
    assert!(
        keyword.len() <= 8,
        "{keyword:?} is longer than a name field"
    );
    bytes
        .as_chunks::<CARD_SIZE>()
        .0
        .iter()
        .position(|card| {
            card[..keyword.len()] == *keyword.as_bytes()
                && card[keyword.len()..8].iter().all(|&b| b == b' ')
        })
        .unwrap_or_else(|| panic!("no {keyword} record"))
}

/// The fixed-format value field of a record, columns 11 to 30 (§4.2.1).
const VALUE_COLUMNS: Range<usize> = 10..30;

/// The bytes of rendered `bytes` that the value field of the first record named
/// `keyword` takes.
pub(crate) fn value_field(bytes: &[u8], keyword: &str) -> Range<usize> {
    let at = card_index(bytes, keyword) * CARD_SIZE;
    at + VALUE_COLUMNS.start..at + VALUE_COLUMNS.end
}

/// The bytes of rendered `bytes` that `len` characters of the string value of the first
/// record named `keyword` take, inside its opening quote in column 11.
pub(crate) fn quoted_field(bytes: &[u8], keyword: &str, len: usize) -> Range<usize> {
    let at = card_index(bytes, keyword) * CARD_SIZE + VALUE_COLUMNS.start + 1;
    at..at + len
}
