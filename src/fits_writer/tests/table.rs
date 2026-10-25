//! Binary tables written and read back: every column kind, scaling and null, and the
//! checks before the first byte.

use crate::fits_writer::tests::*;

#[test]
fn table_writers_enforce_the_exact_tfields_limit_before_output() {
    let binary = empty_binary_columns(MAX_TABLE_FIELDS);
    let file = memory::written(|w| w.write_table(&memory::binary_table(0, &binary), None));
    let reader = file.stream();
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::TFIELDS)
            .unwrap(),
        Some(i64::try_from(MAX_TABLE_FIELDS).unwrap())
    );

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS);
    let file = memory::written(|w| w.write_ascii_table(&memory::ascii_table(0, &ascii), None));
    let reader = file.stream();
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::TFIELDS)
            .unwrap(),
        Some(i64::try_from(MAX_TABLE_FIELDS).unwrap())
    );

    let binary = empty_binary_columns(MAX_TABLE_FIELDS + 1);
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(0, &binary), None)),
        FitsError::InvalidKeywordValue {
            keyword: keywords::TFIELDS,
            ..
        }
    );

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS + 1);
    assert_matches!(
        memory::refused_before_output(
            |w| w.write_ascii_table(&memory::ascii_table(0, &ascii), None)
        ),
        FitsError::InvalidKeywordValue {
            keyword: keywords::TFIELDS,
            ..
        }
    );
}

#[test]
fn writes_and_reads_back_variable_length_arrays() {
    // A fixed column plus a `P` VLA column with rows of differing length.
    let vla_rows = vec![
        ColumnData::I32(vec![10, 20]),
        ColumnData::I32(vec![]), // empty cell
        ColumnData::I32(vec![1, 2, 3, 4, 5]),
    ];
    let columns = vec![
        WriteColumn::fixed("ID", ColumnData::I32(vec![1, 2, 3]), 1),
        WriteColumn::vla(
            "DATA",
            Ragged::from_rows(vla_rows.clone()).unwrap(),
            DescriptorWidth::P,
        ),
    ];
    let file = memory::written(|w| w.write_table(&memory::binary_table(3, &columns), None));
    let mut r = file.stream();
    let table = r.read_table(1).unwrap();
    // A P descriptor sized to the longest row, 5.
    assert_eq!(r.hdus[1].header.get_text("TFORM2").unwrap(), Some("1PJ(5)"));
    let got = table.column(1).unwrap().vla().unwrap();
    assert_eq!(got, Ragged::from_rows(vla_rows).unwrap());

    let empty = [WriteColumn::vla(
        "EMPTY",
        Ragged::<ColumnData>::new(ColumnData::I64(Vec::new()), Vec::new()),
        DescriptorWidth::P,
    )];
    let file = memory::written(|w| w.write_table(&memory::binary_table(0, &empty), None));
    let mut r = file.stream();
    let table = r.read_table(1).unwrap();
    assert_eq!(table.schema().columns[0].tform.element(), ElementKind::I64);
    assert!(table.column(0).unwrap().vla().unwrap().is_empty());
}

