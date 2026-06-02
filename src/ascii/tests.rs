use super::*;
use crate::reader::FitsReader;
use crate::writer::AsciiWriteColumn;
use crate::writer::FitsWriter;
use std::io::Cursor;

#[test]
fn parses_ascii_tform_codes() {
    let fmt = |kind, width, decimals| AsciiFormat {
        kind,
        width,
        decimals,
    };
    assert_eq!(parse_ascii_tform("A8").unwrap(), fmt(AsciiKind::Char, 8, 0));
    assert_eq!(
        parse_ascii_tform("I10").unwrap(),
        fmt(AsciiKind::Integer, 10, 0)
    );
    assert_eq!(
        parse_ascii_tform("F8.2").unwrap(),
        fmt(AsciiKind::Float, 8, 2)
    );
    assert_eq!(
        parse_ascii_tform("E15.7").unwrap(),
        fmt(AsciiKind::Float, 15, 7)
    );
    assert_eq!(
        parse_ascii_tform("D25.17").unwrap(),
        fmt(AsciiKind::Float, 25, 17)
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
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::Text(vec!["abc".into(), "def".into()])
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().raw().unwrap(),
        ColumnData::I64(vec![123, -45])
    );
    // By-name access (case-insensitive) mirrors the by-index reads.
    assert_eq!(
        table.column_by_name("count").unwrap().raw().unwrap(),
        ColumnData::I64(vec![123, -45])
    );
    assert_eq!(
        table.column_by_name("COUNT").unwrap().physical().unwrap(),
        vec![123.0, -45.0]
    );
    assert!(matches!(
        table.column_by_name("missing"),
        Err(FitsError::ColumnNotFound { .. })
    ));
}

#[test]
fn applies_tscal_tzero_and_maps_tnull_to_nan() {
    // One `I6` column, TSCAL=2, TZERO=10, TNULL='***'. Row 0 = 123, row 1 = null.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 6)
        .set("NAXIS2", 2)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TBCOL1", 1)
        .set("TFORM1", "I6")
        .set("TSCAL1", 2.0)
        .set("TZERO1", 10.0)
        .set("TNULL1", "***");
    let data = b"   123   ***".to_vec();
    let table = AsciiTable::from_data(&header, data).unwrap();
    // Raw: the null field is a 0 placeholder; physical: TZERO + TSCAL·field, null → NaN.
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I64(vec![123, 0])
    );
    let phys = table.column_by_idx(0).unwrap().physical().unwrap();
    assert_eq!(phys[0], 256.0); // 10 + 2·123
    assert!(phys[1].is_nan());
}

#[test]
fn implicit_decimal_point_scales_by_ten_to_the_d() {
    // `F8.3`: a field with no explicit point has the point implied 3 from the right.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 8)
        .set("NAXIS2", 2)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TBCOL1", 1)
        .set("TFORM1", "F8.3");
    let data = b"   12345  12.345".to_vec(); // implicit "12345" → 12.345 ; explicit 12.345
    let table = AsciiTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::F64(vec![12.345, 12.345])
    );
}

#[test]
fn ascii_column_index_is_case_insensitive() {
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 4)
        .set("NAXIS2", 1)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TBCOL1", 1)
        .set("TFORM1", "I4")
        .set("TTYPE1", "Count");
    let table = AsciiTable::from_data(&header, b"   7".to_vec()).unwrap();
    assert_eq!(table.column_index("COUNT"), Some(0));
    assert_eq!(table.column_index("count"), Some(0));
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
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "N".into(),
            unit: Some("count".into()),
            data: ColumnData::I64(vec![7, -3]),
            width: 5,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "X".into(),
            unit: None,
            data: ColumnData::F64(vec![1.5, -2.25]),
            width: 8,
            decimals: 2,
            tscale: None,
            tzero: None,
            tnull: None,
        },
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_ascii_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus.len(), 2); // auto dataless primary + the TABLE
    assert_eq!(r.hdus[1].kind, crate::HduKind::AsciiTable);
    let t = r.read_ascii_table(1).unwrap();
    assert_eq!(
        t.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::Text(vec!["alpha".into(), "beta".into()])
    );
    assert_eq!(
        t.column_by_idx(1).unwrap().raw().unwrap(),
        ColumnData::I64(vec![7, -3])
    );
    assert_eq!(
        t.column_by_idx(2).unwrap().raw().unwrap(),
        ColumnData::F64(vec![1.5, -2.25])
    );
}

