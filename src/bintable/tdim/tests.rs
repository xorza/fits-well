use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
use crate::bintable::internals::table_header;
use crate::fits_error::FitsError;
use crate::fits_error::KeywordDefect;
use crate::header_model::keywords;
use crate::internals::whole_column::WholeColumn;
use std::assert_matches;

use crate::ragged::Ragged;
use bitvec::bitvec;
use bitvec::order::Msb0;

/// A shape the cell cannot hold is a defect of `TDIMn` alone: the stored values still
/// read. Text that is no shape, an empty one among them, is a syntax defect, a shape past
/// the cell's four elements a structure one, and a zero axis a range one.
#[test]
fn fixed_cell_takes_a_subshape_and_reports_each_defect() {
    let mut fixed = table_header(16, 1, &["4J"]);
    fixed.set_internal("TDIM1", "(2)");
    assert_eq!(
        Table::from_data(&fixed, vec![0; 16])
            .unwrap()
            .schema()
            .columns[0]
            .tdim()
            .unwrap(),
        Some(&[2][..])
    );
    for (invalid, expected) in [
        ("(5)", KeywordDefect::Structure),
        ("(2,broken)", KeywordDefect::Syntax),
        ("()", KeywordDefect::Syntax),
        ("(0)", KeywordDefect::Range),
    ] {
        fixed.set_internal("TDIM1", invalid);
        let table = Table::from_data(&fixed, vec![0; 16]).unwrap();
        assert_matches!(
            table.schema.columns[0].tdim(),
            Err(FitsError::InvalidColumnKeyword { keyword, source })
                if keyword == "TDIM1"
                    && matches!(*source, FitsError::InvalidKeywordValue { keyword, defect } if keyword == "TDIM1" && defect == expected),
            "{invalid}"
        );
        assert_eq!(
            table.column(0).unwrap().raw().unwrap(),
            ColumnData::I32(vec![0; 4])
        );
    }
}

/// A row of 3 values does not fill a `(2,2)` shape.
#[test]
fn vla_row_must_fill_its_shape() {
    let mut vla = table_header(8, 1, &["1PJ"]);
    vla.set_internal(keywords::PCOUNT, 12)
        .set_internal("TDIM1", "(2,2)");
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    for value in [1i32, 2, 3] {
        data.extend_from_slice(&value.to_be_bytes());
    }
    let table = Table::from_data(&vla, data).unwrap();
    assert_matches!(
        table.column(0).unwrap().vla(),
        Err(FitsError::InvalidKeywordValue { keyword, .. }) if keyword == "TDIM1"
    );
}

/// An empty row of a shaped array column reads empty from its descriptor alone: the
/// out-of-heap offsets prove it does not consult the heap.
#[test]
fn empty_shaped_row_does_not_read_the_heap() {
    let mut mixed = table_header(24, 2, &["1PJ", "1QI"]);
    mixed
        .set_internal(keywords::PCOUNT, 24)
        .set_internal("TDIM1", "(2,2)")
        .set_internal("TDIM2", "(2,2)");
    let mut data = Vec::new();
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
    let table = Table::from_data(&mixed, data).unwrap();
    assert_eq!(
        table.column(0).unwrap().vla().unwrap(),
        Ragged::from_rows([
            ColumnData::I32(vec![]),
            ColumnData::I32(vec![10, 20, 30, 40])
        ])
        .unwrap()
    );
    assert_eq!(
        table.column(1).unwrap().vla().unwrap(),
        Ragged::from_rows([
            ColumnData::I16(vec![]),
            ColumnData::I16(vec![50, 60, 70, 80])
        ])
        .unwrap()
    );
}

/// The bit columns read as the others do: an empty row from its descriptor alone.
#[test]
fn empty_shaped_bit_row_does_not_read_the_heap() {
    let mut mixed_bits = table_header(24, 2, &["1PX", "1QX"]);
    mixed_bits
        .set_internal(keywords::PCOUNT, 4)
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
    let table = Table::from_data(&mixed_bits, data).unwrap();
    let p_bits = table.column(0).unwrap().vla_bits().unwrap();
    assert!(p_bits.row(0).is_empty());
    assert_eq!(
        p_bits.row(1),
        bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 0, 1].as_bitslice()
    );
    let q_bits = table.column(1).unwrap().vla_bits().unwrap();
    assert!(q_bits.row(0).is_empty());
    assert_eq!(
        q_bits.row(1),
        bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1, 0].as_bitslice()
    );
}

/// An unreadable shape fails the array read, an empty row included.
#[test]
fn unreadable_shape_fails_an_empty_row() {
    let mut malformed_empty = table_header(8, 1, &["1PJ"]);
    malformed_empty.set_internal("TDIM1", "(2,broken)");
    let table = Table::from_data(&malformed_empty, vec![0; 8]).unwrap();
    assert_matches!(
        table.column(0).unwrap().vla(),
        Err(FitsError::InvalidColumnKeyword { keyword, .. }) if keyword == "TDIM1"
    );
}
