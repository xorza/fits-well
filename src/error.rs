use std::fmt;
use std::io;

use thiserror::Error;

pub type Result<T> = std::result::Result<T, FitsError>;

/// What an out-of-range index was addressing, naming the bound it exceeded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Indexed {
    /// An HDU within the file's scanned sequence.
    Hdu,
    /// A logical record within a header unit.
    HeaderRecord,
    /// A column within a table.
    Column,
    /// A group within a random-groups array.
    Group,
    /// A **1-based** axis within a WCS description.
    WcsAxis,
}

impl Indexed {
    fn fmt_out_of_bounds(
        &self,
        index: &usize,
        len: &usize,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Indexed::Hdu => write!(f, "HDU index {index} out of bounds (file has {len} HDUs)"),
            Indexed::HeaderRecord => write!(
                f,
                "header record index {index} out of bounds (header has {len} records)"
            ),
            Indexed::Column => write!(
                f,
                "column index {index} out of bounds (table has {len} columns)"
            ),
            Indexed::Group => write!(
                f,
                "group index {index} out of bounds (random-groups array has {len} groups)"
            ),
            Indexed::WcsAxis => write!(
                f,
                "1-based WCS axis {index} out of bounds (WCS has {len} axes)"
            ),
        }
    }
}

/// Which of two axis counts disagreed — the thing the caller supplied, against the
/// structure it had to match.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum Ranked {
    /// An N-dimensional image region, against its image.
    ImageRegion,
    /// A compression tile shape, against the image it tiles.
    TileShape,
    /// A pixel or world coordinate, against its WCS.
    WcsCoordinate,
}