#[test]
fn signed_exponent_without_letter_parses_as_fortran_real() {
    // §7.2.5 rule 3(a): a numeric field may be terminated by a bare '+'/'-' that
    // introduces the exponent (no E/D letter), e.g. 3.14159-2 = 3.14159 × 10⁻².
    let approx = |got: Option<f64>, want: f64| {
        let g = got.expect("should parse");
        assert!((g - want).abs() < 1e-12, "got {g}, want {want}");
    };
    approx(parse_ascii_float("3.14159-2", 5), 0.0314159);
    approx(parse_ascii_float("2.5+3", 1), 2500.0);
    approx(parse_ascii_float("-3.0-1", 1), -0.3);
    // The leading mantissa sign is NOT an exponent; implicit decimal still applies.
    approx(parse_ascii_float("-12", 3), -0.012);
    // Explicit E/D forms keep working.
    approx(parse_ascii_float("1.5E2", 1), 150.0);
    approx(parse_ascii_float("1.5D-2", 1), 0.015);

    assert_eq!(
        split_mantissa_exponent("3.14159-2"),
        Some(("3.14159", "-2"))
    );
    assert_eq!(split_mantissa_exponent("-3.0-1"), Some(("-3.0", "-1")));
    assert_eq!(split_mantissa_exponent("1.5E2"), Some(("1.5", "2")));
    assert_eq!(split_mantissa_exponent("123"), None);
}

#[test]
fn reads_a_column_with_a_bare_sign_exponent_field() {
    // The letter-less exponent form (CFITSIO emits it) must read, not error.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 12)
        .set("NAXIS2", 1)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TBCOL1", 1)
        .set("TFORM1", "E12.5");
    let data = b"   3.14159-2".to_vec(); // 12 chars; 3.14159-2 = 0.0314159
    let table = AsciiTable::from_data(&header, data).unwrap();
    match table.column_by_idx(0).unwrap().raw().unwrap() {
        ColumnData::F64(v) => assert!((v[0] - 0.0314159).abs() < 1e-12, "{}", v[0]),
        other => panic!("expected F64, got {other:?}"),
    }
}

#[test]
fn ascii_write_emits_tscal_tzero_tnull_and_round_trips() {
    // A scaled integer column (raw values + TSCAL/TZERO) and a float column whose
    // undefined cell is written via TNULL and reads back as NaN (§7.2.2/§7.2.4).
    let columns = vec![
        AsciiWriteColumn {
            name: "RAW".into(),
            unit: None,
            data: ColumnData::I64(vec![5, 10]),
            width: 6,
            decimals: 0,
            tscale: Some(2.0),
            tzero: Some(100.0),
            tnull: None,
        },
        AsciiWriteColumn {
            name: "FLUX".into(),
            unit: None,
            data: ColumnData::F64(vec![1.5, f64::NAN]),
            width: 10,
            decimals: 3,
            tscale: None,
            tzero: None,
            tnull: Some("NULL".into()),
        },
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_ascii_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus[1].header.get_real("TSCAL1"), Some(2.0));
    assert_eq!(r.hdus[1].header.get_real("TZERO1"), Some(100.0));
    assert_eq!(r.hdus[1].header.get_text("TNULL2"), Some("NULL"));

    let t = r.read_ascii_table(1).unwrap();
    // Raw stored integers, then the scaled physical plane TZERO + TSCAL·field.
    assert_eq!(
        t.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I64(vec![5, 10])
    );
    assert_eq!(
        t.column_by_idx(0).unwrap().physical().unwrap(),
        vec![110.0, 120.0]
    );
    // The TNULL-marked float cell reads back as NaN.
    let flux = t.column_by_idx(1).unwrap().physical().unwrap();
    assert_eq!(flux[0], 1.5);
    assert!(flux[1].is_nan());
}

#[test]
fn ascii_tfields_beyond_999_is_rejected() {
    // §7.2.1 caps TFIELDS at 999; an absurd value must error, not size a huge Vec.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 0)
        .set("NAXIS2", 0)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1000);
    assert!(matches!(
        AsciiTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "TFIELDS" })
    ));
}

#[test]
fn ascii_row_count_times_width_overflow_is_rejected() {
    // NAXIS2·NAXIS1 from untrusted axes must not wrap a usize to a small product.
    // 3e18 rows × 8 chars = 2.4e19 > usize::MAX, so `from_data` must error.
    let mut header = Header::new();
    header
        .set("XTENSION", "TABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 8)
        .set("NAXIS2", 3_000_000_000_000_000_000i64)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TBCOL1", 1)
        .set("TFORM1", "I8");
    assert!(matches!(
        AsciiTable::from_data(&header, vec![0u8; 8]),
        Err(FitsError::UnexpectedEof)
    ));
}