#[test]
fn writes_tdim_p_q_vla_and_bit_columns() {
    let columns = vec![
        // 2×2 multidimensional column (TDIM '(2,2)'), 4 elements/row.
        WriteColumn::fixed("MAT", ColumnData::I32((1..=8).collect()), 4).with_tdim(vec![2, 2]),
        // 64-bit Q VLA column.
        WriteColumn::vla(
            "QV",
            Ragged::from_rows(vec![
                ColumnData::I16(vec![]),
                ColumnData::I16(vec![7, 8, 9, 10]),
            ])
            .unwrap(),
            DescriptorWidth::Q,
        )
        .with_tdim(vec![2, 2]),
        WriteColumn::vla(
            "PV",
            Ragged::from_rows(vec![
                ColumnData::I16(vec![]),
                ColumnData::I16(vec![1, 2, 3, 4]),
            ])
            .unwrap(),
            DescriptorWidth::P,
        )
        .with_tdim(vec![2, 2]),
        WriteColumn::vla(
            "TXT",
            Ragged::from_rows(vec![
                ColumnData::Character(b"hello".to_vec()),
                ColumnData::Character(b"abc".to_vec()),
            ])
            .unwrap(),
            DescriptorWidth::P,
        ),
        // 12-bit X column: 2 bytes/row, the 4 bits past the width zeroed.
        WriteColumn::bits(
            "FLAGS",
            [
                bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0],
                bitvec![u8, Msb0; 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1],
            ],
            12,
        )
        .unwrap(),
    ];
    let table = memory::binary_table(2, &columns);
    let file = memory::written(|w| w.write_table(&table, None));
    let mut r = file.stream();
    let t = r.read_table(1).unwrap();
    let metadata = t.schema();

    // The data unit, encoded a bounded chunk at a time at any chunk size, is the one the
    // file holds. A row is 16 + 16 + 8 + 8 + 2 = 50 bytes and the largest array 8, so a
    // band of rows holds at most the chunk or one row, and the heap the chunk and one
    // array past it.
    let start = usize::try_from(r.hdus[1].data_offset).unwrap();
    let unit = &file.bytes()[start..start + usize::try_from(r.hdus[1].data_bytes).unwrap()];
    let LaidOut { layout, .. } = table.laid_out().unwrap();
    for chunk_bytes in [1, 50, 101, 1 << 20] {
        let (mut encoded, mut chunk) = (Vec::new(), Vec::new());
        table
            .encode_chunks(&layout, chunk_bytes, &mut chunk, |bytes| {
                assert!(
                    bytes.len() <= chunk_bytes.max(50) + 8,
                    "{chunk_bytes}: {}",
                    bytes.len()
                );
                encoded.extend_from_slice(bytes);
                Ok(())
            })
            .unwrap();
        assert_eq!(encoded, unit, "{chunk_bytes}");
    }

    // TDIM parsed back as a shape.
    assert_eq!(metadata.columns[0].tdim().unwrap(), Some(&[2, 2][..]));
    // P/Q descriptors with TDIM accept an empty row and validate the nonempty row.
    assert_eq!(
        metadata.columns[1].tform.storage(),
        Storage::Array(DescriptorWidth::Q)
    );
    assert_eq!(metadata.columns[1].tdim().unwrap(), Some(&[2, 2][..]));
    assert_eq!(
        t.column(1).unwrap().vla().unwrap(),
        Ragged::from_rows([ColumnData::I16(vec![]), ColumnData::I16(vec![7, 8, 9, 10])]).unwrap()
    );
    assert_eq!(
        metadata.columns[2].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[2].tdim().unwrap(), Some(&[2, 2][..]));
    assert_eq!(
        t.column(2).unwrap().vla().unwrap(),
        Ragged::from_rows([ColumnData::I16(vec![]), ColumnData::I16(vec![1, 2, 3, 4])]).unwrap()
    );
    assert_eq!(r.hdus[1].header.get_text("TFORM4").unwrap(), Some("1PA(5)"));
    assert_eq!(metadata.columns[3].tform.repeat(), 1);
    assert_eq!(
        metadata.columns[3].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[3].tform.element(), ElementKind::Character);
    assert_eq!(
        t.column(3).unwrap().vla().unwrap(),
        Ragged::from_rows([
            ColumnData::Character(b"hello".to_vec()),
            ColumnData::Character(b"abc".to_vec())
        ])
        .unwrap()
    );
    assert_eq!(metadata.columns[4].tform.element(), ElementKind::Bit);
    assert_eq!(metadata.columns[4].tform.repeat(), 12);
    // Each row's 12 bits, MSB-first, with no padding between the rows: 0xAB, 0xC
    // then 0x12, 0x3.
    assert_eq!(
        t.column(4).unwrap().raw().unwrap(),
        ColumnData::Bit(bitvec![
            u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1
        ])
    );
}

