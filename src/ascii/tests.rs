use crate::ascii::internals::ascii_table_header;
use crate::ascii::*;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::ascii_builder::{AsciiFloatFormat, AsciiWriteColumn};
use crate::header_model::record_name::RecordName;
use crate::internals::cards::assert_cards;
use crate::internals::memory::{ascii_table, rejected_before_output, round_trip};
use crate::io::HduKind;
use crate::reader::FitsReader;
use std::io::Cursor;

#[test]
fn parses_ascii_tform_codes() {
    let fmt = |kind, width, decimals| AsciiFormat {
        kind,
        width,
        decimals,
    };
    assert_eq!(parse_ascii_tform("A8").unwrap(), fmt(AsciiKind::Text, 8, 0));
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
    assert!(matches!(
        parse_ascii_tform("Z3"),
        Err(FitsError::InvalidTform { tform }) if tform == "Z3"
    ));
}

/// A text column's rows are read by index and in order, an undefined row as `None`.
#[test]
fn ascii_text_reads_each_row() {
    let fields: AsciiText = [Some("x"), None, Some("yz")].into_iter().collect();
    assert_eq!(fields.row(2), Some("yz"));
    assert_eq!(
        fields.iter().collect::<Vec<_>>(),
        [Some("x"), None, Some("yz")]
    );
    assert_eq!((&fields).into_iter().len(), 3);
}

#[test]
fn decodes_hand_built_ascii_rows() {
    // Two columns: name `A4` at col 1, value `I6` at col 5 → row width 10.
    let mut header = ascii_table_header(10, 2, &[(1, "A4"), (5, "I6")]);
    header
        .set_internal("TTYPE1", "NAME")
        .set_internal("TTYPE2", "COUNT");
    let data = b"  AB   123def    -45".to_vec(); // "  AB" + "   123" ; "def " + "   -45"
    let table = Ascii::from_data(&header, data).unwrap();
    let schema = table.schema();
    assert_eq!(schema.row_count, 2);
    assert_eq!(schema.row_len, 10);
    assert_eq!(schema.columns[1].start, 4);
    assert_eq!(schema.column_index("count"), Some(1));
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text([Some("  AB"), Some("def ")].into_iter().collect())
    );
    assert_eq!(
        table.column(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(-45)])
    );
    // By-name access (case-insensitive) mirrors the by-index reads.
    assert_eq!(
        table.column("count").unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(-45)])
    );
    assert_eq!(
        table.column("COUNT").unwrap().physical().unwrap(),
        vec![123.0, -45.0]
    );
    assert!(matches!(
        table.column("missing"),
        Err(FitsError::ColumnNotFound { .. })
    ));

    // §7.2: the data unit is ASCII text, so a non-ASCII byte is rejected when the
    // table is parsed rather than deferred until whichever column happens to hold it
    // is read — here the corrupt byte sits in NAME while COUNT is well formed.
    let mut corrupt = b"  AB   123def    -45".to_vec();
    corrupt[2] = 0xFF;
    assert!(matches!(
        Ascii::from_data(&header, corrupt),
        Err(FitsError::InvalidAscii {
            text: TextOrigin::TableData
        })
    ));
}