impl Ranked {
    fn fmt_mismatch(
        &self,
        expected: &usize,
        got: &usize,
        f: &mut fmt::Formatter<'_>,
    ) -> fmt::Result {
        match self {
            Ranked::ImageRegion => write!(
                f,
                "image region has {got} axes but the image has {expected}"
            ),
            Ranked::TileShape => {
                write!(f, "tile shape has {got} axes but the image has {expected}")
            }
            Ranked::WcsCoordinate => write!(
                f,
                "coordinate has {got} {} but the WCS has {expected} axes",
                if *got == 1 { "value" } else { "values" }
            ),
        }
    }
}

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum FitsError {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    /// A previous sink error may have left a partial HDU in the output. Further
    /// writes are rejected because appending cannot repair that FITS stream.
    #[error("writer is unusable after a previous output failure")]
    WriterFailed,
    /// A keyword name violated FITS syntax.
    #[error("invalid keyword name {name:?}")]
    InvalidKeyword { name: String },
    /// A control or commentary keyword was used where a valued card was requested.
    #[error("reserved keyword {name:?} cannot be used as a valued card")]
    ReservedKeyword { name: String },
    /// A header value has no conforming FITS representation.
    #[error("header keyword {keyword:?} has an invalid value: {reason}")]
    InvalidHeaderValue {
        keyword: String,
        reason: &'static str,
    },
    /// A logical header card cannot fit one physical 80-byte record.
    #[error("header card {keyword:?} needs {length} bytes but a FITS record holds 80")]
    HeaderCardTooLong { keyword: String, length: usize },
    /// A card's value field could not be parsed as any FITS value type.
    #[error("cannot parse value field of card {card:?}")]
    InvalidValue { card: String },
    /// Interpreting a valid FITS time value requires leap-second, Earth-orientation,
    /// or ephemeris data that this format library does not provide.
    #[error("{operation} requires external astronomical time data")]
    ExternalTimeDataRequired { operation: &'static str },
    /// A named metadata value was present but could not be interpreted as the type
    /// required by its FITS role.
    #[error("value {name} is not a valid {expected}")]
    TypeMismatch {
        name: String,
        expected: &'static str,
    },
    /// Text being written contains bytes outside FITS restricted ASCII (0x20–0x7e).
    #[error("{context} contains characters outside FITS restricted ASCII")]
    InvalidAscii { context: &'static str },
    /// An ASCII-table value cannot fit the field width declared by its `TFORMn`.
    #[error(
        "ASCII column {column:?} row {row} needs at least {minimum_width} characters but its field width is {width}"
    )]
    AsciiFieldTooWide {
        column: String,
        /// Zero-based table row containing the value.
        row: usize,
        width: usize,
        minimum_width: usize,
    },
    /// `BITPIX` held a value outside {8, 16, 32, 64, −32, −64}.
    #[error("invalid BITPIX value {code}")]
    InvalidBitpix { code: i64 },
    /// A header unit ended (ran out of cards) without an `END` record.
    #[error("header unit ended without an END record")]
    MissingEnd,
    /// A mandatory keyword was absent where the structure requires it.
    #[error("missing mandatory keyword {name}")]
    MissingKeyword { name: &'static str },
    /// A keyword was present and well-typed but its value lies outside the range
    /// the standard permits for its role (e.g. `NAXIS > 999`, `PCOUNT < 0`,
    /// a negative `GCOUNT` or axis length, or a `THEAP` that precedes the heap).
    #[error("keyword {name} has an out-of-range value")]
    KeywordOutOfRange { name: &'static str },
    /// An exact FITS integer cannot be represented by the requested bounded type.
    #[error("FITS integer {value} is outside the {target} range")]
    IntegerOutOfRange { value: String, target: &'static str },
    /// The byte stream ended in the middle of a header or data unit.
    #[error("unexpected end of stream inside a FITS unit")]
    UnexpectedEof,
    /// The data-unit size implied by the header overflows a 64-bit byte count
    /// (a malformed or hostile header with absurd `NAXISn`/`PCOUNT`/`GCOUNT`).
    #[error("header-implied data-unit size overflows 64 bits")]
    DataUnitOverflow,
    /// An output or staging buffer sized directly from untrusted FITS metadata is
    /// too large to allocate. Reader staging and final decompressed image/table
    /// planes allocate fallibly so hostile dimensions surface as this error rather
    /// than an out-of-memory process abort.
    #[error("header-implied data-unit size ({bytes} bytes) is too large to allocate")]
    DataUnitTooLarge { bytes: u64 },
    /// A decoded data unit held a different element count than the header's
    /// declared geometry — a corrupt or truncated data unit.
    #[error("decoded data unit has {got} elements, header implies {expected}")]
    DataSizeMismatch { expected: usize, got: usize },
    /// A signed `P`/`Q` heap-array descriptor contains a negative element count
    /// or byte offset.
    #[error("invalid P/Q descriptor {field}: {value}")]
    InvalidPqDescriptor { field: &'static str, value: i64 },
    /// An index named something beyond the bound of whatever it addressed —
    /// [`Indexed`] says which.
    #[error(fmt = Indexed::fmt_out_of_bounds)]
    IndexOutOfBounds {
        indexed: Indexed,
        index: usize,
        len: usize,
    },
    /// Two axis counts that had to agree did not — [`Ranked`] says which pair.
    #[error(fmt = Ranked::fmt_mismatch)]
    RankMismatch {
        ranked: Ranked,
        /// Axis count the structure requires.
        expected: usize,
        /// Axis count the caller supplied.
        got: usize,
    },
    /// One zero-based half-open image-axis range is reversed or exceeds its axis.
    #[error("image region axis {axis} range {start}..{end} exceeds axis length {len}")]
    ImageRegionOutOfBounds {
        axis: usize,
        start: usize,
        end: usize,
        len: usize,
    },
    /// A zero-based half-open table row range is reversed or exceeds the table.
    #[error("table row range {start}..{end} exceeds the table's {len} rows")]
    RowRangeOutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    /// A table column contains a different number of rows from the table being built.
    #[error("table column {column:?} has {got} rows but the table requires {expected}")]
    TableRowCountMismatch {
        column: String,
        expected: usize,
        got: usize,
    },
    /// Empty or zero-width column data did not carry enough information to infer
    /// the intended table row count.
    #[error(
        "table column {column:?} does not determine a row count; declare the table row count explicitly"
    )]
    TableRowCountUndetermined { column: String },
    /// An empty VLA column needs an explicit heap element type.
    #[error("empty VLA column {column:?} needs an explicit heap element type")]
    EmptyVlaNeedsType { column: String },
    /// A FITS keyword family was addressed with zero even though its indices start at 1.
    #[error("{kind} indices are 1-based and cannot be zero")]
    OneBasedIndexRequired { kind: &'static str },
    /// `read_image` was called on an HDU that is not an image array (a table,
    /// random-groups, or unmodelled extension).
    #[error("HDU is not an image array")]
    NotAnImage,
    /// An IMAGE/primary HDU carries group structure (`PCOUNT ≠ 0` or `GCOUNT ≠ 1`),
    /// which a plain image array must not have (§4.3).
    #[error("image HDU has group structure (PCOUNT ≠ 0 or GCOUNT ≠ 1)")]
    ImageHasGroups,
    /// `read_table` was called on an HDU that is not a binary table.
    #[error("HDU is not a binary table")]
    NotABinTable,
    /// `read_groups` was called on an HDU that is not a random-groups primary.
    #[error("HDU is not a random-groups primary")]
    NotRandomGroups,
    /// `read_ascii_table` was called on an HDU that is not an ASCII table.
    #[error("HDU is not an ASCII table")]
    NotAnAsciiTable,
    /// The decompressor was handed an HDU that is not a tiled-compressed image (no
    /// `ZIMAGE = T`). `read_image` guards this and returns [`FitsError::NotAnImage`]
    /// for a plain `BINTABLE`, so this surfaces only via the internal decode path.
    #[error("HDU is not a tiled-compressed image")]
    NotCompressedImage,
    /// `read_compressed_table` was called on an HDU that is not a tiled-compressed
    /// table (no `ZTABLE = T`).
    #[error("HDU is not a tiled-compressed table")]
    NotCompressedTable,
    /// Two mutually-exclusive WCS keyword conventions are both present (e.g. `PC`
    /// and `CD`, or `CROTA` and `PC`); a conforming header uses only one (§8).
    #[error("conflicting WCS keywords: {detail}")]
    ConflictingWcsKeywords { detail: &'static str },
    /// A complete pixel↔world transform was requested for axes whose nonlinear
    /// algorithm is not implemented. The indices are zero-based, matching
    /// [`crate::wcs::WcsView::unsupported_axes`].
    #[error("WCS has unsupported nonlinear transforms on zero-based axes {axes:?}")]
    UnsupportedWcsTransform { axes: Vec<usize> },
    /// A coordinate lies outside the mathematical domain of its WCS projection.
    #[error("coordinate is outside the {projection} projection domain")]
    WcsProjectionDomain { projection: &'static str },
    /// A world or intermediate coordinate lies outside a non-celestial WCS
    /// algorithm's mathematical domain.
    #[error("coordinate on zero-based axis {axis} is outside the {algorithm} WCS domain")]
    WcsCoordinateDomain {
        /// Zero-based WCS axis containing the invalid coordinate.
        axis: usize,
        algorithm: &'static str,
    },
    /// An iterative WCS algorithm did not reach a valid solution.
    #[error("{algorithm} WCS iteration did not converge")]
    WcsNoConvergence { algorithm: &'static str },
    /// A tiled-image compression algorithm or variant is not yet supported.
    #[error("unsupported tiled compression: {name}")]
    UnsupportedCompression { name: String },
    /// A PLIO tile sample cannot be represented losslessly in its unsigned 24-bit
    /// value domain.
    #[error("PLIO tile sample {index} has value {value}, outside 0..=16777215")]
    PlioValueOutOfRange { index: usize, value: i64 },
    /// A `TFORMn` value could not be parsed as a binary-table column format.
    #[error("invalid column format {tform:?}")]
    InvalidTform { tform: String },
    /// `ColumnReader::raw` was called on a variable-length-array (`P`/`Q`) column;
    /// use `ColumnReader::vla` instead.
    #[error("column format '{code}' is a variable-length array; use the column reader's vla()")]
    VariableLengthColumn { code: char },
    /// `ColumnReader::vla` was called on a fixed-width column.
    #[error("column format '{code}' is not a variable-length array")]
    NotAVla { code: char },
    /// `ColumnReader::bits` was called on a column that is not an `X` bit array.
    #[error("column format '{code}' is not an X bit array")]
    NotABitColumn { code: char },
    /// `ColumnReader::complex` was called on a column that is not `C`/`M` complex.
    #[error("column format '{code}' is not a C/M complex column")]
    NotAComplexColumn { code: char },
    /// `ColumnReader::physical` was called on a column with no numeric physical
    /// value (`A`/`L`/`X`/`C`/`M`).
    #[error("column format '{code}' has no numeric physical value")]
    NonNumericColumn { code: char },
    /// No column with the requested `TTYPEn` name exists in the table.
    #[error("no column named {name:?} in the table")]
    ColumnNotFound { name: String },
    /// The summed column widths disagree with the declared row width (`NAXIS1`).
    #[error("column widths sum to {computed} bytes but NAXIS1 declares {declared}")]
    RowWidthMismatch { computed: usize, declared: usize },
    /// Metadata supplied alongside a parsed binary table disagrees with the
    /// table's validated structure.
    #[error("header metadata {name} disagrees with the binary table")]
    TableMetadataMismatch { name: String },
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use crate::error::*;

    #[test]
    fn display_messages_are_specific() {
        assert_eq!(
            FitsError::InvalidBitpix { code: 7 }.to_string(),
            "invalid BITPIX value 7"
        );
        assert_eq!(
            FitsError::WriterFailed.to_string(),
            "writer is unusable after a previous output failure"
        );
        assert_eq!(
            FitsError::DataUnitOverflow.to_string(),
            "header-implied data-unit size overflows 64 bits"
        );
        assert_eq!(
            FitsError::DataUnitTooLarge { bytes: 1 << 60 }.to_string(),
            "header-implied data-unit size (1152921504606846976 bytes) is too large to allocate"
        );
        assert_eq!(
            FitsError::InvalidPqDescriptor {
                field: "offset",
                value: -1,
            }
            .to_string(),
            "invalid P/Q descriptor offset: -1"
        );
        assert_eq!(
            FitsError::MissingKeyword { name: "NAXIS" }.to_string(),
            "missing mandatory keyword NAXIS"
        );
        assert_eq!(
            FitsError::OneBasedIndexRequired {
                kind: "table column",
            }
            .to_string(),
            "table column indices are 1-based and cannot be zero"
        );
        assert_eq!(
            FitsError::IntegerOutOfRange {
                value: "9223372036854775808".to_string(),
                target: "i64",
            }
            .to_string(),
            "FITS integer 9223372036854775808 is outside the i64 range"
        );
        assert_eq!(
            FitsError::TypeMismatch {
                name: "ZSCALE".to_string(),
                expected: "f64 column",
            }
            .to_string(),
            "value ZSCALE is not a valid f64 column"
        );
        assert_eq!(
            FitsError::InvalidAscii {
                context: "table cell"
            }
            .to_string(),
            "table cell contains characters outside FITS restricted ASCII"
        );
        assert_eq!(
            FitsError::ReservedKeyword {
                name: "END".to_string(),
            }
            .to_string(),
            "reserved keyword \"END\" cannot be used as a valued card"
        );
        assert_eq!(
            FitsError::InvalidHeaderValue {
                keyword: "EXPTIME".to_string(),
                reason: "real values must be finite",
            }
            .to_string(),
            "header keyword \"EXPTIME\" has an invalid value: real values must be finite"
        );
        assert_eq!(
            FitsError::HeaderCardTooLong {
                keyword: "COMMENT".to_string(),
                length: 81,
            }
            .to_string(),
            "header card \"COMMENT\" needs 81 bytes but a FITS record holds 80"
        );
        assert_eq!(
            FitsError::AsciiFieldTooWide {
                column: "FLUX".to_string(),
                row: 2,
                width: 8,
                minimum_width: 9,
            }
            .to_string(),
            "ASCII column \"FLUX\" row 2 needs at least 9 characters but its field width is 8"
        );
        assert_eq!(
            FitsError::UnsupportedWcsTransform { axes: vec![0, 2] }.to_string(),
            "WCS has unsupported nonlinear transforms on zero-based axes [0, 2]"
        );
        assert_eq!(
            FitsError::WcsProjectionDomain { projection: "SIN" }.to_string(),
            "coordinate is outside the SIN projection domain"
        );
        assert_eq!(
            FitsError::WcsCoordinateDomain {
                axis: 2,
                algorithm: "LOG",
            }
            .to_string(),
            "coordinate on zero-based axis 2 is outside the LOG WCS domain"
        );
        assert_eq!(
            FitsError::WcsNoConvergence { algorithm: "ZPN" }.to_string(),
            "ZPN WCS iteration did not converge"
        );
        assert_eq!(
            FitsError::PlioValueOutOfRange {
                index: 3,
                value: 1 << 24,
            }
            .to_string(),
            "PLIO tile sample 3 has value 16777216, outside 0..=16777215"
        );
    }

    #[test]
    fn every_index_and_rank_arm_names_its_own_structure() {
        for (indexed, message) in [
            (Indexed::Hdu, "HDU index 3 out of bounds (file has 2 HDUs)"),
            (
                Indexed::HeaderRecord,
                "header record index 3 out of bounds (header has 2 records)",
            ),
            (
                Indexed::Column,
                "column index 3 out of bounds (table has 2 columns)",
            ),
            (
                Indexed::Group,
                "group index 3 out of bounds (random-groups array has 2 groups)",
            ),
            (
                Indexed::WcsAxis,
                "1-based WCS axis 3 out of bounds (WCS has 2 axes)",
            ),
        ] {
            assert_eq!(
                FitsError::IndexOutOfBounds {
                    indexed,
                    index: 3,
                    len: 2,
                }
                .to_string(),
                message
            );
        }

        // The last two rows differ only in `got`, so they pin the value/values branch.
        for (ranked, expected, got, message) in [
            (
                Ranked::ImageRegion,
                2,
                3,
                "image region has 3 axes but the image has 2",
            ),
            (
                Ranked::TileShape,
                2,
                3,
                "tile shape has 3 axes but the image has 2",
            ),
            (
                Ranked::WcsCoordinate,
                2,
                1,
                "coordinate has 1 value but the WCS has 2 axes",
            ),
            (
                Ranked::WcsCoordinate,
                3,
                2,
                "coordinate has 2 values but the WCS has 3 axes",
            ),
        ] {
            assert_eq!(
                FitsError::RankMismatch {
                    ranked,
                    expected,
                    got,
                }
                .to_string(),
                message
            );
        }
    }

    #[test]
    fn io_error_is_preserved_as_source() {
        let io_err = io::Error::new(io::ErrorKind::UnexpectedEof, "boom");
        let err = FitsError::from(io_err);
        assert!(matches!(err, FitsError::Io(_)));
        assert!(Error::source(&err).is_some());
    }
}
