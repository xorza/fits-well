use crate::data::U64_OFFSET;
use crate::data::unsigned_data::UnsignedData;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::table_impl::BinTable;
use crate::table_impl::character_field::CharacterField;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::internals::table_header;
use num_complex::Complex;

#[test]
fn decodes_fixed_width_columns_from_hand_built_data() {
    // 1J (i32) | 2E (two f32) | 3A (string)  →  row width 4 + 8 + 3 = 15.
    let header = table_header(15, 2, &["1J", "2E", "3A"]);
    let mut data = Vec::new();
    for (j, e0, e1, text) in [(1i32, 1.0f32, 2.0f32, b"ABC"), (2, 3.0, 4.0, b"DE ")] {
        data.extend_from_slice(&j.to_be_bytes());
        data.extend_from_slice(&e0.to_be_bytes());
        data.extend_from_slice(&e1.to_be_bytes());
        data.extend_from_slice(text);
    }

    let table = BinTable::from_data(&header, data).unwrap();
    let mut metadata = table.metadata();
    assert_eq!(metadata.nrows, 2);
    assert_eq!(
        metadata
            .columns
            .iter()
            .map(|c| c.byte_offset)
            .collect::<Vec<_>>(),
        vec![0, 4, 12]
    );
    metadata.nrows = usize::MAX;
    metadata.columns = &[];
    assert_eq!(metadata.nrows, usize::MAX);
    assert!(metadata.columns.is_empty());
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2])
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().raw().unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(
        table.column_by_idx(2).unwrap().raw().unwrap(),
        ColumnData::Character(vec![
            CharacterField::new(b"ABC".to_vec()),
            CharacterField::new(b"DE ".to_vec())
        ])
    );
}

#[test]
fn zero_repeat_column_decodes_to_empty() {
    let header = table_header(4, 1, &["0D", "0PJ", "0QD", "0PX", "1J"]);
    let data = 7i32.to_be_bytes().to_vec();
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::F64(vec![])
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().vla().unwrap(),
        vec![ColumnData::I32(Vec::new())]
    );
    assert_eq!(
        table.column_by_idx(2).unwrap().vla().unwrap(),
        vec![ColumnData::F64(Vec::new())]
    );
    assert!(table.column_by_idx(3).unwrap().vla_bits().unwrap()[0].is_empty());
    assert_eq!(
        table.column_by_idx(4).unwrap().raw().unwrap(),
        ColumnData::I32(vec![7])
    );
}

#[test]
fn read_column_physical_applies_tscal_tzero_and_tnull() {
    let mut header = table_header(2, 3, &["1I"]); // i16 column
    header
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", 5);
    let mut data = Vec::new();
    for x in [3i16, 5, 7] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&header, data).unwrap();
    let phys = table.column_by_idx(0).unwrap().physical().unwrap();
    // 3 → 10 + 2·3 = 16 ; 5 == TNULL → NaN ; 7 → 10 + 2·7 = 24
    assert_eq!(phys[0], 16.0);
    assert!(phys[1].is_nan());
    assert_eq!(phys[2], 24.0);

    for (keyword, expected) in [
        ("TSCAL1", "real"),
        ("TZERO1", "real"),
        ("TNULL1", "integer"),
    ] {
        let mut malformed = header.clone();
        malformed.set_internal(keyword, "not numeric");
        assert!(matches!(
            BinTable::from_data(&malformed, vec![0; 6]),
            Err(FitsError::TypeMismatch { name, expected: actual })
                if name == keyword && actual == expected
        ));
    }
}

#[test]
fn read_column_physical_rejects_non_numeric_columns() {
    let header = table_header(3, 1, &["3A"]);
    let table = BinTable::from_data(&header, b"abc".to_vec()).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().physical(),
        Err(FitsError::NonNumericColumn { code: 'A' })
    ));
}

#[test]
fn read_column_on_a_vla_directs_to_read_vla_column() {
    let header = table_header(8, 1, &["1PE(3)"]);
    let table = BinTable::from_data(&header, vec![0u8; 8]).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().raw(),
        Err(FitsError::VariableLengthColumn { code: 'P' })
    ));
}

#[test]
fn decodes_variable_length_arrays_from_the_heap() {
    // One `PE` column (f32 heap arrays), two rows of different lengths.
    // Main table = two 8-byte `P` descriptors; the heap follows at THEAP
    // (default = main size = 16).
    let mut header = table_header(8, 2, &["1PE(3)"]);
    header.set_internal("PCOUNT", 12); // heap = 3 × f32
    let mut data = Vec::new();
    // descriptors: row 0 → (nelem 2, offset 0), row 1 → (nelem 1, offset 8)
    for (nelem, offset) in [(2i32, 0i32), (1, 8)] {
        data.extend_from_slice(&nelem.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
    }
    // heap: [1.0, 2.0] then [3.0]
    for x in [1.0f32, 2.0, 3.0] {
        data.extend_from_slice(&x.to_be_bytes());
    }

    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().vla().unwrap(),
        vec![ColumnData::F32(vec![1.0, 2.0]), ColumnData::F32(vec![3.0]),]
    );
}

