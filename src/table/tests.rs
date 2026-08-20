use crate::error::FitsError;
use crate::reader::internals::open_fixture;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::tform::internals::tform;
use crate::table_impl::tform_kind::TformKind;

#[test]
fn reads_the_real_aips_antenna_table() {
    let mut reader = open_fixture("DDTSUVDATA.fits");
    let table = reader.read_table(1).unwrap();

    assert_eq!(table.schema.nrows, 28);
    assert_eq!(table.schema.columns.len(), 12);
    // ANNAME = 8A, STABXYZ = 3D, ORBPARM = 0D, NOSTA = 1J ...
    assert_eq!(table.schema.columns[0].name.as_deref(), Some("ANNAME"));
    assert_eq!(
        table.schema.columns[0].tform,
        tform(8, TformKind::Char, None)
    );
    assert_eq!(
        table.schema.columns[1].tform,
        tform(3, TformKind::F64, None)
    );
    assert_eq!(
        table.schema.columns[2].tform,
        tform(0, TformKind::F64, None)
    );
    // The 0D ORBPARM column contributes no width, so NOSTA shares its offset.
    assert_eq!(table.schema.columns[2].byte_offset, 32);
    assert_eq!(table.schema.columns[3].byte_offset, 32);
    assert_eq!(table.schema.columns[1].unit.as_deref(), Some("METERS"));

    // Decoded element counts: one ANNAME string per row, 3 doubles per row, none for 0D.
    match table.column_by_idx(0).unwrap().raw().unwrap() {
        ColumnData::Character(v) => assert_eq!(v.len(), 28),
        other => panic!("ANNAME should be Character, got {other:?}"),
    }
    match table.column_by_idx(1).unwrap().raw().unwrap() {
        ColumnData::F64(v) => assert_eq!(v.len(), 28 * 3),
        other => panic!("STABXYZ should be F64, got {other:?}"),
    }
    assert_eq!(
        table.column_by_idx(2).unwrap().raw().unwrap(),
        ColumnData::F64(vec![])
    );
    assert_eq!(table.column_index("NOSTA"), Some(3));
}

#[test]
fn read_table_rejects_non_bintable_hdus() {
    let mut reader = open_fixture("DDTSUVDATA.fits");
    // HDU 0 is a random-groups primary, not a binary table.
    assert!(matches!(reader.read_table(0), Err(FitsError::NotABinTable)));
}
