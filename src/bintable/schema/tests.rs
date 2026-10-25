use crate::bintable::Table;
use crate::bintable::internals::table_header;
use crate::fits_error::FitsError;
use crate::header_model::keywords;
use std::assert_matches;

#[test]
fn theap_below_the_main_table_is_refused() {
    // §6.6: the heap follows the main table, so THEAP < NAXIS1·NAXIS2 is invalid.
    let mut header = table_header(4, 2, &["1J"]); // main table = 8 bytes
    header
        .set_internal(keywords::PCOUNT, 4)
        .set_internal(keywords::THEAP, 4); // THEAP 4 < 8
    assert_matches!(
        Table::from_data(&header, vec![0u8; 12]),
        Err(FitsError::InvalidKeywordValue {
            keyword: keywords::THEAP,
            ..
        })
    );
}

#[test]
fn row_width_mismatch_is_refused() {
    // Declared NAXIS1 = 99 but the one column is only 4 bytes wide.
    let header = table_header(99, 1, &["1J"]);
    assert_matches!(
        Table::from_data(&header, vec![0u8; 4]),
        Err(FitsError::RowWidthMismatch {
            computed: 4,
            declared: 99
        })
    );
}

#[test]
fn hostile_tform_repeat_saturates_to_a_width_mismatch() {
    // A `TFORMn` repeat near usize::MAX makes `repeat × elem_size` overflow. The
    // saturating `byte_width` clamps to usize::MAX rather than wrapping to a small
    // value that could equal NAXIS1 and then slice out of bounds in `cell()`; the
    // result is a clean row-width mismatch, not a panic. (`…9J` ≈ 1e19 < usize::MAX
    // so it parses, then ×8 saturates.)
    let header = table_header(8, 1, &["9999999999999999999J"]);
    assert_matches!(
        Table::from_data(&header, vec![0u8; 8]),
        Err(FitsError::RowWidthMismatch {
            computed: usize::MAX,
            declared: 8
        })
    );
}
