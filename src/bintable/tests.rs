use crate::bintable::column_data::ColumnData;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tform::Storage;
use crate::bintable::tform::Tform;
use crate::internals::fixture::Fixture;
use crate::internals::whole_column::WholeColumn;

#[test]
fn reads_the_real_aips_antenna_table() {
    let mut reader = Fixture::Ddts.stream();
    let table = reader.read_table(1).unwrap();

    assert_eq!(table.schema.row_count(), 28);
    assert_eq!(table.schema.columns.len(), 12);
    // ANNAME = 8A, STABXYZ = 3D, ORBPARM = 0D, NOSTA = 1J ...
    assert_eq!(table.schema.columns[0].name().unwrap(), Some("ANNAME"));
    assert_eq!(
        table.schema.columns[0].tform,
        Tform::new(8, ElementKind::Character, Storage::Fixed)
    );
    assert_eq!(
        table.schema.columns[1].tform,
        Tform::new(3, ElementKind::F64, Storage::Fixed)
    );
    assert_eq!(
        table.schema.columns[2].tform,
        Tform::new(0, ElementKind::F64, Storage::Fixed)
    );
    // The 0D ORBPARM column contributes no width, so NOSTA shares its offset.
    assert_eq!(table.schema.columns[2].byte_offset, 32);
    assert_eq!(table.schema.columns[3].byte_offset, 32);
    assert_eq!(table.schema.columns[1].unit().unwrap(), Some("METERS"));

    // Eight ANNAME bytes and three STABXYZ doubles per row, none for 0D; the first
    // and last rows as the file stores them.
    match table.column(0).unwrap().raw().unwrap() {
        ColumnData::Character(v) => {
            assert_eq!(v.len(), 28 * 8);
            assert_eq!(&v[..8], b"VLA:N28 ");
            assert_eq!(&v[27 * 8..], b"VLA:W20 ");
        }
        other => panic!("ANNAME should be Character, got {other:?}"),
    }
    match table.column(1).unwrap().raw().unwrap() {
        ColumnData::F64(v) => {
            assert_eq!(v.len(), 28 * 3);
            assert_eq!(
                v[..3],
                [
                    -2_091.496_075,
                    -326.602_865_599_561_5,
                    3_089.414_323_996_752_5
                ]
            );
            assert_eq!(
                v[27 * 3..],
                [
                    733.344_811_5,
                    -1_932.975_686_999_037_9,
                    -1_078.110_032_999_888
                ]
            );
        }
        other => panic!("STABXYZ should be F64, got {other:?}"),
    }
    assert_eq!(
        table.column(2).unwrap().raw().unwrap(),
        ColumnData::F64(vec![])
    );
    assert_eq!(table.schema().column_index("NOSTA").unwrap(), Some(3));
}