#[test]
fn writes_p_q_variable_length_bit_arrays_with_exact_counts_and_padding() {
    let mut one_bit = bitvec![u8, Msb0; 1; 8];
    one_bit.truncate(1);
    let mut p_nine = bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1];
    p_nine.truncate(9);
    let mut q_nine = bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1];
    q_nine.truncate(9);
    assert_eq!(one_bit.as_raw_slice(), &[0xFF]);
    assert_eq!(p_nine.as_raw_slice(), &[0xAA, 0xFF]);
    assert_eq!(q_nine.as_raw_slice(), &[0x55, 0x7F]);
    let p_rows = [BitVec::<u8, Msb0>::new(), one_bit.clone(), p_nine];
    let q_rows = [BitVec::<u8, Msb0>::new(), one_bit, q_nine];
    let columns = [
        WriteColumn::vla(
            "PFLAGS",
            Ragged::from_rows(p_rows.iter().cloned().map(ColumnData::Bit)).unwrap(),
            DescriptorWidth::P,
        ),
        WriteColumn::vla(
            "QFLAGS",
            Ragged::from_rows(q_rows.iter().cloned().map(ColumnData::Bit)).unwrap(),
            DescriptorWidth::Q,
        ),
    ];
    let file = memory::written(|w| w.write_table(&memory::binary_table(3, &columns), None));
    let mut reader = file.stream();

    cards::assert_cards(
        &reader.hdus[1].header,
        &[
            ("TFORM1", "1PX(9)".into()),
            ("TFORM2", "1QX(9)".into()),
            ("PCOUNT", 6.into()),
        ],
    );

    let raw = reader.read_data_unit(1).unwrap();
    assert_eq!(i32::from_be_bytes(raw[0..4].try_into().unwrap()), 0);
    assert_eq!(i32::from_be_bytes(raw[4..8].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[8..16].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[16..24].try_into().unwrap()), 0);
    assert_eq!(i32::from_be_bytes(raw[24..28].try_into().unwrap()), 1);
    assert_eq!(i32::from_be_bytes(raw[28..32].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[32..40].try_into().unwrap()), 1);
    assert_eq!(i64::from_be_bytes(raw[40..48].try_into().unwrap()), 1);
    assert_eq!(i32::from_be_bytes(raw[48..52].try_into().unwrap()), 9);
    assert_eq!(i32::from_be_bytes(raw[52..56].try_into().unwrap()), 2);
    assert_eq!(i64::from_be_bytes(raw[56..64].try_into().unwrap()), 9);
    assert_eq!(i64::from_be_bytes(raw[64..72].try_into().unwrap()), 4);
    assert_eq!(&raw[72..78], &[0x80, 0x80, 0xAA, 0x80, 0x55, 0x00]);

    let table = reader.read_table(1).unwrap();
    let metadata = table.schema();
    assert_eq!(
        metadata.columns[0].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[0].tform.element(), ElementKind::Bit);
    assert_eq!(
        metadata.columns[1].tform.storage(),
        Storage::Array(DescriptorWidth::Q)
    );
    assert_eq!(metadata.columns[1].tform.element(), ElementKind::Bit);
    let p = table.column(0).unwrap().vla_bits().unwrap();
    let q = table.column(1).unwrap().vla_bits().unwrap();
    for row in 0..3 {
        assert_eq!(p.row(row), p_rows[row].as_bitslice());
        assert_eq!(q.row(row), q_rows[row].as_bitslice());
    }
    // The decoded rows end at their descriptor counts, 0, 1 and 9 bits: a 1-bit row
    // and a 9-bit one, which share no byte count, stay apart.
    let decoded = table.column(0).unwrap().vla().unwrap();
    assert_eq!(decoded.ends(), [0, 1, 10]);
    assert_eq!(
        decoded,
        Ragged::from_rows(p_rows.iter().cloned().map(ColumnData::Bit)).unwrap()
    );
}

/// What a single column reads back as: its values, or its arrays.
#[derive(Debug)]
enum ReadBack {
    Values(ColumnData),
    Arrays(Ragged<ColumnData>),
}

