//! The rules both table kinds share, each checked on a binary and an ASCII table.

use crate::ascii::internals::ascii_table_header;
use crate::ascii::{Ascii, AsciiSchema};
use crate::bintable::Table;
use crate::bintable::internals::table_header;
use crate::bintable::schema::Schema;
use crate::fits_error::FitsError;
use crate::header_model::Header;
use crate::internals::whole_column::{WholeAsciiColumn, WholeColumn};
use std::assert_matches;

/// A binary table of two `1J` columns and an ASCII table of two `I4` columns, the first
/// of each named `Flux`.
fn named_first_of_two() -> [Header; 2] {
    let mut binary = table_header(8, 1, &["1J", "1J"]);
    binary.set_internal("TTYPE1", "Flux");
    let mut ascii = ascii_table_header(8, 1, &[(1, "I4"), (5, "I4")]);
    ascii.set_internal("TTYPE1", "Flux");
    [binary, ascii]
}

/// §6.7 and §7.2.2 give both kinds one rule: a name matches without regard to case, and
/// an unnamed column matches nothing, `""` included.
#[test]
fn column_name_matches_without_regard_to_case() {
    let [binary, ascii] = named_first_of_two();
    let binary = Schema::parse(&binary).unwrap();
    let ascii = AsciiSchema::parse(&ascii).unwrap();
    for (name, expected) in [
        ("FLUX", Some(0)),
        ("flux", Some(0)),
        ("missing", None),
        ("", None),
    ] {
        assert_eq!(
            binary.column_index(name).unwrap(),
            expected,
            "binary {name:?}"
        );
        assert_eq!(
            ascii.column_index(name).unwrap(),
            expected,
            "ASCII {name:?}"
        );
    }
}

/// `TDISPn` is kept as its text, a form the reader does not know included, and does not
/// change what the data reads as.
#[test]
fn tdisp_is_kept_as_text_apart_from_the_data() {
    let [mut binary, mut ascii] = named_first_of_two();
    for display in ["I5", "Q5", "E12.5E3junk"] {
        binary.set_internal("TDISP1", display);
        ascii.set_internal("TDISP1", display);
        let binary = Table::from_data(&binary, vec![0, 0, 0, 7, 0, 0, 0, 0]).unwrap();
        let ascii = Ascii::from_data(&ascii, b"   7   0".to_vec()).unwrap();
        assert_eq!(binary.schema().columns[0].tdisp().unwrap(), Some(display));
        assert_eq!(ascii.schema().columns[0].tdisp().unwrap(), Some(display));
        assert_eq!(binary.column(0).unwrap().physical().unwrap(), [7.0]);
        assert_eq!(ascii.column(0).unwrap().physical().unwrap(), [7.0]);
    }
}

/// A parse of a header with one keyword set.
type Parse = fn(&str, i64) -> Result<(), FitsError>;

/// §7.3.1 and §7.2.1 fix `BITPIX = 8`, `NAXIS = 2` and `GCOUNT = 1`, which size the data
/// unit, and cap `TFIELDS` at 999. A binary table takes no negative `NAXIS1` or `PCOUNT`,
/// and an ASCII table no `PCOUNT` at all, nor a `TBCOLn` before its row.
#[test]
fn table_structure_out_of_range_is_refused() {
    let binary = |keyword: &str, value: i64| {
        let mut header = table_header(0, 0, &[]);
        header.set_internal(keyword, value);
        Schema::parse(&header).map(drop)
    };
    let ascii = |keyword: &str, value: i64| {
        let mut header = ascii_table_header(1, 0, &[(1, "A1")]);
        header.set_internal(keyword, value);
        AsciiSchema::parse(&header).map(drop)
    };
    let cases: [(&str, Parse, &str, i64); 13] = [
        ("binary", binary, "BITPIX", 16),
        ("binary", binary, "BITPIX", -32),
        ("binary", binary, "NAXIS", 1),
        ("binary", binary, "GCOUNT", 3),
        ("binary", binary, "TFIELDS", 1000),
        ("binary", binary, "NAXIS1", -1),
        ("binary", binary, "PCOUNT", -1),
        ("ASCII", ascii, "BITPIX", 16),
        ("ASCII", ascii, "NAXIS", 1),
        ("ASCII", ascii, "GCOUNT", 3),
        ("ASCII", ascii, "TFIELDS", 1000),
        ("ASCII", ascii, "PCOUNT", 4),
        ("ASCII", ascii, "TBCOL1", 0),
    ];
    for (kind, parse, keyword, value) in cases {
        assert_matches!(
            parse(keyword, value),
            Err(FitsError::InvalidKeywordValue { keyword: got, .. }) if got == keyword,
            "{kind} {keyword} = {value}"
        );
    }
}

/// 3·10¹⁸ rows of 8 bytes pass `usize`: an error, never a small wrapped length that the
/// data unit could meet.
#[test]
fn row_count_times_width_past_usize_is_refused() {
    let binary = table_header(8, 3_000_000_000_000_000_000, &["1K"]);
    assert_matches!(
        Table::from_data(&binary, vec![0; 8]),
        Err(FitsError::DataUnitOverflow)
    );
    let ascii = ascii_table_header(8, 3_000_000_000_000_000_000, &[(1, "I8")]);
    assert_matches!(
        Ascii::from_data(&ascii, vec![0; 8]),
        Err(FitsError::DataUnitOverflow)
    );
}
