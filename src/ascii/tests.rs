use crate::ascii::ascii_tform::AsciiFloatFormat;
use crate::ascii::internals::ascii_table_header;
use crate::ascii::*;
use crate::fits_error::AsciiValueDefect;
use crate::fits_writer::ascii_builder::AsciiWriteColumn;
use crate::header_model::record_name::RecordName;
use crate::internals::cards;
use crate::internals::memory;
use crate::internals::samples;
use crate::internals::whole_column::WholeAsciiColumn;
use crate::io::HduKind;
use std::assert_matches;

/// Each `TFORMn` form parses to its type, width and decimals, the float letter kept, and
/// writes back as the standard spells it; a float form with no `.d` takes 0 decimals.
#[test]
fn parses_ascii_tform_codes() {
    for (text, tform, written) in [
        ("A8", AsciiTform::Text { width: 8 }, "A8"),
        ("I10", AsciiTform::Integer { width: 10 }, "I10"),
        (
            "F8.2",
            AsciiTform::Float(AsciiFloatFormat::F {
                width: 8,
                decimals: 2,
            }),
            "F8.2",
        ),
        (
            "E15.7",
            AsciiTform::Float(AsciiFloatFormat::E {
                width: 15,
                decimals: 7,
            }),
            "E15.7",
        ),
        (
            " D25.17 ",
            AsciiTform::Float(AsciiFloatFormat::D {
                width: 25,
                decimals: 17,
            }),
            "D25.17",
        ),
        (
            "F6",
            AsciiTform::Float(AsciiFloatFormat::F {
                width: 6,
                decimals: 0,
            }),
            "F6.0",
        ),
    ] {
        let parsed: AsciiTform = text.parse().unwrap();
        assert_eq!(parsed, tform, "{text:?}");
        assert_eq!(parsed.to_string(), written, "{text:?}");
        assert_eq!(written.parse::<AsciiTform>().unwrap(), tform, "{written:?}");
        assert_eq!(parsed.width(), tform.width(), "{text:?}");
    }
    for text in ["Z3", "", "A", "Fx.2"] {
        assert_matches!(
            text.parse::<AsciiTform>(),
            Err(FitsError::InvalidTform { tform }) if tform == text
        );
    }
}