#[test]
fn preserves_tdisp_without_making_it_part_of_data_decoding() {
    let mut header = table_header(4, 1, &["1J"]);
    for display in ["I5", "Q5", "E12.5E3junk"] {
        header.set_internal("TDISP1", display);
        let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
        assert_eq!(table.schema.columns[0].tdisp.as_deref(), Some(display));
        assert_eq!(
            table.column_by_idx(0).unwrap().raw().unwrap(),
            ColumnData::I32(vec![0])
        );
    }
}

#[test]
fn read_column_complex_widens_and_scales() {
    let mut header = table_header(24, 1, &["1C", "1M"]);
    header
        .set_internal("TSCAL1", 3.0)
        .set_internal("TZERO1", 4.0)
        .set_internal("TSCAL2", 3.0)
        .set_internal("TZERO2", 4.0);
    let mut data = Vec::new();
    data.extend_from_slice(&1.0f32.to_be_bytes());
    data.extend_from_slice(&2.0f32.to_be_bytes());
    data.extend_from_slice(&1.0f64.to_be_bytes());
    data.extend_from_slice(&2.0f64.to_be_bytes());
    let table = BinTable::from_data(&header, data).unwrap();
    // (4 + 3·1) + (3·2)i = 7 + 6i; TZERO has no imaginary component.
    assert_eq!(
        table.column_by_idx(0).unwrap().complex().unwrap(),
        vec![Complex { re: 7.0, im: 6.0 }]
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().complex().unwrap(),
        vec![Complex { re: 7.0, im: 6.0 }]
    );
    // A non-complex column errors.
    let h2 = table_header(4, 1, &["1J"]);
    let t2 = BinTable::from_data(&h2, vec![0u8; 4]).unwrap();
    assert!(matches!(
        t2.column_by_idx(0).unwrap().complex(),
        Err(FitsError::NotAComplexColumn { code: 'J' })
    ));
}

#[test]
fn read_column_unsigned_recovers_typed_values() {
    // `1I` with TZERO=2¹⁵ → u16; `1B` with TZERO=-128 → i8.
    let mut header = table_header(3, 1, &["1I", "1B"]);
    header
        .set_internal("TZERO1", 32768.0)
        .set_internal("TZERO2", -128.0);
    let mut data = Vec::new();
    data.extend_from_slice(&((50000u16 ^ 0x8000) as i16).to_be_bytes());
    data.push(((-10i8) as u8) ^ 0x80);
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().unsigned().unwrap(),
        Some(UnsignedData::U16(vec![50000]))
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().unsigned().unwrap(),
        Some(UnsignedData::I8(vec![-10]))
    );
}

#[test]
fn read_column_unsigned_is_exact_for_u64_and_none_otherwise() {
    // `1K` with TZERO=2⁶³ → u64, exact past 2⁵³; a plain `1J` (TZERO=0) is not
    // an unsigned column.
    let mut header = table_header(12, 1, &["1K", "1J"]);
    header.set_internal("TZERO1", 9_223_372_036_854_775_808.0); // 2⁶³
    let mut data = Vec::new();
    data.extend_from_slice(&((u64::MAX ^ 0x8000_0000_0000_0000) as i64).to_be_bytes());
    data.extend_from_slice(&7i32.to_be_bytes());
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().unsigned().unwrap(),
        Some(UnsignedData::U64(vec![u64::MAX]))
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().unsigned().unwrap(),
        None // TZERO=0
    );
}

#[test]
fn read_vla_column_physical_scales_heap_arrays_and_nulls() {
    // 1PJ column, TSCAL=2, TZERO=10, TNULL=99. Row 0 = [5, 99(null)], row 1 = [3].
    let mut header = table_header(8, 2, &["1PJ(2)"]);
    header
        .set_internal("PCOUNT", 12)
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", 99);
    let mut data = Vec::new();
    for (nelem, offset) in [(2i32, 0i32), (1, 8)] {
        data.extend_from_slice(&nelem.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
    }
    for x in [5i32, 99, 3] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&header, data).unwrap();
    let phys = table.column_by_idx(0).unwrap().vla_physical().unwrap();
    assert_eq!(phys[0][0], 20.0); // 10 + 2·5
    assert!(phys[0][1].is_nan()); // 99 == TNULL
    assert_eq!(phys[1], vec![16.0]); // 10 + 2·3
}

