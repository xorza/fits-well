//! [`Repair`]: a near-valid header record the parse read as the record it means.

/// One repair `Header::parse` made, reported by `Header::repairs`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Repair {
    /// The 1-based position of the record in the header unit as read.
    pub record: usize,
    pub kind: RepairKind,
}

/// What a repair changed, as cfitsio and astropy read such a record.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepairKind {
    /// A lowercase name read as its uppercase keyword.
    LowercaseName,
    /// A value indicator `=` before byte 9, or in byte 9 with no blank in byte 10,
    /// read with the value that follows it.
    ValueIndicator,
    /// A non-ASCII byte in a comment or in commentary text read as `?`.
    NonAsciiText,
    /// An `END` record with bytes other than blanks after its name, read as `END`.
    EndText,
}