#[test]
fn applies_tscal_tzero_and_maps_tnull_to_nan() {
    // One `I6` column, TSCAL=2, TZERO=10, TNULL='***': 123, blank zero, then null.
    let mut header = ascii_table_header(6, 3, &[(1, "I6")]);
    header
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", "***");
    let data = b"   123         ***".to_vec();
    let table = Ascii::from_data(&header, data).unwrap();
    // Raw preserves nullness; physical applies TZERO + TSCAL·field and maps null to NaN.
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(123), Some(0), None])
    );
    let phys = table.column(0).unwrap().physical().unwrap();
    assert_eq!(phys[0], 256.0); // 10 + 2·123
    assert_eq!(phys[1], 10.0); // blank field = stored zero
    assert!(phys[2].is_nan());

    // A bad optional keyword stays on its column: `TSCALn` and `TZEROn` fail the
    // physical read, `TNULLn` the raw one too, and `TDISPn` only its own accessor.
    for (keyword, raw_fails) in [("TSCAL1", false), ("TZERO1", false), ("TNULL1", true)] {
        let mut malformed = header.clone();
        malformed.set_internal(keyword, 1.5);
        if keyword != "TNULL1" {
            malformed.set_internal(keyword, "not numeric");
        }
        let table = Ascii::from_data(&malformed, b"   123         ***".to_vec()).unwrap();
        let defect = |error: FitsError| {
            matches!(
                error,
                FitsError::InvalidColumnKeyword { keyword: name, source }
                    if name == keyword
                        && matches!(*source, FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), .. }
                            if name == keyword)
            )
        };
        let column = table.column(0).unwrap();
        assert!(defect(column.physical().unwrap_err()), "{keyword}");
        assert_eq!(column.raw().is_err(), raw_fails, "{keyword}");
    }
    let mut display = header.clone();
    display.set_internal("TDISP1", 6);
    let table = Ascii::from_data(&display, b"   123         ***".to_vec()).unwrap();
    assert!(matches!(
        table.schema().columns[0].tdisp(),
        Err(FitsError::InvalidColumnKeyword { keyword, .. }) if keyword == "TDISP1"
    ));
    assert_eq!(table.column(0).unwrap().physical().unwrap()[0], 256.0);
}

#[test]
fn implicit_decimal_point_scales_by_ten_to_the_d() {
    // `F8.3`: a field with no explicit point has the point implied 3 from the right.
    let header = ascii_table_header(8, 2, &[(1, "F8.3")]);
    let data = b"   12345  12.345".to_vec(); // implicit "12345" → 12.345 ; explicit 12.345
    let table = Ascii::from_data(&header, data).unwrap();
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(12.345), Some(12.345)])
    );
}

#[test]
fn ascii_column_index_is_case_insensitive() {
    let mut header = ascii_table_header(4, 1, &[(1, "I4")]);
    header.set_internal("TTYPE1", "Count");
    let table = Ascii::from_data(&header, b"   7".to_vec()).unwrap();
    assert_eq!(table.schema().column_index("COUNT"), Some(0));
    assert_eq!(table.schema().column_index("count"), Some(0));
    // §7.2.2 and §6.7 are the same rule, so the ASCII form rejects the same
    // non-matches the binary one does.
    assert_eq!(table.schema().column_index("missing"), None);
    assert_eq!(table.schema().column_index(""), None);
}

#[test]
fn ascii_table_round_trips_through_write_and_read() {
    let mut columns = vec![
        AsciiWriteColumn::text(
            "NAME",
            [Some("  AB"), Some("beta")].into_iter().collect(),
            6,
        ),
        AsciiWriteColumn::integer("N", vec![Some(7), Some(-3)], 5).with_unit("count"),
        AsciiWriteColumn::float(
            "X",
            vec![Some(1.5), Some(-2.25)],
            AsciiFloatFormat::F {
                width: 8,
                decimals: 2,
            },
        ),
    ];
    let mut r = round_trip(|w| w.write_ascii_table(&ascii_table(2, &columns), None));

    assert_eq!(r.hdus().len(), 2); // auto dataless primary + the TABLE
    assert_eq!(r.hdus[1].kind, HduKind::AsciiTable);
    // The header alone gives the schema the read gives, and only for a `TABLE`.
    let schema = r.hdus[1].ascii_schema().unwrap().clone();
    assert_eq!((schema.row_count, schema.row_len), (2, 19));
    assert_eq!(schema.column_index("x"), Some(2));
    assert!(matches!(
        r.hdus[0].ascii_schema(),
        Err(FitsError::NotAnAsciiTable)
    ));
    assert_eq!(&r.read_data_unit(1).unwrap()[..6], b"  AB  ");
    let t = r.read_ascii_table(1).unwrap();
    assert_eq!(
        t.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text([Some("  AB  "), Some("beta  ")].into_iter().collect())
    );
    assert_eq!(
        t.column(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(7), Some(-3)])
    );
    let integers = t.column(1).unwrap().raw().unwrap();
    assert_eq!(integers.as_integer(), Some(&[Some(7), Some(-3)][..]));
    assert_eq!(integers.as_float(), None);
    assert_eq!(
        Vec::<Option<i64>>::try_from(integers).unwrap(),
        [Some(7), Some(-3)]
    );
    let floats = t.column(2).unwrap().raw().unwrap();
    assert!(matches!(
        AsciiText::try_from(floats),
        Err(AsciiColumnData::Float(_))
    ));
    assert_eq!(
        t.column(2).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(1.5), Some(-2.25)])
    );

    columns[0] = AsciiWriteColumn::text(
        "NAME",
        [Some("café"), Some("beta")].into_iter().collect(),
        6,
    );
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(&ascii_table(2, &columns), None)),
        FitsError::InvalidAscii {
            text: TextOrigin::Cell
        }
    ));
}