/// A single column writes its keywords and reads back as it was written: scaled with a
/// null; three-state logical, its null kept (§7.3.3); and `u64` extremes at the exact offset
/// `2⁶³`, fixed and in `P` and `Q` arrays, each descriptor sized to its longest row.
#[test]
fn single_columns_round_trip_with_their_keywords() {
    let u64_rows = || {
        Ragged::from_rows([
            ColumnData::I64(vec![i64::MIN]),
            ColumnData::I64(vec![i64::MAX]),
        ])
        .unwrap()
    };
    let offset = || Value::Integer(FitsInteger::from(1u64 << 63));
    let cases = [
        (
            WriteColumn::fixed("X", ColumnData::I32(vec![5, 99]), 1)
                .with_scaling(2.0, 10.0)
                .with_null(99),
            vec![
                ("TSCAL1", 2.0.into()),
                ("TZERO1", 10.0.into()),
                ("TNULL1", 99.into()),
            ],
            ReadBack::Values(ColumnData::I32(vec![5, 99])),
        ),
        (
            WriteColumn::fixed("FLAG", ColumnData::Logical(vec![Some(true), None]), 1),
            vec![("TFORM1", "1L".into())],
            ReadBack::Values(ColumnData::Logical(vec![Some(true), None])),
        ),
        (
            WriteColumn::fixed("U64", ColumnData::I64(vec![i64::MIN, i64::MAX]), 1)
                .with_scaling(1.0, U64_OFFSET),
            vec![("TZERO1", offset())],
            ReadBack::Values(ColumnData::I64(vec![i64::MIN, i64::MAX])),
        ),
        (
            WriteColumn::vla("PU64", u64_rows(), DescriptorWidth::P).with_scaling(1.0, U64_OFFSET),
            vec![("TFORM1", "1PK(1)".into()), ("TZERO1", offset())],
            ReadBack::Arrays(u64_rows()),
        ),
        (
            WriteColumn::vla("QU64", u64_rows(), DescriptorWidth::Q).with_scaling(1.0, U64_OFFSET),
            vec![("TFORM1", "1QK(1)".into()), ("TZERO1", offset())],
            ReadBack::Arrays(u64_rows()),
        ),
    ];
    for (column, cards, expected) in cases {
        let file = memory::written(|w| w.write_table(&memory::binary_table(2, &[column]), None));
        let mut reader = file.stream();
        cards::assert_cards(&reader.hdus[1].header, &cards);
        let table = reader.read_table(1).unwrap();
        let column = table.column(0).unwrap();
        match &expected {
            ReadBack::Values(values) => assert_eq!(&column.raw().unwrap(), values),
            ReadBack::Arrays(arrays) => assert_eq!(&column.vla().unwrap(), arrays),
        }
    }
}

#[test]
fn binary_metadata_is_validated_for_every_fixed_and_vla_stored_type() {
    for (stored_type, data) in every_column_kind() {
        for column in fixed_and_vla(data) {
            let context = format!("{stored_type} {column:?}");
            if matches!(stored_type, 'A' | 'L' | 'X') {
                assert_table_column_refused(column.clone().with_scaling(2.0, 3.0), "TSCAL1");
            } else {
                let header = written_table_header(column.clone().with_scaling(2.0, 3.0));
                assert_eq!(header.get_real("TSCAL1").unwrap(), Some(2.0), "{context}");
                assert_eq!(header.get_real("TZERO1").unwrap(), Some(3.0), "{context}");
            }

            if matches!(stored_type, 'B' | 'I' | 'J' | 'K') {
                let header = written_table_header(column.with_null(0));
                assert_eq!(header.get_integer("TNULL1").unwrap(), Some(0), "{context}");
            } else {
                assert_table_column_refused(column.with_null(0), "TNULL1");
            }
        }
    }
}

