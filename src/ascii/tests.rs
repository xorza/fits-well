use super::*;
use crate::reader::FitsReader;
use crate::writer::AsciiWriteColumn;
use crate::writer::FitsWriter;
use std::io::Cursor;

#[test]
fn parses_ascii_tform_codes() {
    assert_eq!(parse_ascii_tform("A8").unwrap(), (AsciiKind::Char, 8, 0));
    assert_eq!(
        parse_ascii_tform("I10").unwrap(),
        (AsciiKind::Integer, 10, 0)
    );
    assert_eq!(parse_ascii_tform("F8.2").unwrap(), (AsciiKind::Float, 8, 2));
    assert_eq!(
        parse_ascii_tform("E15.7").unwrap(),
        (AsciiKind::Float, 15, 7)
    );
    assert_eq!(
        parse_ascii_tform("D25.17").unwrap(),
        (AsciiKind::Float, 25, 17)
    );
    assert!(parse_ascii_tform("Z3").is_err());
}

#[test]
fn decodes_hand_built_ascii_rows() {
    // Two columns: name `A4` at col 1, value `I6` at col 5 → row width 10.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 10)
        .set("NAXIS2", 2)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 2)
        .set("TBCOL1", 1)
        .set("TFORM1", "A4")
        .set("TTYPE1", "NAME")
        .set("TBCOL2", 5)
        .set("TFORM2", "I6")
        .set("TTYPE2", "COUNT");
    let data = b"abc    123def    -45".to_vec(); // "abc " + "   123" ; "def " + "   -45"
    let table = AsciiTable::from_data(&header, data).unwrap();
    assert_eq!(table.nrows, 2);
    assert_eq!(table.columns[1].start, 4);
    assert_eq!(
        table.read_column(0).unwrap(),
        ColumnData::Text(vec!["abc".into(), "def".into()])
    );
    assert_eq!(
        table.read_column(1).unwrap(),
        ColumnData::I64(vec![123, -45])
    );
}

#[test]
fn ascii_table_round_trips_through_write_and_read() {
    let columns = vec![
        AsciiWriteColumn {
            name: "NAME".into(),
            unit: None,
            data: ColumnData::Text(vec!["alpha".into(), "beta".into()]),
            width: 6,
            decimals: 0,
        },
        AsciiWriteColumn {
            name: "N".into(),
            unit: Some("count".into()),
            data: ColumnData::I64(vec![7, -3]),
            width: 5,
            decimals: 0,
        },
        AsciiWriteColumn {
            name: "X".into(),
            unit: None,
            data: ColumnData::F64(vec![1.5, -2.25]),
            width: 8,
            decimals: 2,
        },
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_ascii_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus.len(), 2); // auto dataless primary + the TABLE
    assert_eq!(r.hdus[1].kind, crate::HduKind::AsciiTable);
    let t = r.read_ascii_table(1).unwrap();
    assert_eq!(
        t.read_column(0).unwrap(),
        ColumnData::Text(vec!["alpha".into(), "beta".into()])
    );
    assert_eq!(t.read_column(1).unwrap(), ColumnData::I64(vec![7, -3]));
    assert_eq!(t.read_column(2).unwrap(), ColumnData::F64(vec![1.5, -2.25]));
}
