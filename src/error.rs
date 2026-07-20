use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, FitsError>;

#[derive(Debug)]
#[non_exhaustive]
pub enum FitsError {
    Io(io::Error),
    /// A previous sink error may have left a partial HDU in the output. Further
    /// writes are rejected because appending cannot repair that FITS stream.
    WriterFailed,
    /// A keyword name violated FITS syntax.
    InvalidKeyword {
        name: String,
    },
    /// A control or commentary keyword was used where a valued card was requested.
    ReservedKeyword {
        name: String,
    },
    /// A header value has no conforming FITS representation.
    InvalidHeaderValue {
        keyword: String,
        reason: &'static str,
    },
    /// A logical header card cannot fit one physical 80-byte record.
    HeaderCardTooLong {
        keyword: String,
        length: usize,
    },
    /// A card's value field could not be parsed as any FITS value type.
    InvalidValue {
        card: String,
    },
    /// Interpreting a valid FITS time value requires leap-second, Earth-orientation,
    /// or ephemeris data that this format library does not provide.
    ExternalTimeDataRequired {
        operation: &'static str,
    },
    /// A named metadata value was present but could not be interpreted as the type
    /// required by its FITS role.
    TypeMismatch {
        name: String,
        expected: &'static str,
    },
    /// Text being written contains bytes outside FITS restricted ASCII (0x20–0x7e).
    InvalidAscii {
        context: &'static str,
    },
    /// An ASCII-table value cannot fit the field width declared by its `TFORMn`.
    AsciiFieldTooWide {
        column: String,
        /// Zero-based table row containing the value.
        row: usize,
        width: usize,
        minimum_width: usize,
    },
    /// `BITPIX` held a value outside {8, 16, 32, 64, −32, −64}.
    InvalidBitpix {
        code: i64,
    },
    /// A header unit ended (ran out of cards) without an `END` record.
    MissingEnd,
    /// A mandatory keyword was absent where the structure requires it.
    MissingKeyword {
        name: &'static str,
    },
    /// A keyword was present and well-typed but its value lies outside the range
    /// the standard permits for its role (e.g. `NAXIS > 999`, `PCOUNT < 0`,
    /// a negative `GCOUNT` or axis length, or a `THEAP` that precedes the heap).
    KeywordOutOfRange {
        name: &'static str,
    },
    /// An exact FITS integer cannot be represented by the requested bounded type.
    IntegerOutOfRange {
        value: String,
        target: &'static str,
    },
    /// The byte stream ended in the middle of a header or data unit.
    UnexpectedEof,
    /// The data-unit size implied by the header overflows a 64-bit byte count
    /// (a malformed or hostile header with absurd `NAXISn`/`PCOUNT`/`GCOUNT`).
    DataUnitOverflow,
    /// An output or staging buffer sized directly from untrusted FITS metadata is
    /// too large to allocate. Reader staging and final decompressed image/table
    /// planes allocate fallibly so hostile dimensions surface as this error rather
    /// than an out-of-memory process abort.
    DataUnitTooLarge {
        bytes: u64,
    },
    /// A decoded data unit held a different element count than the header's
    /// declared geometry — a corrupt or truncated data unit.
    DataSizeMismatch {
        expected: usize,
        got: usize,
    },
    /// A signed `P`/`Q` heap-array descriptor contains a negative element count
    /// or byte offset.
    InvalidPqDescriptor {
        field: &'static str,
        value: i64,
    },
    /// A data-unit read named an HDU index beyond the parsed sequence.
    HduIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// An ordered header operation named a record position beyond the stored
    /// logical record list.
    HeaderIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// An N-dimensional image region has a different rank from the image.
    ImageRegionRankMismatch {
        region_rank: usize,
        image_rank: usize,
    },
    /// One zero-based half-open image-axis range is reversed or exceeds its axis.
    ImageRegionOutOfBounds {
        axis: usize,
        start: usize,
        end: usize,
        len: usize,
    },
    /// A zero-based half-open table row range is reversed or exceeds the table.
    RowRangeOutOfBounds {
        start: usize,
        end: usize,
        len: usize,
    },
    /// A table column contains a different number of rows from the table being built.
    TableRowCountMismatch {
        column: String,
        expected: usize,
        got: usize,
    },
    /// Empty or zero-width column data did not carry enough information to infer
    /// the intended table row count.
    TableRowCountUndetermined {
        column: String,
    },
    /// An empty VLA column needs an explicit heap element type.
    EmptyVlaNeedsType {
        column: String,
    },
    /// A WCS transform received the wrong number of pixel or world coordinates.
    CoordinateCountMismatch {
        expected: usize,
        got: usize,
    },
    /// A 1-based FITS axis index named an axis outside the parsed WCS.
    WcsAxisIndexOutOfBounds {
        axis: usize,
        len: usize,
    },
    /// A FITS keyword family was addressed with zero even though its indices start at 1.
    OneBasedIndexRequired {
        kind: &'static str,
    },
    /// `read_image` was called on an HDU that is not an image array (a table,
    /// random-groups, or unmodelled extension).
    NotAnImage,
    /// An IMAGE/primary HDU carries group structure (`PCOUNT ≠ 0` or `GCOUNT ≠ 1`),
    /// which a plain image array must not have (§4.3).
    ImageHasGroups,
    /// `read_table` was called on an HDU that is not a binary table.
    NotABinTable,
    /// `read_groups` was called on an HDU that is not a random-groups primary.
    NotRandomGroups,
    /// `read_ascii_table` was called on an HDU that is not an ASCII table.
    NotAnAsciiTable,
    /// The decompressor was handed an HDU that is not a tiled-compressed image (no
    /// `ZIMAGE = T`). `read_image` guards this and returns [`FitsError::NotAnImage`]
    /// for a plain `BINTABLE`, so this surfaces only via the internal decode path.
    NotCompressedImage,
    /// `read_compressed_table` was called on an HDU that is not a tiled-compressed
    /// table (no `ZTABLE = T`).
    NotCompressedTable,
    /// Two mutually-exclusive WCS keyword conventions are both present (e.g. `PC`
    /// and `CD`, or `CROTA` and `PC`); a conforming header uses only one (§8).
    ConflictingWcsKeywords {
        detail: &'static str,
    },
    /// A complete pixel↔world transform was requested for axes whose nonlinear
    /// algorithm is not implemented. The indices are zero-based, matching
    /// [`crate::wcs::WcsView::unsupported_axes`].
    UnsupportedWcsTransform {
        axes: Vec<usize>,
    },
    /// A coordinate lies outside the mathematical domain of its WCS projection.
    WcsProjectionDomain {
        projection: &'static str,
    },
    /// A world or intermediate coordinate lies outside a non-celestial WCS
    /// algorithm's mathematical domain.
    WcsCoordinateDomain {
        /// Zero-based WCS axis containing the invalid coordinate.
        axis: usize,
        algorithm: &'static str,
    },
    /// An iterative WCS algorithm did not reach a valid solution.
    WcsNoConvergence {
        algorithm: &'static str,
    },
    /// A tiled-image compression algorithm or variant is not yet supported.
    UnsupportedCompression {
        name: String,
    },
    /// A PLIO tile sample cannot be represented losslessly in its unsigned 24-bit
    /// value domain.
    PlioValueOutOfRange {
        index: usize,
        value: i64,
    },
    /// A `TFORMn` value could not be parsed as a binary-table column format.
    InvalidTform {
        tform: String,
    },
    /// `ColumnReader::raw` was called on a variable-length-array (`P`/`Q`) column;
    /// use `ColumnReader::vla` instead.
    VariableLengthColumn {
        code: char,
    },
    /// `ColumnReader::vla` was called on a fixed-width column.
    NotAVla {
        code: char,
    },
    /// `ColumnReader::bits` was called on a column that is not an `X` bit array.
    NotABitColumn {
        code: char,
    },
    /// `ColumnReader::complex` was called on a column that is not `C`/`M` complex.
    NotAComplexColumn {
        code: char,
    },
    /// `ColumnReader::physical` was called on a column with no numeric physical
    /// value (`A`/`L`/`X`/`C`/`M`).
    NonNumericColumn {
        code: char,
    },
    /// A column index named a field beyond the table's column list.
    ColumnIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// A group index named an entry beyond a random-groups array's group list.
    GroupIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// No column with the requested `TTYPEn` name exists in the table.
    ColumnNotFound {
        name: String,
    },
    /// The summed column widths disagree with the declared row width (`NAXIS1`).
    RowWidthMismatch {
        computed: usize,
        declared: usize,
    },
    /// Metadata supplied alongside a parsed binary table disagrees with the
    /// table's validated structure.
    TableMetadataMismatch {
        name: String,
    },
    /// A requested compression tile shape has a different rank (axis count) than the
    /// image it tiles. Pass an empty shape for the default row-tiling instead.
    TileShapeRankMismatch {
        tile_rank: usize,
        image_rank: usize,
    },
}