#[test]
fn read_vla_complex_scales_p_and_q_heap_values() {
    let mut header = table_header(24, 2, &["1PC(2)", "1QM(1)"]);
    header
        .set_internal("PCOUNT", 32)
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TSCAL2", -0.5)
        .set_internal("TZERO2", 3.0);
    let mut data = Vec::new();
    data.extend_from_slice(&2i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&1i64.to_be_bytes());
    data.extend_from_slice(&16i64.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&0i64.to_be_bytes());
    data.extend_from_slice(&0i64.to_be_bytes());
    for (re, im) in [(1.0f32, 2.0f32), (-3.0, 4.0)] {
        data.extend_from_slice(&re.to_be_bytes());
        data.extend_from_slice(&im.to_be_bytes());
    }
    data.extend_from_slice(&6.0f64.to_be_bytes());
    data.extend_from_slice(&(-8.0f64).to_be_bytes());

    let table = BinTable::from_data(&header, data).unwrap();
    // PC: 10 + 2·(1 + 2i) = 12 + 4i; QM: 3 - 0.5·(6 - 8i) = 0 + 4i.
    assert_eq!(
        table.column_by_idx(0).unwrap().vla_complex().unwrap(),
        vec![
            vec![Complex { re: 12.0, im: 4.0 }, Complex { re: 4.0, im: 8.0 }],
            vec![],
        ]
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().vla_complex().unwrap(),
        vec![vec![Complex { re: 0.0, im: 4.0 }], vec![]]
    );
}

#[test]
fn read_vla_unsigned_is_exact_past_f64_integer_precision() {
    let mut header = table_header(24, 1, &["1PK(3)", "1QK(3)"]);
    header
        .set_internal("PCOUNT", 48)
        .set_internal("TZERO1", U64_OFFSET)
        .set_internal("TZERO2", U64_OFFSET);
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&3i64.to_be_bytes());
    data.extend_from_slice(&24i64.to_be_bytes());
    let expected = [0, 9_007_199_254_740_993, u64::MAX];
    // Stored = physical - 2^63, computed independently of the sign-flip decoder.
    let stored = [i64::MIN, -9_214_364_837_600_034_815, i64::MAX];
    for _ in 0..2 {
        for value in stored {
            data.extend_from_slice(&value.to_be_bytes());
        }
    }

    let table = BinTable::from_data(&header, data.clone()).unwrap();
    let exact = Some(vec![UnsignedData::U64(expected.to_vec())]);
    assert_eq!(
        table.column_by_idx(0).unwrap().vla_unsigned().unwrap(),
        exact
    );
    assert_eq!(
        table.column_by_idx(1).unwrap().vla_unsigned().unwrap(),
        exact
    );
    assert_eq!(
        table.column_by_idx(0).unwrap().vla_physical().unwrap()[0][1],
        9_007_199_254_740_992.0
    );

    header
        .set_internal("TSCAL1", 2.0)
        .set_internal("TNULL2", i64::MIN);
    let non_convention = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        non_convention
            .column_by_idx(0)
            .unwrap()
            .vla_unsigned()
            .unwrap(),
        None
    );
    assert_eq!(
        non_convention
            .column_by_idx(1)
            .unwrap()
            .vla_unsigned()
            .unwrap(),
        None
    );
}

#[test]
fn vla_descriptor_overrunning_the_heap_is_rejected() {
    // §6.6: a span must lie within the heap (`PCOUNT` bytes), not the block fill.
    // Heap is 8 bytes (PCOUNT=8) but the descriptor claims 3 f32 = 12 bytes.
    let mut header = table_header(8, 1, &["1PE(3)"]);
    header.set_internal("PCOUNT", 8);
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes()); // nelem = 3
    data.extend_from_slice(&0i32.to_be_bytes()); // offset = 0
    data.extend_from_slice(&[0u8; 8]); // only 8 heap bytes (then block fill)
    data.resize(2880, 0); // block-padded fill that must NOT be read as heap
    let table = BinTable::from_data(&header, data).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().vla(),
        Err(FitsError::UnexpectedEof)
    ));
}

#[test]
fn read_column_by_name_and_one_step_physical() {
    let mut header = table_header(2, 3, &["1I"]); // one i16 column
    header
        .set_internal("TTYPE1", "FLUX")
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0);
    let mut data = Vec::new();
    for x in [1i16, 2, 3] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&header, data).unwrap();
    // Raw, by name (case-insensitive).
    assert_eq!(
        table.column_by_name("flux").unwrap().raw().unwrap(),
        ColumnData::I16(vec![1, 2, 3])
    );
    // Physical in one call: 10 + 2·x — by index and by name.
    assert_eq!(
        table.column_by_idx(0).unwrap().physical().unwrap(),
        vec![12.0, 14.0, 16.0]
    );
    assert_eq!(
        table.column_by_name("FLUX").unwrap().physical().unwrap(),
        vec![12.0, 14.0, 16.0]
    );
    // A missing name is a clean error.
    assert!(matches!(
        table.column_by_name("nope"),
        Err(FitsError::ColumnNotFound { .. })
    ));
}

#[test]
fn read_vla_on_a_fixed_column_is_an_error() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.column_by_idx(0).unwrap().vla(),
        Err(FitsError::NotAVla { code: 'J' })
    ));
}

#[test]
fn out_of_bounds_column_is_an_error() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.column_by_idx(9),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Column,
            index: 9,
            len: 1
        })
    ));
}