/// A null takes exactly the values its stored integer type holds: a column's `TNULLn`,
/// fixed or variable-length, and an image's `BLANK`. A float image has no `BLANK`.
#[test]
fn nulls_are_range_checked_for_every_stored_type() {
    for bitpix in BITPIXES {
        let image = |blank| {
            // A literal: `Image::new` refuses this image, and the write's own check is
            // under test.
            Image {
                shape: vec![1],
                samples: samples::ramp(bitpix, 1),
                scaling: Scaling {
                    blank: Some(blank),
                    ..Scaling::IDENTITY
                },
            }
        };
        let blank_refused = |blank| {
            matches!(
                memory::refused_before_output(|w| w.write_image(image(blank).view(), None)),
                FitsError::InvalidKeywordValue {
                    keyword: keywords::BLANK,
                    ..
                }
            )
        };
        let Some(range) = samples::integer_range(bitpix) else {
            assert!(blank_refused(0), "{bitpix:?}");
            continue;
        };
        for null in [*range.start(), *range.end()] {
            for column in fixed_and_vla(integer_column_data(bitpix)) {
                let header = written_table_header(column.with_null(null));
                cards::assert_cards(&header, &[("TNULL1", null.into())]);
            }
            let file = memory::written(|w| w.write_image(image(null).view(), None));
            let mut reader = file.stream();
            cards::assert_cards(&reader.hdus[0].header, &[("BLANK", null.into())]);
            assert_eq!(memory::read_whole(&mut reader, 0).scaling.blank, Some(null));
        }
        for null in [range.start().checked_sub(1), range.end().checked_add(1)]
            .into_iter()
            .flatten()
        {
            for column in fixed_and_vla(integer_column_data(bitpix)) {
                assert_table_column_refused(column.with_null(null), "TNULL1");
            }
            assert!(blank_refused(null), "{bitpix:?} {null}");
        }
    }
}

#[test]
fn writes_and_reads_back_a_binary_table() {
    let columns = vec![
        WriteColumn::fixed("NOSTA", ColumnData::I32(vec![1, 2, 3]), 1),
        WriteColumn::fixed(
            "XYZ",
            ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
            3, // 3 floats per row
        )
        .with_unit("m"),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Character(b"AB CDEF  ".to_vec()),
            3, // 3-char field
        ),
    ];
    let file = memory::written(|w| w.write_table(&memory::binary_table(3, &columns), None));
    let mut r = file.stream();

    // A dataless primary is auto-written before the table extension.
    assert_eq!(r.hdus().len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[0].header.naxis().unwrap(), 0);
    assert_eq!(r.hdus[1].kind, HduKind::BinTable);

    let t = r.read_table(1).unwrap();
    let metadata = t.schema();
    assert_eq!(metadata.row_count(), 3);
    assert_eq!(metadata.columns.len(), 3);
    assert_eq!(metadata.columns[0].name().unwrap(), Some("NOSTA"));
    assert_eq!(metadata.columns[1].unit().unwrap(), Some("m"));
    assert_eq!(
        t.column(0).unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2, 3])
    );
    assert_eq!(
        t.column(1).unwrap().raw().unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
    );
    assert_eq!(
        t.column(2).unwrap().raw().unwrap(),
        ColumnData::Character(b"AB CDEF  ".to_vec())
    );
}

#[test]
fn binary_character_columns_round_trip_exactly_and_refuse_over_width() {
    let fields = b"AB  AB\0x\0xyz    ".to_vec();
    let vla_rows =
        Ragged::<ColumnData>::new(ColumnData::Character(fields.clone()), vec![4, 8, 12, 16]);
    let columns = [
        WriteColumn::fixed("FIXED", ColumnData::Character(fields.clone()), 4),
        WriteColumn::vla("PCHAR", vla_rows.clone(), DescriptorWidth::P),
        WriteColumn::vla("QCHAR", vla_rows.clone(), DescriptorWidth::Q),
    ];
    let bytes = memory::written(|w| w.write_table(&memory::binary_table(4, &columns), None));

    let mut reader = bytes.stream();
    cards::assert_cards(
        &reader.hdus[1].header,
        &[("TFORM2", "1PA(4)".into()), ("TFORM3", "1QA(4)".into())],
    );
    let table = reader.read_table(1).unwrap();
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        ColumnData::Character(fields)
    );
    assert_eq!(table.column(1).unwrap().vla().unwrap(), vla_rows);
    assert_eq!(table.column(2).unwrap().vla().unwrap(), vla_rows);
    let too_wide = WriteColumn::characters("BAD", [&b"ABCD"[..], b"ABCDE"], 4).unwrap_err();
    assert_matches!(
        too_wide,
        FitsError::CharacterFieldTooWide {
            column,
            row: 1,
            length: 5,
            width: 4
        } if column == "BAD"
    );
    // A bit row has no padding, so a shorter row is refused as a longer one is.
    for (row, bits) in [
        (0, bitvec![u8, Msb0; 1, 0]),
        (0, bitvec![u8, Msb0; 1, 0, 1, 1]),
    ] {
        assert_matches!(
            WriteColumn::bits("BITS", [bits.clone()], 3),
            Err(FitsError::BitFieldLengthMismatch { column, row: r, length, width: 3 })
                if column == "BITS" && r == row && length == bits.len()
        );
    }
    let padded = WriteColumn::characters("PAD", ["AB", "CDE", "F"], 3).unwrap();
    let file = memory::written(|w| w.write_table(&memory::binary_table(3, &[padded]), None));
    let mut reader = file.stream();
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::Character(b"AB CDEF  ".to_vec())
    );
}