impl fmt::Display for FitsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FitsError::Io(e) => write!(f, "I/O error: {e}"),
            FitsError::WriterFailed => {
                write!(f, "writer is unusable after a previous output failure")
            }
            FitsError::InvalidKeyword { name } => write!(f, "invalid keyword name {name:?}"),
            FitsError::ReservedKeyword { name } => {
                write!(
                    f,
                    "reserved keyword {name:?} cannot be used as a valued card"
                )
            }
            FitsError::InvalidHeaderValue { keyword, reason } => {
                write!(
                    f,
                    "header keyword {keyword:?} has an invalid value: {reason}"
                )
            }
            FitsError::HeaderCardTooLong { keyword, length } => write!(
                f,
                "header card {keyword:?} needs {length} bytes but a FITS record holds 80"
            ),
            FitsError::InvalidValue { card } => {
                write!(f, "cannot parse value field of card {card:?}")
            }
            FitsError::ExternalTimeDataRequired { operation } => {
                write!(f, "{operation} requires external astronomical time data")
            }
            FitsError::TypeMismatch { name, expected } => {
                write!(f, "value {name} is not a valid {expected}")
            }
            FitsError::InvalidAscii { context } => {
                write!(
                    f,
                    "{context} contains characters outside FITS restricted ASCII"
                )
            }
            FitsError::AsciiFieldTooWide {
                column,
                row,
                width,
                minimum_width,
            } => write!(
                f,
                "ASCII column {column:?} row {row} needs at least {minimum_width} characters but its field width is {width}"
            ),
            FitsError::InvalidBitpix { code } => write!(f, "invalid BITPIX value {code}"),
            FitsError::MissingEnd => write!(f, "header unit ended without an END record"),
            FitsError::MissingKeyword { name } => write!(f, "missing mandatory keyword {name}"),
            FitsError::KeywordOutOfRange { name } => {
                write!(f, "keyword {name} has an out-of-range value")
            }
            FitsError::IntegerOutOfRange { value, target } => {
                write!(f, "FITS integer {value} is outside the {target} range")
            }
            FitsError::UnexpectedEof => write!(f, "unexpected end of stream inside a FITS unit"),
            FitsError::DataUnitOverflow => {
                write!(f, "header-implied data-unit size overflows 64 bits")
            }
            FitsError::DataUnitTooLarge { bytes } => {
                write!(
                    f,
                    "header-implied data-unit size ({bytes} bytes) is too large to allocate"
                )
            }
            FitsError::DataSizeMismatch { expected, got } => {
                write!(
                    f,
                    "decoded data unit has {got} elements, header implies {expected}"
                )
            }
            FitsError::InvalidPqDescriptor { field, value } => {
                write!(f, "invalid P/Q descriptor {field}: {value}")
            }
            FitsError::HduIndexOutOfBounds { index, len } => {
                write!(f, "HDU index {index} out of bounds (file has {len} HDUs)")
            }
            FitsError::HeaderIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "header record index {index} out of bounds (header has {len} records)"
                )
            }
            FitsError::ImageRegionRankMismatch {
                region_rank,
                image_rank,
            } => write!(
                f,
                "image region has {region_rank} axes but the image has {image_rank}"
            ),
            FitsError::ImageRegionOutOfBounds {
                axis,
                start,
                end,
                len,
            } => write!(
                f,
                "image region axis {axis} range {start}..{end} exceeds axis length {len}"
            ),
            FitsError::RowRangeOutOfBounds { start, end, len } => write!(
                f,
                "table row range {start}..{end} exceeds the table's {len} rows"
            ),
            FitsError::TableRowCountMismatch {
                column,
                expected,
                got,
            } => write!(
                f,
                "table column {column:?} has {got} rows but the table requires {expected}"
            ),
            FitsError::TableRowCountUndetermined { column } => write!(
                f,
                "table column {column:?} does not determine a row count; declare the table row count explicitly"
            ),
            FitsError::EmptyVlaNeedsType { column } => write!(
                f,
                "empty VLA column {column:?} needs an explicit heap element type"
            ),
            FitsError::CoordinateCountMismatch { expected, got } => {
                write!(
                    f,
                    "coordinate has {got} {} but the WCS has {expected} axes",
                    if *got == 1 { "value" } else { "values" }
                )
            }
            FitsError::WcsAxisIndexOutOfBounds { axis, len } => {
                write!(
                    f,
                    "1-based WCS axis {axis} out of bounds (WCS has {len} axes)"
                )
            }
            FitsError::OneBasedIndexRequired { kind } => {
                write!(f, "{kind} indices are 1-based and cannot be zero")
            }
            FitsError::NotAnImage => write!(f, "HDU is not an image array"),
            FitsError::ImageHasGroups => {
                write!(
                    f,
                    "image HDU has group structure (PCOUNT ≠ 0 or GCOUNT ≠ 1)"
                )
            }
            FitsError::NotABinTable => write!(f, "HDU is not a binary table"),
            FitsError::NotRandomGroups => write!(f, "HDU is not a random-groups primary"),
            FitsError::NotAnAsciiTable => write!(f, "HDU is not an ASCII table"),
            FitsError::NotCompressedImage => write!(f, "HDU is not a tiled-compressed image"),
            FitsError::NotCompressedTable => write!(f, "HDU is not a tiled-compressed table"),
            FitsError::ConflictingWcsKeywords { detail } => {
                write!(f, "conflicting WCS keywords: {detail}")
            }
            FitsError::UnsupportedWcsTransform { axes } => {
                write!(
                    f,
                    "WCS has unsupported nonlinear transforms on zero-based axes {axes:?}"
                )
            }
            FitsError::WcsProjectionDomain { projection } => {
                write!(
                    f,
                    "coordinate is outside the {projection} projection domain"
                )
            }
            FitsError::WcsCoordinateDomain { axis, algorithm } => {
                write!(
                    f,
                    "coordinate on zero-based axis {axis} is outside the {algorithm} WCS domain"
                )
            }
            FitsError::WcsNoConvergence { algorithm } => {
                write!(f, "{algorithm} WCS iteration did not converge")
            }
            FitsError::UnsupportedCompression { name } => {
                write!(f, "unsupported tiled compression: {name}")
            }
            FitsError::PlioValueOutOfRange { index, value } => write!(
                f,
                "PLIO tile sample {index} has value {value}, outside 0..=16777215"
            ),
            FitsError::InvalidTform { tform } => write!(f, "invalid column format {tform:?}"),
            FitsError::VariableLengthColumn { code } => write!(
                f,
                "column format '{code}' is a variable-length array; use the column reader's vla()"
            ),
            FitsError::NotAVla { code } => {
                write!(f, "column format '{code}' is not a variable-length array")
            }
            FitsError::NotABitColumn { code } => {
                write!(f, "column format '{code}' is not an X bit array")
            }
            FitsError::NotAComplexColumn { code } => {
                write!(f, "column format '{code}' is not a C/M complex column")
            }
            FitsError::NonNumericColumn { code } => {
                write!(f, "column format '{code}' has no numeric physical value")
            }
            FitsError::ColumnIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "column index {index} out of bounds (table has {len} columns)"
                )
            }
            FitsError::GroupIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "group index {index} out of bounds (random-groups array has {len} groups)"
                )
            }
            FitsError::ColumnNotFound { name } => {
                write!(f, "no column named {name:?} in the table")
            }
            FitsError::RowWidthMismatch { computed, declared } => write!(
                f,
                "column widths sum to {computed} bytes but NAXIS1 declares {declared}"
            ),
            FitsError::TableMetadataMismatch { name } => {
                write!(f, "header metadata {name} disagrees with the binary table")
            }
            FitsError::TileShapeRankMismatch {
                tile_rank,
                image_rank,
            } => write!(
                f,
                "tile shape has {tile_rank} axes but the image has {image_rank}"
            ),
        }
    }
}

impl std::error::Error for FitsError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            FitsError::Io(e) => Some(e),
            _ => None,
        }
    }
}

impl From<io::Error> for FitsError {
    fn from(e: io::Error) -> Self {
        FitsError::Io(e)
    }
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
            FitsError::GroupIndexOutOfBounds { index: 3, len: 2 }.to_string(),
            "group index 3 out of bounds (random-groups array has 2 groups)"
        );
        assert_eq!(
            FitsError::CoordinateCountMismatch {
                expected: 2,
                got: 1,
            }
            .to_string(),
            "coordinate has 1 value but the WCS has 2 axes"
        );
        assert_eq!(
            FitsError::WcsAxisIndexOutOfBounds { axis: 3, len: 2 }.to_string(),
            "1-based WCS axis 3 out of bounds (WCS has 2 axes)"
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
    fn io_error_is_preserved_as_source() {
        let io_err = io::Error::new(io::ErrorKind::UnexpectedEof, "boom");
        let err = FitsError::from(io_err);
        assert!(matches!(err, FitsError::Io(_)));
        assert!(Error::source(&err).is_some());
    }
}
