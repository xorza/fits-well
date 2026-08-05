use crate::ascii::*;
use crate::reader::FitsReader;
use crate::writer::FitsWriter;
use crate::writer::ascii::{AsciiTableBuilder, AsciiWriteColumn};
use std::io::Cursor;

fn write_table(nrows: usize, columns: &[AsciiWriteColumn]) -> AsciiTableBuilder {
    AsciiTableBuilder {
        nrows: Some(nrows),
        columns: columns.to_vec(),
    }
}

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
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 10)
        .set_internal("NAXIS2", 2)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 2)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "A4")
        .set_internal("TTYPE1", "NAME")
        .set_internal("TBCOL2", 5)
        .set_internal("TFORM2", "I6")
        .set_internal("TTYPE2", "COUNT");
    let data = b"  AB   123def    -45".to_vec(); // "  AB" + "   123" ; "def " + "   -45"
    let table = AsciiTable::from_data(&header, data).unwrap();
    let mut metadata = table.metadata();
    assert_eq!(metadata.nrows, 2);
    assert_eq!(metadata.columns[1].start, 4);
    metadata.nrows = usize::MAX;
    metadata.columns = &[];
    assert_eq!(metadata.nrows, usize::MAX);
    assert!(metadata.columns.is_empty());
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text(vec![Some("  AB".into()), Some("def ".into())])
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(-45)])
    );
    // By-name access (case-insensitive) mirrors the by-index reads.
    assert_eq!(
        table.column_by_name("count").unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(-45)])
    );
    assert_eq!(
        table.column_by_name("COUNT").unwrap().physical().unwrap(),
        vec![123.0, -45.0]
    );
    assert!(matches!(
        table.column_by_name("missing"),
        Err(FitsError::ColumnNotFound { .. })
    ));

    // §7.2: the data unit is ASCII text, so a non-ASCII byte is rejected when the
    // table is parsed rather than deferred until whichever column happens to hold it
    // is read — here the corrupt byte sits in NAME while COUNT is well formed.
    let mut corrupt = b"  AB   123def    -45".to_vec();
    corrupt[2] = 0xFF;
    assert!(matches!(
        AsciiTable::from_data(&header, corrupt),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn applies_tscal_tzero_and_maps_tnull_to_nan() {
    // One `I6` column, TSCAL=2, TZERO=10, TNULL='***': 123, blank zero, then null.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 6)
        .set_internal("NAXIS2", 3)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "I6")
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", "***");
    let data = b"   123         ***".to_vec();
    let table = AsciiTable::from_data(&header, data).unwrap();
    // Raw preserves nullness; physical applies TZERO + TSCAL·field and maps null to NaN.
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(0), None])
    );
    let phys = table.column_by_idx(0).unwrap().physical().unwrap();
    assert_eq!(phys[0], 256.0); // 10 + 2·123
    assert_eq!(phys[1], 10.0); // blank field = stored zero
    assert!(phys[2].is_nan());

    for keyword in ["TSCAL1", "TZERO1"] {
        let mut malformed = header.clone();
        malformed.set_internal(keyword, "not numeric");
        assert!(matches!(
            AsciiTable::from_data(&malformed, b"   123         ***".to_vec()),
            Err(FitsError::TypeMismatch { name, .. }) if name == keyword
        ));
    }
}

#[test]
fn implicit_decimal_point_scales_by_ten_to_the_d() {
    // `F8.3`: a field with no explicit point has the point implied 3 from the right.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8)
        .set_internal("NAXIS2", 2)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "F8.3");
    let data = b"   12345  12.345".to_vec(); // implicit "12345" → 12.345 ; explicit 12.345
    let table = AsciiTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(12.345), Some(12.345)])
    );
}

#[test]
fn ascii_column_index_is_case_insensitive() {
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 4)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "I4")
        .set_internal("TTYPE1", "Count");
    let table = AsciiTable::from_data(&header, b"   7".to_vec()).unwrap();
    assert_eq!(table.column_index("COUNT"), Some(0));
    assert_eq!(table.column_index("count"), Some(0));
    // §7.2.2 and §6.7 are the same rule, and both table forms now resolve names
    // through one implementation — so the ASCII form rejects the same non-matches
    // the binary one does.
    assert_eq!(table.column_index("missing"), None);
    assert_eq!(table.column_index(""), None);
}