#[test]
fn signed_exponent_without_letter_parses_as_fortran_real() {
    // §7.2.5 rule 3(a): a numeric field may be terminated by a bare '+'/'-' that
    // introduces the exponent (no E/D letter), e.g. 3.14159-2 = 3.14159 × 10⁻².
    // Every value is the f64 nearest the decimal the field spells, as strtod (cfitsio,
    // astropy) reads it: `-3.0-1` is -0.3, not -3.0 · 0.1 = -0.30000000000000004.
    let exact = |got: Option<f64>, want: f64| {
        assert_eq!(got.expect("should parse").to_bits(), want.to_bits());
    };
    exact(parse_ascii_float("3.14159-2", 5), 0.031_415_9);
    exact(parse_ascii_float("2.5+3", 1), 2500.0);
    exact(parse_ascii_float("-3.0-1", 1), -0.3);
    // The leading mantissa sign is NOT an exponent; implicit decimal still applies.
    exact(parse_ascii_float("-12", 3), -0.012);
    // Explicit E/D forms keep working.
    exact(parse_ascii_float("1.5E2", 1), 150.0);
    exact(parse_ascii_float("1.5D-2", 1), 0.015);
    // A point-less mantissa with an *explicit* exponent is read literally (strtod),
    // NOT implied-decimal-scaled: `1E5` is 100000, `15E2` is 1500 — matching
    // cfitsio/astropy. The implied decimal applies only to point-less `Fw.d` fields.
    exact(parse_ascii_float("1E5", 3), 100_000.0);
    exact(parse_ascii_float("15E2", 3), 1500.0);
    exact(parse_ascii_float("2E-3", 4), 0.002);
    // Past 10²², `10f64.powi` is no longer exact; the literal stays exact to the last bit.
    exact(parse_ascii_float("7", 30), 7e-30);
    exact(parse_ascii_float("1.7976931348623157E308", 1), f64::MAX);
    exact(parse_ascii_float("1.0E+308", 1), 1e308);
    // A subnormal reads exactly too, where a scale by a power of ten would round twice.
    exact(parse_ascii_float("1.0E-320", 1), 1e-320);

    assert_eq!(
        split_mantissa_exponent("3.14159-2"),
        Some(Scientific {
            mantissa: "3.14159",
            exponent: "-2",
        })
    );
    assert_eq!(
        split_mantissa_exponent("-3.0-1"),
        Some(Scientific {
            mantissa: "-3.0",
            exponent: "-1",
        })
    );
    assert_eq!(
        split_mantissa_exponent("1.5E2"),
        Some(Scientific {
            mantissa: "1.5",
            exponent: "2",
        })
    );
    // The Fortran double `D`/`d` exponent splits in place (no String normalization).
    assert_eq!(
        split_mantissa_exponent("1.5D-2"),
        Some(Scientific {
            mantissa: "1.5",
            exponent: "-2",
        })
    );
    assert_eq!(split_mantissa_exponent("123"), None);
}

#[test]
fn reads_a_column_with_a_bare_sign_exponent_field() {
    // The letter-less exponent form (CFITSIO emits it) must read, not error.
    let header = ascii_table_header(12, 1, &[(1, "E12.5")]);
    let data = b"   3.14159-2".to_vec(); // 12 chars; 3.14159-2 = 0.0314159
    let table = Ascii::from_data(&header, data).unwrap();
    match table.column(0).unwrap().raw().unwrap() {
        AsciiColumnData::Float(values) => {
            assert_eq!(values, [Some(0.031_415_9)]);
        }
        other => panic!("expected Float, got {other:?}"),
    }
}

