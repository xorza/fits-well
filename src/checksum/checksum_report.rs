//! [`ChecksumReport`]: the verdict on an HDU's `DATASUM` and `CHECKSUM`.

/// The state of one FITS checksum keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChecksumStatus {
    /// The keyword is not present, so the HDU asserts no checksum knowledge.
    Absent,
    /// The keyword is empty or holds only blanks: an undefined or unknown value (§4.4.2.7).
    Unknown,
    /// The asserted checksum matches the recomputed value.
    Valid,
    /// The value is malformed or does not match the recomputed checksum.
    Invalid,
}

/// Result of [`FitsReader::verify_checksum`](crate::FitsReader::verify_checksum).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChecksumReport {
    pub datasum: ChecksumStatus,
    pub checksum: ChecksumStatus,
}

impl ChecksumReport {
    /// Whether both `DATASUM` and `CHECKSUM` are present and verify.
    pub const fn is_valid(&self) -> bool {
        matches!(
            (self.datasum, self.checksum),
            (ChecksumStatus::Valid, ChecksumStatus::Valid)
        )
    }

    /// Whether `DATASUM` or `CHECKSUM` is malformed or does not match its recomputed
    /// value.
    pub const fn has_invalid(&self) -> bool {
        matches!(self.datasum, ChecksumStatus::Invalid)
            || matches!(self.checksum, ChecksumStatus::Invalid)
    }
}