#[test]
fn ascii_table_round_trips_through_write_and_read() {
    let mut columns = vec![
        AsciiWriteColumn {
            name: "NAME".into(),
            unit: None,
            data: AsciiColumnData::Text(vec![Some("  AB".into()), Some("beta".into())]),
            width: 6,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "N".into(),
            unit: Some("count".into()),
            data: AsciiColumnData::Integer(vec![Some(7), Some(-3)]),
            width: 5,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "X".into(),
            unit: None,
            data: AsciiColumnData::Float(vec![Some(1.5), Some(-2.25)]),
            width: 8,
            decimals: 2,
            tscale: None,
            tzero: None,
            tnull: None,
        },
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_ascii_table(&write_table(2, &columns)).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus.len(), 2); // auto dataless primary + the TABLE
    assert_eq!(r.hdus[1].kind, crate::io::HduKind::AsciiTable);
    assert_eq!(&r.read_data_raw(1).unwrap().data()[..6], b"  AB  ");
    let t = r.read_ascii_table(1).unwrap();
    assert_eq!(
        t.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text(vec![Some("  AB  ".into()), Some("beta  ".into())])
    );
    assert_eq!(
        t.column_by_idx(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(7), Some(-3)])
    );
    assert_eq!(
        t.column_by_idx(2).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(1.5), Some(-2.25)])
    );

    columns[0].data = AsciiColumnData::Text(vec![Some("café".into()), Some("beta".into())]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(&write_table(2, &columns)),
        Err(FitsError::InvalidAscii {
            context: "ASCII text cell"
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
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
    // A point-less mantissa with an *explicit* exponent is read literally (strtod),
    // NOT implied-decimal-scaled: `1E5` is 100000, `15E2` is 1500 — matching
    // cfitsio/astropy. The implied decimal applies only to point-less `Fw.d` fields.
    approx(parse_ascii_float("1E5", 3), 100_000.0);
    approx(parse_ascii_float("15E2", 3), 1500.0);
    approx(parse_ascii_float("2E-3", 4), 0.002);

    assert_eq!(
        split_mantissa_exponent("3.14159-2"),
        Some(("3.14159", "-2"))
    );
    assert_eq!(split_mantissa_exponent("-3.0-1"), Some(("-3.0", "-1")));
    assert_eq!(split_mantissa_exponent("1.5E2"), Some(("1.5", "2")));
    // The Fortran double `D`/`d` exponent splits in place (no String normalization).
    assert_eq!(split_mantissa_exponent("1.5D-2"), Some(("1.5", "-2")));
    assert_eq!(split_mantissa_exponent("123"), None);
}

#[test]
fn reads_a_column_with_a_bare_sign_exponent_field() {
    // The letter-less exponent form (CFITSIO emits it) must read, not error.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 12)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "E12.5");
    let data = b"   3.14159-2".to_vec(); // 12 chars; 3.14159-2 = 0.0314159
    let table = AsciiTable::from_data(&header, data).unwrap();
    match table.column_by_idx(0).unwrap().raw().unwrap() {
        AsciiColumnData::Float(values) => {
            let value = values[0].unwrap();
            assert!((value - 0.0314159).abs() < 1e-12, "{value}");
        }
        other => panic!("expected Float, got {other:?}"),
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
            data: AsciiColumnData::Integer(vec![Some(5), Some(10)]),
            width: 6,
            decimals: 0,
            tscale: Some(2.0),
            tzero: Some(100.0),
            tnull: None,
        },
        AsciiWriteColumn {
            name: "FLUX".into(),
            unit: None,
            data: AsciiColumnData::Float(vec![Some(1.5), None]),
            width: 10,
            decimals: 3,
            tscale: None,
            tzero: None,
            tnull: Some("NULL".into()),
        },
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_ascii_table(&write_table(2, &columns)).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus[1].header.get_real("TSCAL1").unwrap(), Some(2.0));
    assert_eq!(r.hdus[1].header.get_real("TZERO1").unwrap(), Some(100.0));
    assert_eq!(r.hdus[1].header.get_text("TNULL2").unwrap(), Some("NULL"));

    let t = r.read_ascii_table(1).unwrap();
    // Raw stored integers, then the scaled physical plane TZERO + TSCAL·field.
    assert_eq!(
        t.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(5), Some(10)])
    );
    assert_eq!(
        t.column_by_idx(0).unwrap().physical().unwrap(),
        vec![110.0, 120.0]
    );
    // The TNULL-marked float cell reads back as NaN.
    let flux = t.column_by_idx(1).unwrap().physical().unwrap();
    assert_eq!(flux[0], 1.5);
    assert!(flux[1].is_nan());
    assert_eq!(
        t.column_by_idx(1).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(1.5), None])
    );

    for marker in [None, Some(""), Some("TOO-LONG")] {
        let invalid = [AsciiWriteColumn {
            name: "BAD".into(),
            unit: None,
            data: AsciiColumnData::Float(vec![None]),
            width: 4,
            decimals: 1,
            tscale: None,
            tzero: None,
            tnull: marker.map(str::to_string),
        }];
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_ascii_table(&write_table(1, &invalid)),
            Err(FitsError::KeywordOutOfRange { name: "TNULLn" })
        ));
        assert!(writer.into_inner().into_inner().is_empty());
    }

    let collision = [AsciiWriteColumn {
        name: "BAD".into(),
        unit: None,
        data: AsciiColumnData::Integer(vec![Some(0)]),
        width: 4,
        decimals: 0,
        tscale: None,
        tzero: None,
        tnull: Some("0".into()),
    }];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(&write_table(1, &collision)),
        Err(FitsError::InvalidValue { card }) if card == "ASCII value equals its TNULLn marker"
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let nonfinite = [AsciiWriteColumn {
        name: "BAD".into(),
        unit: None,
        data: AsciiColumnData::Float(vec![Some(f64::INFINITY)]),
        width: 8,
        decimals: 1,
        tscale: None,
        tzero: None,
        tnull: Some("NULL".into()),
    }];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(&write_table(1, &nonfinite)),
        Err(FitsError::InvalidValue { card })
            if card == "ASCII float cells must be finite; use None for null"
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

