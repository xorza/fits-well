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
    /// A keyword was present but held the wrong value type for its role.
    WrongValueType {
        name: &'static str,
    },
    /// The byte stream ended in the middle of a header or data unit.
    UnexpectedEof,
    /// The data-unit size implied by the header overflows a 64-bit byte count
    /// (a malformed or hostile header with absurd `NAXISn`/`PCOUNT`/`GCOUNT`).
    DataUnitOverflow,
    /// A data-unit read named an HDU index beyond the parsed sequence.
    HduIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// `read_image` was called on an HDU that is not an image array (a table,
    /// random-groups, or unmodelled extension).
    NotAnImage,
    /// `read_table` was called on an HDU that is not a binary table.
    NotABinTable,
    /// A `TFORMn` value could not be parsed as a binary-table column format.
    InvalidTform {
        tform: String,
    },
    /// A column format is valid but not yet decodable (the `P`/`Q`
    /// variable-length-array descriptors).
    UnsupportedColumn {
        tform: String,
    },
    /// A column index named a field beyond the table's column list.
    ColumnIndexOutOfBounds {
        index: usize,
        len: usize,
    },
    /// The summed column widths disagree with the declared row width (`NAXIS1`).
    RowWidthMismatch {
        computed: usize,
        declared: usize,
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
            FitsError::WrongValueType { name } => {
                write!(f, "keyword {name} has the wrong value type")
            }
            FitsError::UnexpectedEof => write!(f, "unexpected end of stream inside a FITS unit"),
            FitsError::DataUnitOverflow => {
                write!(f, "header-implied data-unit size overflows 64 bits")
            }
            FitsError::HduIndexOutOfBounds { index, len } => {
                write!(f, "HDU index {index} out of bounds (file has {len} HDUs)")
            }
            FitsError::NotAnImage => write!(f, "HDU is not an image array"),
            FitsError::NotABinTable => write!(f, "HDU is not a binary table"),
            FitsError::InvalidTform { tform } => write!(f, "invalid column format {tform:?}"),
            FitsError::UnsupportedColumn { tform } => {
                write!(
                    f,
                    "column format {tform:?} (variable-length array) is not yet supported"
                )
            }
            FitsError::ColumnIndexOutOfBounds { index, len } => {
                write!(
                    f,
                    "column index {index} out of bounds (table has {len} columns)"
                )
            }
            FitsError::RowWidthMismatch { computed, declared } => write!(
                f,
                "column widths sum to {computed} bytes but NAXIS1 declares {declared}"
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
