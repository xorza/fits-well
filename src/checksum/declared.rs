//! [`Declared`]: what a header says of its `DATASUM` and `CHECKSUM`.

use crate::checksum;
use crate::checksum::checksum_report::{ChecksumReport, ChecksumStatus};
use crate::fits_error::Result;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::keyword::Keyword;

/// A header's `DATASUM` and `CHECKSUM`, parsed once by the one rule the reader and the
/// editor share: an empty or all-blank value is unknown (§4.4.2.7), as cfitsio `ffvcks`
/// reads it; a `DATASUM` that is no 32-bit unsigned decimal is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Declared {
    datasum: Declaration,
    checksum: Declaration,
}

/// One checksum keyword, as a header declares it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Declaration {
    Absent,
    Unknown,
    Invalid,
    /// A `DATASUM`'s sum, or `0` for a `CHECKSUM`, whose value only its HDU's sum checks.
    Present(u32),
}

impl Declared {
    pub(crate) fn read(header: &Header) -> Self {
        Self {
            datasum: Declaration::read(header, keywords::DATASUM, |text| text.parse::<u32>().ok()),
            checksum: Declaration::read(header, keywords::CHECKSUM, |_| Some(0)),
        }
    }

    /// Whether the header holds a `CHECKSUM`, which a write must sign.
    pub(crate) const fn has_checksum(&self) -> bool {
        !matches!(self.checksum, Declaration::Absent)
    }

    /// The data sum the header's `DATASUM` declares, when it declares a valid one.
    pub(crate) const fn data_sum(&self) -> Option<u32> {
        match self.datasum {
            Declaration::Present(sum) => Some(sum),
            Declaration::Absent | Declaration::Unknown | Declaration::Invalid => None,
        }
    }

    /// The verdict on an HDU whose header sums to `header_sum`, `data_sum` giving its data
    /// unit's sum; `data_sum` runs only when a keyword needs it.
    pub(crate) fn report(
        &self,
        header_sum: u32,
        data_sum: impl FnOnce() -> Result<u32>,
    ) -> Result<ChecksumReport> {
        let needs_data = matches!(self.datasum, Declaration::Present(_))
            || matches!(self.checksum, Declaration::Present(_));
        let data_sum = if needs_data { Some(data_sum()?) } else { None };
        Ok(ChecksumReport {
            datasum: self.datasum.status(self.data_sum() == data_sum),
            checksum: self.checksum.status(
                data_sum.is_some_and(|data| checksum::combine(header_sum, data) == 0xFFFF_FFFF),
            ),
        })
    }
}

impl Declaration {
    /// The status of this declaration, a present one `valid` or not.
    const fn status(self, valid: bool) -> ChecksumStatus {
        match self {
            Self::Absent => ChecksumStatus::Absent,
            Self::Unknown => ChecksumStatus::Unknown,
            Self::Invalid => ChecksumStatus::Invalid,
            Self::Present(_) if valid => ChecksumStatus::Valid,
            Self::Present(_) => ChecksumStatus::Invalid,
        }
    }

    /// `keyword` of `header`: its text, blank-trimmed and parsed by `parse`, or an integer
    /// value that fits `u32`, which some writers give without quotes.
    fn read(header: &Header, keyword: Keyword, parse: impl Fn(&str) -> Option<u32>) -> Self {
        match header.get(keyword) {
            Ok(None) => Self::Absent,
            Ok(Some(Value::Text(text))) if text.trim().is_empty() => Self::Unknown,
            Ok(Some(Value::Text(text))) => parse(text.trim()).map_or(Self::Invalid, Self::Present),
            Ok(Some(value)) if keyword == keywords::DATASUM => value
                .as_integer()
                .ok()
                .flatten()
                .and_then(|sum| u32::try_from(sum).ok())
                .map_or(Self::Invalid, Self::Present),
            Ok(Some(_)) | Err(_) => Self::Invalid,
        }
    }
}