#[test]
fn table_builders_infer_rows_and_refuse_cross_column_mismatches() {
    let mut binary = Builder::new();
    binary
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))
        .unwrap();
    binary
        .push(WriteColumn::fixed(
            "PAIR",
            ColumnData::I16(vec![10, 11, 20, 21, 30, 31]),
            2,
        ))
        .unwrap();
    assert_eq!(binary.row_count, Some(3));
    assert_matches!(
        binary.push(WriteColumn::scalar(
            "SHORT",
            ColumnData::F64(vec![1.0, 2.0]),
        )),
        Err(FitsError::TableRowCountMismatch {
            column,
            expected: 3,
            got: 2,
        }) if column == "SHORT"
    );

    let inferred_vla = WriteColumn::vla(
        "SAMPLES",
        Ragged::from_rows(vec![
            ColumnData::I16(vec![1, 2]),
            ColumnData::I16(Vec::new()),
            ColumnData::I16(vec![3]),
        ])
        .unwrap(),
        DescriptorWidth::P,
    );
    binary.push(inferred_vla).unwrap();

    // 7 I16 values in rows of 3 leave a row of 1; 3 bytes in 12-bit (2-byte) rows
    // leave a row of 1 byte; values in 0-element rows fit no row count; an empty
    // 0-element column fits any.
    for (name, column, count, row_width) in [
        (
            "F",
            WriteColumn::fixed("F", ColumnData::I16(vec![0; 7]), 3),
            7,
            3,
        ),
        (
            "Z",
            WriteColumn::fixed("Z", ColumnData::I16(vec![1, 2]), 0),
            2,
            0,
        ),
    ] {
        assert_matches!(
            Builder::with_rows(2).push(column).map(|_| ()),
            Err(FitsError::PartialTableRow { column, count: c, row_width: w })
                if column == name && c == count && w == row_width
        );
    }
    let mut empty = Builder::with_rows(2);
    empty
        .push(WriteColumn::fixed("Z", ColumnData::I16(Vec::new()), 0))
        .unwrap();
    assert_eq!(empty.row_count, Some(2));
    Builder::with_rows(0)
        .push(WriteColumn::vla(
            "EMPTY",
            Ragged::<ColumnData>::new(ColumnData::I64(Vec::new()), Vec::new()),
            DescriptorWidth::P,
        ))
        .unwrap();

    let mut ascii = AsciiBuilder::new();
    ascii
        .push(AsciiWriteColumn::text(
            "NAME",
            [Some("A"), Some("B")].into_iter().collect(),
            2,
        ))
        .unwrap()
        .push(
            AsciiWriteColumn::float(
                "VALUE",
                vec![Some(1.25), None],
                AsciiFloatFormat::F {
                    width: 8,
                    decimals: 2,
                },
            )
            .with_null("NULL"),
        )
        .unwrap();
    let file = memory::written(|w| w.write_ascii_table(&ascii, None));
    let reader = file.stream();
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::NAXIS.indexed(2).unwrap())
            .unwrap(),
        Some(2)
    );
}