/// A text column's rows are read by index and in order, an undefined row as `None`. A
/// field keeps its leading blanks and drops its trailing ones, so an all-blank field is
/// `""`, which a null is not.
#[test]
fn text_column_reads_each_row() {
    let fields: AsciiText = [Some("x"), None, Some("yz")].into_iter().collect();
    assert_eq!(fields.row(2), Some("yz"));
    assert_eq!(
        fields.iter().collect::<Vec<_>>(),
        [Some("x"), None, Some("yz")]
    );
    assert_eq!((&fields).into_iter().len(), 3);

    let padded: AsciiText = [Some(" a  "), Some("   "), None, Some("b\t ")]
        .into_iter()
        .collect();
    assert_eq!(
        padded.iter().collect::<Vec<_>>(),
        [Some(" a"), Some(""), None, Some("b\t")]
    );
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
    assert_eq!(schema.row_count(), 2);
    assert_eq!(schema.row_len(), 10);
    assert_eq!(schema.columns[1].byte_offset(), 4);
    assert_eq!(schema.column_index("count").unwrap(), Some(1));
    let AsciiColumnData::Text(names) = table.column(0).unwrap().raw().unwrap() else {
        panic!("NAME is a text column");
    };
    assert_eq!(
        names.iter().collect::<Vec<_>>(),
        [Some("  AB"), Some("def")]
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
    // One buffer serves both columns, taking each one's kind; the physical values land
    // after what an `f32` buffer holds.
    let mut fields = AsciiColumnData::Float(vec![Some(9.0)]);
    table.column(1).unwrap().raw_into(&mut fields).unwrap();
    assert_eq!(fields, AsciiColumnData::Integer(vec![Some(123), Some(-45)]));
    table.column(0).unwrap().raw_into(&mut fields).unwrap();
    assert_eq!(fields, AsciiColumnData::Text(names));
    let mut narrow: Vec<f32> = vec![0.5];
    table.column(1).unwrap().physical_into(&mut narrow).unwrap();
    assert_eq!(narrow, [0.5, 123.0, -45.0]);
    assert_matches!(
        table.column("missing"),
        Err(FitsError::ColumnNotFound { name }) if name == "missing"
    );

    // §7.2: the data unit is ASCII text, so a non-ASCII byte is rejected when the
    // table is parsed rather than deferred until whichever column happens to hold it
    // is read — here the corrupt byte sits in NAME while COUNT is well formed.
    let mut corrupt = b"  AB   123def    -45".to_vec();
    corrupt[2] = 0xFF;
    assert_matches!(
        Ascii::from_data(&header, corrupt),
        Err(FitsError::InvalidAscii {
            text: TextOrigin::TableData
        })
    );
}

/// A physical value is one fused multiply-add: with TSCAL = 0.1 and TZERO = −0.3, the
/// field 3 gives fma(0.1, 3, −0.3) = 2.7755575615628914e-17, where the product rounded
/// first gives 5.551115123125783e-17. `nan` and `inf` are no §7.2.5 number, nor is an
/// exponent past the f64 range.
#[test]
fn physical_values_fuse_the_scaling_and_refuse_non_finite_fields() {
    let mut header = ascii_table_header(8, 1, &[(1, "F8.0")]);
    header
        .set_internal("TSCAL1", 0.1)
        .set_internal("TZERO1", -0.3);
    let table = Ascii::from_data(&header, b"       3".to_vec()).unwrap();
    assert_eq!(
        table.column(0).unwrap().physical().unwrap(),
        [2.7755575615628914e-17]
    );
    for field in ["     nan", "     inf", "   1E400"] {
        let table = Ascii::from_data(&header, field.as_bytes().to_vec()).unwrap();
        assert_matches!(
            table.column(0).unwrap().physical(),
            Err(FitsError::InvalidAsciiField { column: 0, row: 0, field: got }) if got == field.trim()
        );
    }
    // The refusal names the field's column and row, in either read.
    let two = ascii_table_header(8, 2, &[(1, "I4"), (5, "I4")]);
    let table = Ascii::from_data(&two, b"   1   2   3  zz".to_vec()).unwrap();
    assert_matches!(
        table.column(1).unwrap().raw(),
        Err(FitsError::InvalidAsciiField { column: 1, row: 1, field }) if field == "zz"
    );
    assert_matches!(
        table.column(1).unwrap().physical(),
        Err(FitsError::InvalidAsciiField { column: 1, row: 1, field }) if field == "zz"
    );
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
    // 10 + 2·123, then a blank field, a stored zero, then the null.
    assert_eq!(
        samples::nan_blind_of(&phys),
        samples::nan_blind_of(&[256.0, 10.0, f64::NAN])
    );

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
        if raw_fails {
            assert!(defect(column.raw().unwrap_err()), "{keyword}");
        } else {
            assert_eq!(
                column.raw().unwrap(),
                AsciiColumnData::Integer(vec![Some(123), Some(0), None]),
                "{keyword}"
            );
        }
    }
    let mut display = header.clone();
    display.set_internal("TDISP1", 6);
    let table = Ascii::from_data(&display, b"   123         ***".to_vec()).unwrap();
    assert_matches!(
        table.schema().columns[0].tdisp(),
        Err(FitsError::InvalidColumnKeyword { keyword, .. }) if keyword == "TDISP1"
    );
    assert_eq!(table.column(0).unwrap().physical().unwrap()[0], 256.0);

    // A `TTYPEn` or `TUNITn` that does not read fails its accessor and a lookup by name,
    // as the column may carry that name; an index read does not.
    for keyword in ["TTYPE1", "TUNIT1"] {
        let mut labelled = header.clone();
        labelled.set_internal(keyword, 7);
        let table = Ascii::from_data(&labelled, b"   123         ***".to_vec()).unwrap();
        let column = &table.schema().columns[0];
        let accessor = if keyword == "TTYPE1" {
            column.name()
        } else {
            column.unit()
        };
        assert_matches!(
            accessor,
            Err(FitsError::InvalidColumnKeyword { keyword: name, .. }) if name == keyword
        );
        assert_eq!(table.column(0).unwrap().physical().unwrap()[0], 256.0);
        let lookup = table.schema().column_index("FLUX");
        if keyword == "TTYPE1" {
            assert_matches!(
                lookup,
                Err(FitsError::InvalidColumnKeyword { keyword: name, .. }) if name == keyword
            );
        } else {
            assert_eq!(lookup.unwrap(), None);
        }
    }
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
fn table_round_trips_through_write_and_read() {
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
    let file = memory::written(|w| w.write_ascii_table(&memory::ascii_table(2, &columns), None));
    let mut r = file.stream();

    assert_eq!(r.hdus().len(), 2); // auto dataless primary + the TABLE
    assert_eq!(r.hdus[1].kind, HduKind::AsciiTable);
    // The header alone gives the schema the read gives.
    let schema = r.hdus[1].ascii_schema().unwrap().clone();
    assert_eq!((schema.row_count(), schema.row_len()), (2, 19));
    assert_eq!(schema.column_index("x").unwrap(), Some(2));
    assert_eq!(&r.read_data_unit(1).unwrap()[..6], b"  AB  ");
    let t = r.read_ascii_table(1).unwrap();
    // The field pads to its width of 6 and reads back as it was written.
    let AsciiColumnData::Text(names) = t.column(0).unwrap().raw().unwrap() else {
        panic!("NAME is a text column");
    };
    assert_eq!(
        names.iter().collect::<Vec<_>>(),
        [Some("  AB"), Some("beta")]
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
    assert_matches!(AsciiText::try_from(floats), Err(AsciiColumnData::Float(_)));
    assert_eq!(
        t.column(2).unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(1.5), Some(-2.25)])
    );

    columns[0] = AsciiWriteColumn::text(
        "NAME",
        [Some("café"), Some("beta")].into_iter().collect(),
        6,
    );
    assert_matches!(
        memory::refused_before_output(
            |w| w.write_ascii_table(&memory::ascii_table(2, &columns), None)
        ),
        FitsError::InvalidAscii {
            text: TextOrigin::Cell
        }
    );
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
    // A point-less mantissa takes its implied point with an exponent too, as cfitsio's
    // `fffstrr8` reads it (design.md): `1E5` in `E10.2` is 1·10⁵⁻² = 1000, `15E2` with
    // 3 decimals 15·10²⁻³ = 1.5, and `2E-3` with 4 decimals 2·10⁻³⁻⁴ = 2e-7.
    exact(parse_ascii_float("1E5", 2), 1000.0);
    exact(parse_ascii_float("15E2", 3), 1.5);
    exact(parse_ascii_float("2E-3", 4), 2e-7);
    exact(parse_ascii_float("2.5E-3", 4), 0.0025);
    // Past 10²², `10f64.powi` is no longer exact; the literal stays exact to the last bit.
    exact(parse_ascii_float("7", 30), 7e-30);
    exact(parse_ascii_float("1.7976931348623157E308", 1), f64::MAX);
    exact(parse_ascii_float("1.0E+308", 1), 1e308);
    // A subnormal reads exactly too, where a scale by a power of ten would round twice.
    exact(parse_ascii_float("1.0E-320", 1), 1e-320);
    // A field that is a Rust literal reads as one, yet an overflow is refused, and so is an
    // exponent past `i64`, which a literal parse would take as 0.
    assert_eq!(parse_ascii_float("1.0E+309", 1), None);
    assert_eq!(parse_ascii_float("1.0E-99999999999999999999", 1), None);
    assert_eq!("1.0E-99999999999999999999".parse::<f64>(), Ok(0.0));

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
fn write_emits_tscal_tzero_tnull_and_round_trips() {
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
    let file = memory::written(|w| w.write_ascii_table(&memory::ascii_table(2, &columns), None));
    let mut r = file.stream();

    cards::assert_cards(
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
    assert_eq!(
        samples::nan_blind_of(&flux),
        samples::nan_blind_of(&[1.5, f64::NAN])
    );
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
        assert_matches!(
            memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(1, &invalid), None)),
            FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TNULL1"
        );
    }

    let collision = [AsciiWriteColumn::integer("BAD", vec![Some(0)], 4).with_null("0")];
    assert_matches!(
        memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(1, &collision), None)),
        FitsError::InvalidAsciiValue {
            column,
            row: 0,
            defect: AsciiValueDefect::NullMarker,
        } if column == "BAD"
    );

    let nonfinite = [AsciiWriteColumn::float(
        "BAD",
        vec![Some(f64::INFINITY)],
        AsciiFloatFormat::F {
            width: 8,
            decimals: 1,
        },
    )
    .with_null("NULL")];
    assert_matches!(
        memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(1, &nonfinite), None)),
        FitsError::InvalidAsciiValue {
            column,
            row: 0,
            defect: AsciiValueDefect::NonFinite,
        } if column == "BAD"
    );
}

