use crate::error::FitsError;
use crate::header::Header;
use crate::table_impl::BinTable;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::internals::table_header;
use bitvec::bitvec;
use bitvec::order::Msb0;

#[test]
fn x_bit_column_unpacks_msb_first() {
    // One `12X` column, 2 bytes/row. 0xAB 0xC0 = 1010_1011 1100_0000; the first
    // 12 bits MSB-first are 1010_1011_1100.
    let header = table_header(2, 1, &["12X"]);
    let table = BinTable::from_data(&header, vec![0xAB, 0xC0]).unwrap();
    let bits = table.column_by_idx(0).unwrap().bits().unwrap();
    assert_eq!(bits.nrows(), 1);
    assert_eq!(
        bits.row(0),
        bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0].as_bitslice()
    );
    assert_eq!(bits.get(0, 0), Some(true)); // first bit (MSB)
    assert_eq!(bits.get(0, 1), Some(false));
    assert_eq!(bits.get(0, 12), None); // past the 12 bits
    // Indexing: `bits[row]` is the row, `bits[row][col]` and `bits[(row, col)]` the bit.
    assert!(bits[0][0]);
    assert!(!bits[0][1]);
    assert!(bits[(0, 0)]);
    assert!(!bits[(0, 1)]);
    // `raw()` still yields the packed bytes.
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::Bytes(vec![0xAB, 0xC0])
    );
}

#[test]
fn read_bit_column_on_a_non_bit_column_errors() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().bits(),
        Err(FitsError::NotABitColumn { code: 'J' })
    ));
}

#[test]
fn vla_bit_column_unpacks_msb_first() {
    // A `1PX` column: row 0 = 12 bits (0xAB 0xC0), row 1 = 4 bits (0xF0), MSB-first.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8) // one P descriptor (2 × i32) per row
        .set_internal("NAXIS2", 2)
        .set_internal("PCOUNT", 3) // heap bytes
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1PX");
    let mut data = Vec::new();
    data.extend_from_slice(&12i32.to_be_bytes()); // row 0: 12 bits …
    data.extend_from_slice(&0i32.to_be_bytes()); //        … at heap offset 0
    data.extend_from_slice(&4i32.to_be_bytes()); // row 1: 4 bits …
    data.extend_from_slice(&2i32.to_be_bytes()); //        … at heap offset 2
    data.extend_from_slice(&[0xAB, 0xC0, 0xF0]); // heap
    let table = BinTable::from_data(&header, data).unwrap();

    let rows = table.column_by_idx(0).unwrap().vla_bits().unwrap();
    assert_eq!(rows.nrows(), 2);
    assert_eq!(
        rows.row(0),
        bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0].as_bitslice()
    );
    // Jagged: row 1 is only 4 bits wide.
    assert_eq!(rows.row(1), bitvec![u8, Msb0; 1, 1, 1, 1].as_bitslice());
    assert_eq!(rows.row(1).len(), 4);
}
