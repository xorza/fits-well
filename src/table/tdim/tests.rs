use crate::error::FitsError;
use crate::table_impl::BinTable;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::internals::table_header;
use bitvec::bitvec;
use bitvec::order::Msb0;

#[test]
fn tdim_accepts_subshapes_and_checks_vla_cells() {
    let mut fixed = table_header(16, 1, &["4J"]);
    fixed.set_internal("TDIM1", "(2)");
    assert!(BinTable::from_data(&fixed, vec![0; 16]).is_ok());
    for invalid in ["(5)", "(2,broken)", "()", "(0)"] {
        fixed.set_internal("TDIM1", invalid);
        assert!(matches!(
            BinTable::from_data(&fixed, vec![0; 16]),
            Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
        ));
    }

    let mut vla = table_header(8, 1, &["1PJ"]);
    vla.set_internal("PCOUNT", 12)
        .set_internal("TDIM1", "(2,2)");
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    for value in [1i32, 2, 3] {
        data.extend_from_slice(&value.to_be_bytes());
    }
    let table = BinTable::from_data(&vla, data).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().vla(),
        Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
    ));

    let mut mixed = table_header(24, 2, &["1PJ", "1QI"]);
    mixed
        .set_internal("PCOUNT", 24)
        .set_internal("TDIM1", "(2,2)")
        .set_internal("TDIM2", "(2,2)");
    let mut data = Vec::new();
    // The out-of-heap offsets prove empty descriptors do not consult the heap.
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&9999i32.to_be_bytes());
    data.extend_from_slice(&0i64.to_be_bytes());
    data.extend_from_slice(&9999i64.to_be_bytes());
    data.extend_from_slice(&4i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&4i64.to_be_bytes());
    data.extend_from_slice(&16i64.to_be_bytes());
    for value in [10i32, 20, 30, 40] {
        data.extend_from_slice(&value.to_be_bytes());
    }
    for value in [50i16, 60, 70, 80] {
        data.extend_from_slice(&value.to_be_bytes());
    }
    let table = BinTable::from_data(&mixed, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().vla().unwrap(),
        vec![
            ColumnData::I32(vec![]),
            ColumnData::I32(vec![10, 20, 30, 40])
        ]
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().vla().unwrap(),
        vec![
            ColumnData::I16(vec![]),
            ColumnData::I16(vec![50, 60, 70, 80])
        ]
    );

    let mut mixed_bits = table_header(24, 2, &["1PX", "1QX"]);
    mixed_bits
        .set_internal("PCOUNT", 4)
        .set_internal("TDIM1", "(3,3)")
        .set_internal("TDIM2", "(3,3)");
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&9999i32.to_be_bytes());
    data.extend_from_slice(&0i64.to_be_bytes());
    data.extend_from_slice(&9999i64.to_be_bytes());
    data.extend_from_slice(&9i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&9i64.to_be_bytes());
    data.extend_from_slice(&2i64.to_be_bytes());
    data.extend_from_slice(&[0xAA, 0x80, 0x55, 0x00]);
    let table = BinTable::from_data(&mixed_bits, data).unwrap();
    let p_bits = table.column_by_idx(0).unwrap().vla_bits().unwrap();
    assert!(p_bits.row(0).is_empty());
    assert_eq!(
        p_bits.row(1),
        bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 0, 1].as_bitslice()
    );
    let q_bits = table.column_by_idx(1).unwrap().vla_bits().unwrap();
    assert!(q_bits.row(0).is_empty());
    assert_eq!(
        q_bits.row(1),
        bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1, 0].as_bitslice()
    );

    let mut malformed_empty = table_header(8, 1, &["1PJ"]);
    malformed_empty.set_internal("TDIM1", "(2,broken)");
    assert!(matches!(
        BinTable::from_data(&malformed_empty, vec![0; 8]),
        Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
    ));
}