#[test]
fn writer_accepts_exact_width_values() {
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
    let file = memory::written(|w| w.write_ascii_table(&memory::ascii_table(1, &columns), None));
    let mut reader = file.stream();
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
    // The reader holds each column's form as the writer wrote it, its float letter too.
    let read: Vec<String> = table
        .schema()
        .columns
        .iter()
        .map(|column| column.tform.to_string())
        .collect();
    assert_eq!(read, tforms);
    assert_eq!(
        table.schema().columns[4].tform,
        AsciiTform::Float(AsciiFloatFormat::E {
            width: 10,
            decimals: 2,
        })
    );
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
fn writer_refuses_a_one_byte_overflow_before_output() {
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
        assert_matches!(
            memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(case.row_count, &[case.column]), None)),
            FitsError::AsciiFieldTooWide {
                column,
                row,
                width: actual_width,
                minimum_width,
            } if column == column_name
                && row == case.row
                && actual_width == width
                && minimum_width == case.minimum_width
        );
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
        assert_matches!(
            memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(0, &narrow), None)),
            FitsError::AsciiFloatFormatTooNarrow {
                column,
                width,
                minimum_width: needed,
            } if column == "PRECISION" && width == format.width() && needed == minimum_width
        );
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
    assert_matches!(
        memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(1, &columns), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TNULL1"
    );
}

/// A text column takes no `TSCALn`. The non-finite scalings of the number columns sit
/// with the writer's other scalings, in `nonfinite_scaling_is_refused_before_output`.
#[test]
fn text_column_refuses_a_scaling() {
    let columns = [
        AsciiWriteColumn::text("BAD", [Some("A")].into_iter().collect(), 8).with_scaling(2.0, 0.0),
    ];
    assert_matches!(
        memory::refused_before_output(|w| w.write_ascii_table(&memory::ascii_table(1, &columns), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TSCAL1"
    );
}

#[test]
fn nulls_round_trip_distinct_from_zero_and_text() {
    let columns = [
        AsciiWriteColumn::text(
            "LABEL",
            [Some("zero"), None, Some("star")].into_iter().collect(),
            5,
        )
        .with_null("NULL"),
        AsciiWriteColumn::integer("COUNT", vec![Some(0), None, Some(-2)], 5).with_null("NULL"),
    ];
    let file = memory::written(|w| w.write_ascii_table(&memory::ascii_table(3, &columns), None));
    let mut reader = file.stream();
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
    assert_eq!(
        samples::nan_blind_of(&physical),
        samples::nan_blind_of(&[0.0, f64::NAN, -2.0])
    );
}