#[test]
fn ascii_write_emits_tscal_tzero_tnull_and_round_trips() {
    // A scaled integer column (raw values + TSCAL/TZERO) and a float column whose
    // undefined cell is written via TNULL and reads back as NaN (§7.2.2/§7.2.4).
    let columns = vec![
        AsciiWriteColumn::integer("RAW", vec![Some(5), Some(10)], 6).with_scaling(2.0, 100.0),
        AsciiWriteColumn::float(
            "FLUX",
            vec![Some(1.5), None],
            AsciiFloatFormat::F {
                width: 10,
                decimals: 3,
            },
        )
        .with_null("NULL"),
    ];
    let mut r = round_trip(|w| w.write_ascii_table(&ascii_table(2, &columns), None));

    assert_cards(
        &r.hdus[1].header,
        &[
            ("TSCAL1", 2.0.into()),
            ("TZERO1", 100.0.into()),
            ("TNULL2", "NULL".into()),
        ],
    );

    let t = r.read_ascii_table(1).unwrap();
    // Raw stored integers, then the scaled physical plane TZERO + TSCAL·field.
    assert_eq!(
        t.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(5), Some(10)])
    );
    assert_eq!(t.column(0).unwrap().physical().unwrap(), vec![110.0, 120.0]);
    // The TNULL-marked float cell reads back as NaN.
    let flux = t.column(1).unwrap().physical().unwrap();
    assert_eq!(flux[0], 1.5);
    assert!(flux[1].is_nan());
    assert_eq!(
        t.column(1).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(1.5), None])
    );

    for marker in [None, Some(""), Some("TOO-LONG")] {
        let column = AsciiWriteColumn::float(
            "BAD",
            vec![None],
            AsciiFloatFormat::F {
                width: 4,
                decimals: 1,
            },
        );
        let invalid = [match marker {
            Some(marker) => column.with_null(marker),
            None => column,
        }];
        assert!(matches!(
            rejected_before_output(|w| w.write_ascii_table(&ascii_table(1, &invalid), None)),
            FitsError::KeywordOutOfRange { keyword } if keyword == "TNULL1"
        ));
    }

    let collision = [AsciiWriteColumn::integer("BAD", vec![Some(0)], 4).with_null("0")];
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(&ascii_table(1, &collision), None)),
        FitsError::InvalidAsciiValue {
            reason: "the value equals its column's TNULLn marker"
        }
    ));

    let nonfinite = [AsciiWriteColumn::float(
        "BAD",
        vec![Some(f64::INFINITY)],
        AsciiFloatFormat::F {
            width: 8,
            decimals: 1,
        },
    )
    .with_null("NULL")];
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(&ascii_table(1, &nonfinite), None)),
        FitsError::InvalidAsciiValue {
            reason: "float cells must be finite; use None for null"
        }
    ));
}

#[test]
fn ascii_writer_accepts_exact_width_values() {
    let columns = [
        AsciiWriteColumn::text("TEXT", [Some("abc")].into_iter().collect(), 3),
        AsciiWriteColumn::integer("INT", vec![Some(-12)], 3),
        AsciiWriteColumn::float(
            "FLOAT",
            vec![Some(1.25)],
            AsciiFloatFormat::F {
                width: 4,
                decimals: 2,
            },
        ),
        AsciiWriteColumn::float(
            "NULL",
            vec![None],
            AsciiFloatFormat::F {
                width: 4,
                decimals: 1,
            },
        )
        .with_null("NULL"),
        // `-1.25E-30` fills 9 of 10, where `F` would round it to zero.
        AsciiWriteColumn::float(
            "EXPONENT",
            vec![Some(-1.25e-30)],
            AsciiFloatFormat::E {
                width: 10,
                decimals: 2,
            },
        ),
        // 12346 rounds to 3 decimals as `1.235`, ×10⁴.
        AsciiWriteColumn::float(
            "DOUBLE",
            vec![Some(12346.0)],
            AsciiFloatFormat::D {
                width: 9,
                decimals: 3,
            },
        ),
        // No decimals still write the point.
        AsciiWriteColumn::float(
            "WHOLE",
            vec![Some(2.0)],
            AsciiFloatFormat::F {
                width: 3,
                decimals: 0,
            },
        ),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_ascii_table(&ascii_table(1, &columns), None)
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        &reader.read_data_unit(1).unwrap()[..36],
        b"abc-121.25NULL -1.25E-301.235D+04 2."
    );
    let header = &reader.hdus()[1].header;
    let tforms: Vec<_> = (1..=7)
        .map(|n| {
            header
                .get_text(format!("TFORM{n}").as_str())
                .unwrap()
                .unwrap()
                .to_string()
        })
        .collect();
    assert_eq!(
        tforms,
        ["A3", "I3", "F4.2", "F4.1", "E10.2", "D9.3", "F3.0"]
    );
    let table = reader.read_ascii_table(1).unwrap();
    let floats: Vec<_> = (4..7)
        .map(|column| table.column(column).unwrap().raw().unwrap())
        .collect();
    assert_eq!(
        floats,
        [
            AsciiColumnData::Float(vec![Some(-1.25e-30)]),
            AsciiColumnData::Float(vec![Some(12350.0)]),
            AsciiColumnData::Float(vec![Some(2.0)]),
        ]
    );
}