#[test]
fn ascii_writer_accepts_exact_width_values() {
    let columns = [
        AsciiWriteColumn {
            name: "TEXT".into(),
            unit: None,
            data: AsciiColumnData::Text(vec![Some("abc".into())]),
            width: 3,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "INT".into(),
            unit: None,
            data: AsciiColumnData::Integer(vec![Some(-12)]),
            width: 3,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "FLOAT".into(),
            unit: None,
            data: AsciiColumnData::Float(vec![Some(1.25)]),
            width: 4,
            decimals: 2,
            tscale: None,
            tzero: None,
            tnull: None,
        },
        AsciiWriteColumn {
            name: "NULL".into(),
            unit: None,
            data: AsciiColumnData::Float(vec![None]),
            width: 4,
            decimals: 1,
            tscale: None,
            tzero: None,
            tnull: Some("NULL".into()),
        },
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_ascii_table(&write_table(1, &columns)).unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        &reader.read_data_raw(1).unwrap().data()[..14],
        b"abc-121.25NULL"
    );
}

#[derive(Debug)]
struct AsciiOverflowCase {
    column: AsciiWriteColumn,
    nrows: usize,
    row: usize,
    minimum_width: usize,
}

#[test]
fn ascii_writer_rejects_one_byte_overflow_before_output() {
    let cases = [
        AsciiOverflowCase {
            column: AsciiWriteColumn {
                name: "TEXT".into(),
                unit: None,
                data: AsciiColumnData::Text(vec![Some("ok".into()), Some("abcd".into())]),
                width: 3,
                decimals: 0,
                tscale: None,
                tzero: None,
                tnull: None,
            },
            nrows: 2,
            row: 1,
            minimum_width: 4,
        },
        AsciiOverflowCase {
            column: AsciiWriteColumn {
                name: "INT".into(),
                unit: None,
                data: AsciiColumnData::Integer(vec![Some(-123)]),
                width: 3,
                decimals: 0,
                tscale: None,
                tzero: None,
                tnull: None,
            },
            nrows: 1,
            row: 0,
            minimum_width: 4,
        },
        AsciiOverflowCase {
            column: AsciiWriteColumn {
                name: "FLOAT".into(),
                unit: None,
                data: AsciiColumnData::Float(vec![Some(-1.25)]),
                width: 4,
                decimals: 2,
                tscale: None,
                tzero: None,
                tnull: None,
            },
            nrows: 1,
            row: 0,
            minimum_width: 5,
        },
        AsciiOverflowCase {
            column: AsciiWriteColumn {
                name: "PRECISION".into(),
                unit: None,
                data: AsciiColumnData::Float(vec![Some(1.0)]),
                width: 1,
                decimals: usize::MAX,
                tscale: None,
                tzero: None,
                tnull: None,
            },
            nrows: 1,
            row: 0,
            minimum_width: usize::MAX,
        },
    ];
    for case in cases {
        let column_name = case.column.name.clone();
        let width = case.column.width;
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_ascii_table(&write_table(case.nrows, &[case.column])),
            Err(FitsError::AsciiFieldTooWide {
                column,
                row,
                width: actual_width,
                minimum_width,
            }) if column == column_name
                && row == case.row
                && actual_width == width
                && minimum_width == case.minimum_width
        ));
        assert!(writer.into_inner().into_inner().is_empty());
    }

    let columns = [AsciiWriteColumn {
        name: "NULL".into(),
        unit: None,
        data: AsciiColumnData::Float(vec![None]),
        width: 3,
        decimals: 1,
        tscale: None,
        tzero: None,
        tnull: Some("NULL".into()),
    }];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(&write_table(1, &columns)),
        Err(FitsError::KeywordOutOfRange { name: "TNULLn" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

#[derive(Debug)]
struct InvalidAsciiScale {
    data: AsciiColumnData,
    tscale: Option<f64>,
    tzero: Option<f64>,
    keyword: &'static str,
}

#[test]
fn ascii_scaling_metadata_is_validated_by_stored_type_before_output() {
    let cases = [
        InvalidAsciiScale {
            data: AsciiColumnData::Text(vec![Some("A".into())]),
            tscale: Some(2.0),
            tzero: None,
            keyword: "TSCALn",
        },
        InvalidAsciiScale {
            data: AsciiColumnData::Text(vec![Some("A".into())]),
            tscale: None,
            tzero: Some(3.0),
            keyword: "TZEROn",
        },
        InvalidAsciiScale {
            data: AsciiColumnData::Integer(vec![Some(1)]),
            tscale: Some(f64::NAN),
            tzero: None,
            keyword: "TSCALn",
        },
        InvalidAsciiScale {
            data: AsciiColumnData::Float(vec![Some(1.0)]),
            tscale: None,
            tzero: Some(f64::INFINITY),
            keyword: "TZEROn",
        },
    ];
    for case in cases {
        let columns = [AsciiWriteColumn {
            name: "BAD".into(),
            unit: None,
            data: case.data,
            width: 8,
            decimals: 1,
            tscale: case.tscale,
            tzero: case.tzero,
            tnull: None,
        }];
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_ascii_table(&write_table(1, &columns)),
            Err(FitsError::KeywordOutOfRange { name }) if name == case.keyword
        ));
        assert!(writer.into_inner().into_inner().is_empty());
    }
}

