use std::fmt;
use std::io;

pub type Result<T> = std::result::Result<T, FitsError>;

#[derive(Debug)]
pub enum FitsError {
    Io(io::Error),
    /// A keyword name violated the FITS character set or 8-byte length limit.
    InvalidKeyword {
        name: String,
    },
    /// A card's value field could not be parsed as any FITS value type.
    InvalidValue {
        card: String,
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
    /// `GCOUNT < 1`, a negative axis length, or a `THEAP` that precedes the heap).
    KeywordOutOfRange {
        name: &'static str,
    },
    /// The byte stream ended in the middle of a header or data unit.
    UnexpectedEof,
    /// The data-unit size implied by the header overflows a 64-bit byte count
    /// (a malformed or hostile header with absurd `NAXISn`/`PCOUNT`/`GCOUNT`).
    DataUnitOverflow,
    /// A decoded data unit held a different element count than the header's
    /// declared geometry — a corrupt or truncated data unit.
    DataSizeMismatch {
        expected: usize,
        got: usize,
    },
    /// A data-unit read named an HDU index beyond the parsed sequence.
    HduIndexOutOfBounds {
        index: usize,
        len: usize,
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
    /// A tiled-image compression algorithm or variant is not yet supported.
    UnsupportedCompression {
        name: String,
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
    /// No column with the requested `TTYPEn` name exists in the table.
    ColumnNotFound {
        name: String,
    },
    /// The summed column widths disagree with the declared row width (`NAXIS1`).
    RowWidthMismatch {
        computed: usize,
        declared: usize,
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
            FitsError::InvalidKeyword { name } => write!(f, "invalid keyword name {name:?}"),
            FitsError::InvalidValue { card } => {
                write!(f, "cannot parse value field of card {card:?}")
            }
            FitsError::InvalidBitpix { code } => write!(f, "invalid BITPIX value {code}"),
            FitsError::MissingEnd => write!(f, "header unit ended without an END record"),
            FitsError::MissingKeyword { name } => write!(f, "missing mandatory keyword {name}"),
            FitsError::KeywordOutOfRange { name } => {
                write!(f, "keyword {name} has an out-of-range value")
            }
            FitsError::UnexpectedEof => write!(f, "unexpected end of stream inside a FITS unit"),
            FitsError::DataUnitOverflow => {
                write!(f, "header-implied data-unit size overflows 64 bits")
            }
            FitsError::DataSizeMismatch { expected, got } => {
                write!(
                    f,
                    "decoded data unit has {got} elements, header implies {expected}"
                )
            }
            FitsError::HduIndexOutOfBounds { index, len } => {
                write!(f, "HDU index {index} out of bounds (file has {len} HDUs)")
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
            FitsError::UnsupportedCompression { name } => {
                write!(f, "unsupported tiled compression: {name}")
            }
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
            FitsError::ColumnNotFound { name } => {
                write!(f, "no column named {name:?} in the table")
            }
            FitsError::RowWidthMismatch { computed, declared } => write!(
                f,
                "column widths sum to {computed} bytes but NAXIS1 declares {declared}"
            ),
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
    use super::*;

    #[test]
    fn display_messages_are_specific() {
        assert_eq!(
            FitsError::InvalidBitpix { code: 7 }.to_string(),
            "invalid BITPIX value 7"
        );
        assert_eq!(
            FitsError::DataUnitOverflow.to_string(),
            "header-implied data-unit size overflows 64 bits"
        );
        assert_eq!(
            FitsError::MissingKeyword { name: "NAXIS" }.to_string(),
            "missing mandatory keyword NAXIS"
        );
    }

    #[test]
    fn io_error_is_preserved_as_source() {
        let io_err = io::Error::new(io::ErrorKind::UnexpectedEof, "boom");
        let err = FitsError::from(io_err);
        assert!(matches!(err, FitsError::Io(_)));
        assert!(std::error::Error::source(&err).is_some());
    }
}