#[derive(Debug)]
struct AsciiOverflowCase {
    column: AsciiWriteColumn,
    name: &'static str,
    width: usize,
    row_count: usize,
    row: usize,
    minimum_width: usize,
}

#[test]
fn ascii_writer_rejects_one_byte_overflow_before_output() {
    let cases = [
        AsciiOverflowCase {
            column: AsciiWriteColumn::text(
                "TEXT",
                [Some("ok"), Some("abcd")].into_iter().collect(),
                3,
            ),
            name: "TEXT",
            width: 3,
            row_count: 2,
            row: 1,
            minimum_width: 4,
        },
        AsciiOverflowCase {
            column: AsciiWriteColumn::integer("INT", vec![Some(-123)], 3),
            name: "INT",
            width: 3,
            row_count: 1,
            row: 0,
            minimum_width: 4,
        },
        AsciiOverflowCase {
            column: AsciiWriteColumn::float(
                "FLOAT",
                vec![Some(-1.25)],
                AsciiFloatFormat::F {
                    width: 4,
                    decimals: 2,
                },
            ),
            name: "FLOAT",
            width: 4,
            row_count: 1,
            row: 0,
            minimum_width: 5,
        },
        // 1e9 is `1.0E+09` as `E7.1`, and `10.000000E+08` never.
        AsciiOverflowCase {
            column: AsciiWriteColumn::float(
                "EXPONENT",
                vec![Some(-1e9)],
                AsciiFloatFormat::E {
                    width: 7,
                    decimals: 1,
                },
            ),
            name: "EXPONENT",
            width: 7,
            row_count: 1,
            row: 0,
            minimum_width: 8,
        },
    ];
    for case in cases {
        let column_name = case.name;
        let width = case.width;
        assert!(matches!(
            rejected_before_output(|w| w.write_ascii_table(&ascii_table(case.row_count, &[case.column]), None)),
            FitsError::AsciiFieldTooWide {
                column,
                row,
                width: actual_width,
                minimum_width,
            } if column == column_name
                && row == case.row
                && actual_width == width
                && minimum_width == case.minimum_width
        ));
    }

    // `F1.n` holds no cell: a digit, the point and n decimals need n + 2 characters, and
    // `E7.2` none either, whose `1.00E+00` takes 8.
    for (format, minimum_width) in [
        (
            AsciiFloatFormat::F {
                width: 1,
                decimals: usize::MAX,
            },
            usize::MAX,
        ),
        (
            AsciiFloatFormat::E {
                width: 7,
                decimals: 2,
            },
            8,
        ),
    ] {
        let narrow = [AsciiWriteColumn::float("PRECISION", Vec::new(), format)];
        assert!(matches!(
            rejected_before_output(|w| w.write_ascii_table(&ascii_table(0, &narrow), None)),
            FitsError::AsciiFloatFormatTooNarrow {
                column,
                width,
                minimum_width: needed,
            } if column == "PRECISION" && width == format.width() && needed == minimum_width
        ));
    }

    let columns = [AsciiWriteColumn::float(
        "NULL",
        vec![None],
        AsciiFloatFormat::F {
            width: 3,
            decimals: 1,
        },
    )
    .with_null("NULL")];
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(&ascii_table(1, &columns), None)),
        FitsError::KeywordOutOfRange { keyword } if keyword == "TNULL1"
    ));
}

