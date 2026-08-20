use crate::error::FitsError;
use crate::table_impl::BinTable;
use crate::table_impl::internals::table_header;
use crate::table_impl::table_schema::TableSchema;

#[test]
fn theap_below_the_main_table_is_rejected() {
    // §6.6: the heap follows the main table, so THEAP < NAXIS1·NAXIS2 is invalid.
    let mut header = table_header(4, 2, &["1J"]); // main table = 8 bytes
    header.set_internal("PCOUNT", 4).set_internal("THEAP", 4); // THEAP 4 < 8
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 12]),
        Err(FitsError::KeywordOutOfRange { name: "THEAP" })
    ));
}

#[test]
fn column_index_is_case_insensitive() {
    // Two columns, only the first named — so the unnamed one exercises the rule that
    // a column with no `TTYPEn` never matches, whatever is asked for.
    let mut header = table_header(8, 1, &["1J", "1J"]);
    header.set_internal("TTYPE1", "Flux");
    let table = BinTable::from_data(&header, vec![0u8; 8]).unwrap();
    assert_eq!(table.column_index("FLUX"), Some(0));
    assert_eq!(table.column_index("flux"), Some(0));
    assert_eq!(table.column_index("missing"), None);
    // An unnamed column is not the empty-named column: `""` matches nothing.
    assert_eq!(table.column_index(""), None);

    // The schema resolves names the same way, without the data unit — it is the one
    // implementation, which `BinTable` and the reader's column selectors both use.
    let schema = TableSchema::parse(&header).unwrap();
    assert_eq!(schema.column_index("FLUX"), Some(0));
    assert_eq!(schema.column_index("missing"), None);
    assert_eq!(schema.column_index(""), None);
}

#[test]
fn row_width_mismatch_is_an_error() {
    // Declared NAXIS1 = 99 but the one column is only 4 bytes wide.
    let header = table_header(99, 1, &["1J"]);
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 4]),
        Err(FitsError::RowWidthMismatch {
            computed: 4,
            declared: 99
        })
    ));
}

#[test]
fn tfields_beyond_999_is_rejected() {
    // §7.3.1 caps TFIELDS at 999; an absurd value must error, not size a huge Vec.
    let mut header = table_header(0, 0, &[]);
    header.set_internal("TFIELDS", 1000);
    assert!(matches!(
        BinTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "TFIELDS" })
    ));

    header.set_internal("TFIELDS", 0).set_internal("NAXIS1", -1);
    assert!(matches!(
        BinTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "NAXIS1" })
    ));
    header.set_internal("NAXIS1", 0).set_internal("PCOUNT", -1);
    assert!(matches!(
        BinTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "PCOUNT" })
    ));
}

#[test]
fn hostile_tform_repeat_saturates_to_a_width_mismatch() {
    // A `TFORMn` repeat near usize::MAX makes `repeat × elem_size` overflow. The
    // saturating `byte_width` clamps to usize::MAX rather than wrapping to a small
    // value that could equal NAXIS1 and then slice out of bounds in `cell()`; the
    // result is a clean row-width mismatch, not a panic. (`…9J` ≈ 1e19 < usize::MAX
    // so it parses, then ×8 saturates.)
    let header = table_header(8, 1, &["9999999999999999999J"]);
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 8]),
        Err(FitsError::RowWidthMismatch { .. })
    ));
}

#[test]
fn row_count_times_width_overflow_is_rejected_not_wrapped() {
    // NAXIS2·NAXIS1 from untrusted axes must not wrap a usize to a small product
    // that passes the length check. One 8-byte row (`1K`) × 3e18 rows = 2.4e19 >
    // usize::MAX, so `from_data` must error rather than truncate.
    let header = table_header(8, 3_000_000_000_000_000_000, &["1K"]);
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 8]),
        Err(FitsError::UnexpectedEof)
    ));
}