#[test]
fn ascii_nulls_round_trip_distinct_from_zero_and_text() {
    let columns = [
        AsciiWriteColumn {
            name: "LABEL".into(),
            unit: None,
            data: AsciiColumnData::Text(vec![Some("zero".into()), None, Some("star".into())]),
            width: 5,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: Some("NULL".into()),
        },
        AsciiWriteColumn {
            name: "COUNT".into(),
            unit: None,
            data: AsciiColumnData::Integer(vec![Some(0), None, Some(-2)]),
            width: 5,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: Some("NULL".into()),
        },
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_ascii_table(&write_table(3, &columns)).unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader.read_data_raw(1).unwrap().data()[..30],
        *b"zero     0NULL NULL star    -2"
    );

    let table = reader.read_ascii_table(1).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text(vec![Some("zero ".into()), None, Some("star ".into())])
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(0), None, Some(-2)])
    );
    let physical = table.column_by_idx(1).unwrap().physical().unwrap();
    assert_eq!(physical[0], 0.0);
    assert!(physical[1].is_nan());
    assert_eq!(physical[2], -2.0);
}

#[test]
fn ascii_tfields_beyond_999_is_rejected() {
    // §7.2.1 caps TFIELDS at 999; an absurd value must error, not size a huge Vec.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 0)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1000);
    assert!(matches!(
        AsciiTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "TFIELDS" })
    ));

    header
        .set_internal("NAXIS1", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 0)
        .set_internal("TFORM1", "A1");
    assert!(matches!(
        AsciiTable::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { name: "TBCOLn" })
    ));
}

#[test]
fn ascii_row_count_times_width_overflow_is_rejected() {
    // NAXIS2·NAXIS1 from untrusted axes must not wrap a usize to a small product.
    // 3e18 rows × 8 chars = 2.4e19 > usize::MAX, so `from_data` must error.
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "TABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8)
        .set_internal("NAXIS2", 3_000_000_000_000_000_000i64)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TBCOL1", 1)
        .set_internal("TFORM1", "I8");
    assert!(matches!(
        AsciiTable::from_data(&header, vec![0u8; 8]),
        Err(FitsError::UnexpectedEof)
    ));
}