/// A scaling a column's values take none of, or one that is not finite, names the
/// first keyword at fault: text takes no `TSCALn`, and a number no infinite `TZEROn`.
#[test]
fn ascii_scaling_metadata_is_validated_by_stored_type_before_output() {
    let format = AsciiFloatFormat::F {
        width: 8,
        decimals: 1,
    };
    let cases = [
        (
            AsciiWriteColumn::text("BAD", [Some("A")].into_iter().collect(), 8)
                .with_scaling(2.0, 0.0),
            "TSCAL1",
        ),
        (
            AsciiWriteColumn::integer("BAD", vec![Some(1)], 8).with_scaling(f64::NAN, 0.0),
            "TSCAL1",
        ),
        (
            AsciiWriteColumn::float("BAD", vec![Some(1.0)], format)
                .with_scaling(1.0, f64::INFINITY),
            "TZERO1",
        ),
    ];
    for (column, expected) in cases {
        let columns = [column];
        assert!(matches!(
            rejected_before_output(|w| w.write_ascii_table(&ascii_table(1, &columns), None)),
            FitsError::KeywordOutOfRange { keyword } if keyword == expected
        ));
    }
}

#[test]
fn ascii_nulls_round_trip_distinct_from_zero_and_text() {
    let columns = [
        AsciiWriteColumn::text(
            "LABEL",
            [Some("zero"), None, Some("star")].into_iter().collect(),
            5,
        )
        .with_null("NULL"),
        AsciiWriteColumn::integer("COUNT", vec![Some(0), None, Some(-2)], 5).with_null("NULL"),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_ascii_table(&ascii_table(3, &columns), None)
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader.read_data_unit(1).unwrap()[..30],
        *b"zero     0NULL NULL star    -2"
    );

    let table = reader.read_ascii_table(1).unwrap();
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        AsciiColumnData::Text([Some("zero "), None, Some("star ")].into_iter().collect())
    );
    assert_eq!(
        table.column(1).unwrap().raw().unwrap(),
        AsciiColumnData::Integer(vec![Some(0), None, Some(-2)])
    );
    let physical = table.column(1).unwrap().physical().unwrap();
    assert_eq!(physical[0], 0.0);
    assert!(physical[1].is_nan());
    assert_eq!(physical[2], -2.0);
}

/// §7.2.1 fixes `BITPIX = 8`, `NAXIS = 2`, `PCOUNT = 0` and `GCOUNT = 1`, and caps
/// `TFIELDS` at 999.
#[test]
fn ascii_table_structure_out_of_range_is_rejected() {
    for (keyword, value) in [
        (keywords::BITPIX, 16),
        (keywords::NAXIS, 1),
        (keywords::PCOUNT, 4),
        (keywords::GCOUNT, 3),
    ] {
        let mut header = ascii_table_header(0, 0, &[]);
        header.set_internal(keyword, value);
        assert!(
            matches!(
                AsciiSchema::parse(&header),
                Err(FitsError::KeywordOutOfRange { keyword: got }) if got == keyword
            ),
            "{keyword} = {value}"
        );
    }

    let mut header = ascii_table_header(0, 0, &[]);
    header.set_internal(keywords::TFIELDS, 1000);
    assert!(matches!(
        Ascii::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange {
            keyword: keywords::TFIELDS
        })
    ));

    header
        .set_internal("NAXIS1", 1)
        .set_internal(keywords::TFIELDS, 1)
        .set_internal("TBCOL1", 0)
        .set_internal("TFORM1", "A1");
    assert!(matches!(
        Ascii::from_data(&header, vec![]),
        Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "TBCOL1"
    ));
}

#[test]
fn ascii_row_count_times_width_overflow_is_rejected() {
    // NAXIS2·NAXIS1 from untrusted axes must not wrap a usize to a small product.
    // 3e18 rows × 8 chars = 2.4e19 > usize::MAX, so `from_data` must error.
    let header = ascii_table_header(8, 3_000_000_000_000_000_000, &[(1, "I8")]);
    assert!(matches!(
        Ascii::from_data(&header, vec![0u8; 8]),
        Err(FitsError::DataUnitOverflow)
    ));
}
