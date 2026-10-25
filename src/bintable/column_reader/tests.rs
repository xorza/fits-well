use crate::bintable::Table;
use crate::bintable::column::Column;
use crate::bintable::column_data::ColumnData;
use crate::bintable::column_reader::ColumnReader;
use crate::bintable::internals::table_header;
use crate::fits_error::ExpectedColumn;
use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::fits_error::ValueKind;
use crate::header_model::keywords;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::{I8_OFFSET, U16_OFFSET, U64_OFFSET};
use crate::internals::raw_table::RawTable;
use crate::internals::samples;
use crate::internals::whole_column::WholeColumn;
use std::assert_matches;

use crate::ragged::Ragged;
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

    let table = Table::from_data(&header, data).unwrap();
    let schema = table.schema();
    assert_eq!(schema.row_count(), 2);
    assert_eq!(
        schema
            .columns
            .iter()
            .map(Column::byte_offset)
            .collect::<Vec<_>>(),
        vec![0, 4, 12]
    );
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2])
    );
    assert_eq!(
        table.column(1).unwrap().raw().unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(
        table.column(2).unwrap().raw().unwrap(),
        ColumnData::Character(b"ABCDE ".to_vec())
    );
}

#[test]
fn zero_repeat_column_decodes_to_empty() {
    let header = table_header(4, 1, &["0D", "0PJ", "0QD", "0PX", "1J"]);
    let data = 7i32.to_be_bytes().to_vec();
    let table = Table::from_data(&header, data).unwrap();
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        ColumnData::F64(vec![])
    );
    assert_eq!(
        table.column(1).unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(Vec::new()), vec![0])
    );
    assert_eq!(
        table.column(2).unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::F64(Vec::new()), vec![0])
    );
    assert!(table.column(3).unwrap().vla_bits().unwrap()[0].is_empty());
    assert_eq!(
        table.column(4).unwrap().raw().unwrap(),
        ColumnData::I32(vec![7])
    );
}

#[test]
fn physical_applies_tscal_tzero_and_tnull() {
    let mut header = table_header(2, 3, &["1I"]); // i16 column
    header
        .set_internal("TTYPE1", "FLUX")
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", 5);
    let mut data = Vec::new();
    for x in [3i16, 5, 7] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = Table::from_data(&header, data).unwrap();
    // 3 → 10 + 2·3 = 16 ; 5 == TNULL → NaN ; 7 → 10 + 2·7 = 24, by index and by a
    // name in any case.
    for column in [table.column(0), table.column("flux")] {
        let phys = column.unwrap().physical().unwrap();
        assert_eq!(
            samples::nan_blind_of(&phys),
            samples::nan_blind_of(&[16.0, f64::NAN, 24.0])
        );
    }
    // An `f32` buffer takes the narrowed values after what it holds.
    let mut narrow: Vec<f32> = vec![1.5];
    table.column(0).unwrap().physical_into(&mut narrow).unwrap();
    assert_eq!(
        samples::nan_blind_of(&narrow),
        samples::nan_blind_of(&[1.5f32, 16.0, f32::NAN, 24.0])
    );
    assert_eq!(
        table.column("FLUX").unwrap().raw().unwrap(),
        ColumnData::I16(vec![3, 5, 7])
    );
    assert_matches!(table.column("nope"), Err(FitsError::ColumnNotFound { name }) if name == "nope");

    // A bad optional keyword stays on its column: the reads that need it fail and
    // name it, and the stored values and every other column read as they are.
    let mut two = table_header(4, 1, &["1I", "1I"]);
    two.set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", 5);
    for (keyword, expected) in [
        ("TSCAL1", ValueKind::Real),
        ("TZERO1", ValueKind::Real),
        ("TNULL1", ValueKind::Integer),
    ] {
        let mut malformed = two.clone();
        malformed.set_internal(keyword, "not numeric");
        let table = Table::from_data(&malformed, vec![0, 3, 0, 4]).unwrap();
        let defect = |error: FitsError| {
            matches!(
                error,
                FitsError::InvalidColumnKeyword { keyword: name, source }
                    if name == keyword
                        && matches!(*source, FitsError::ValueTypeMismatch { expected: actual, .. }
                            if actual == expected)
            )
        };
        let column = table.column(0).unwrap();
        assert!(defect(column.physical().unwrap_err()), "{keyword}");
        assert!(defect(column.unsigned().unwrap_err()), "{keyword}");
        assert_eq!(column.raw().unwrap(), ColumnData::I16(vec![3]), "{keyword}");
        assert_eq!(
            table.column(1).unwrap().physical().unwrap(),
            [4.0],
            "{keyword}"
        );
    }
}

#[test]
fn decodes_variable_length_arrays_from_the_heap() {
    // One `PE` column (f32 heap arrays), three rows of different lengths, the third
    // overlapping the first two (§7.3.5). Main table = three 8-byte `P` descriptors; the
    // heap follows at THEAP (default = main size = 24).
    let mut header = table_header(8, 3, &["1PE(3)"]);
    header.set_internal(keywords::PCOUNT, 12); // heap = 3 × f32
    let mut data = Vec::new();
    // descriptors: row 0 → (nelem 2, offset 0), row 1 → (nelem 1, offset 8), row 2 →
    // (nelem 3, offset 0)
    for (nelem, offset) in [(2i32, 0i32), (1, 8), (3, 0)] {
        data.extend_from_slice(&nelem.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
    }
    // heap: [1.0, 2.0] then [3.0]
    for x in [1.0f32, 2.0, 3.0] {
        data.extend_from_slice(&x.to_be_bytes());
    }

    let table = Table::from_data(&header, data).unwrap();
    let vla = table.column(0).unwrap().vla().unwrap();
    assert_eq!(
        vla,
        Ragged::<ColumnData>::new(
            ColumnData::F32(vec![1.0, 2.0, 3.0, 1.0, 2.0, 3.0]),
            vec![2, 3, 6]
        )
    );
    // The descriptors' counts, 2 + 1 + 3, are reserved once and exactly, before the decode.
    let ColumnData::F32(values) = vla.into_values() else {
        panic!("an E column decodes to f32");
    };
    assert_eq!(values.capacity(), 6);
}

#[test]
fn complex_widens_and_scales() {
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
    let table = Table::from_data(&header, data).unwrap();
    // (4 + 3·1) + (3·2)i = 7 + 6i; TZERO has no imaginary component.
    assert_eq!(
        table.column(0).unwrap().complex().unwrap(),
        vec![Complex { re: 7.0, im: 6.0 }]
    );
    assert_eq!(
        table.column(1).unwrap().complex().unwrap(),
        vec![Complex { re: 7.0, im: 6.0 }]
    );
}

#[test]
fn unsigned_recovers_typed_values() {
    // `1I` with TZERO=2¹⁵ → u16; `1B` with TZERO=-128 → i8.
    let mut header = table_header(3, 1, &["1I", "1B"]);
    header
        .set_internal("TZERO1", U16_OFFSET)
        .set_internal("TZERO2", I8_OFFSET);
    let mut data = Vec::new();
    data.extend_from_slice(&17_232i16.to_be_bytes()); // 50 000 − 2¹⁵
    data.push(118); // −10 + 2⁷
    let table = Table::from_data(&header, data).unwrap();
    assert_eq!(
        table.column(0).unwrap().unsigned().unwrap(),
        Some(UnsignedData::U16(vec![50000]))
    );
    assert_eq!(
        table.column(1).unwrap().unsigned().unwrap(),
        Some(UnsignedData::I8(vec![-10]))
    );
}

#[test]
fn unsigned_is_exact_for_u64_and_none_otherwise() {
    // `1K` with TZERO=2⁶³ → u64, exact past 2⁵³; a plain `1J` (TZERO=0) is not
    // an unsigned column.
    let mut header = table_header(12, 1, &["1K", "1J"]);
    header.set_internal("TZERO1", U64_OFFSET);
    let mut data = Vec::new();
    data.extend_from_slice(
        &(u64::MAX ^ 0x8000_0000_0000_0000)
            .cast_signed()
            .to_be_bytes(),
    );
    data.extend_from_slice(&7i32.to_be_bytes());
    let table = Table::from_data(&header, data).unwrap();
    assert_eq!(
        table.column(0).unwrap().unsigned().unwrap(),
        Some(UnsignedData::U64(vec![u64::MAX]))
    );
    assert_eq!(
        table.column(1).unwrap().unsigned().unwrap(),
        None // TZERO=0
    );
    // A buffer of another kind takes the column's; a column that is not unsigned leaves
    // it as it was.
    let mut reused = UnsignedData::U16(vec![9, 9]);
    assert!(table.column(0).unwrap().unsigned_into(&mut reused).unwrap());
    assert_eq!(reused, UnsignedData::U64(vec![u64::MAX]));
    assert!(!table.column(1).unwrap().unsigned_into(&mut reused).unwrap());
    assert_eq!(reused, UnsignedData::U64(vec![u64::MAX]));
}

#[test]
fn vla_physical_scales_heap_arrays_and_nulls() {
    // 1PJ column, TSCAL=2, TZERO=10, TNULL=99. Row 0 = [5, 99(null)], row 1 = [3].
    let raw = RawTable::new(2).heap(
        None,
        'J',
        &[
            &samples::be(&[5i32, 99], i32::to_be_bytes),
            &samples::be(&[3i32], i32::to_be_bytes),
        ],
    );
    let mut header = raw.header();
    header
        .set_internal("TSCAL1", 2.0)
        .set_internal("TZERO1", 10.0)
        .set_internal("TNULL1", 99);
    let table = Table::from_data(&header, raw.data()).unwrap();
    let phys = table.column(0).unwrap().vla_physical().unwrap();
    assert_eq!(phys.ends(), [2, 3]);
    // 10 + 2·5, then 99, the TNULL.
    assert_eq!(
        samples::nan_blind_of(phys.row(0)),
        samples::nan_blind_of(&[20.0, f64::NAN])
    );
    assert_eq!(phys.row(1), [16.0]); // 10 + 2·3
    // An `f32` read refills the buffer it is given, rows and all.
    let mut narrow = Ragged::<Vec<f32>>::new(vec![7.0; 5], vec![5]);
    table
        .column(0)
        .unwrap()
        .vla_physical_into(&mut narrow)
        .unwrap();
    assert_eq!(narrow.ends(), [2, 3]);
    assert_eq!(
        samples::nan_blind_of(narrow.row(0)),
        samples::nan_blind_of(&[20.0f32, f32::NAN])
    );
    assert_eq!(narrow.row(1), [16.0f32]);
}

#[test]
fn vla_complex_scales_p_and_q_heap_values() {
    let mut header = table_header(24, 2, &["1PC(2)", "1QM(1)"]);
    header
        .set_internal(keywords::PCOUNT, 32)
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

    let table = Table::from_data(&header, data).unwrap();
    // PC: 10 + 2·(1 + 2i) = 12 + 4i; QM: 3 - 0.5·(6 - 8i) = 0 + 4i.
    assert_eq!(
        table.column(0).unwrap().vla_complex().unwrap(),
        Ragged::<Vec<_>>::new(
            vec![Complex { re: 12.0, im: 4.0 }, Complex { re: 4.0, im: 8.0 }],
            vec![2, 2]
        )
    );
    assert_eq!(
        table.column(1).unwrap().vla_complex().unwrap(),
        Ragged::<Vec<_>>::new(vec![Complex { re: 0.0, im: 4.0 }], vec![1, 1])
    );
}

#[test]
fn vla_unsigned_is_exact_past_f64_integer_precision() {
    let mut header = table_header(24, 1, &["1PK(3)", "1QK(3)"]);
    header
        .set_internal(keywords::PCOUNT, 48)
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

    let table = Table::from_data(&header, data.clone()).unwrap();
    let exact = Some(Ragged::<UnsignedData>::new(
        UnsignedData::U64(expected.to_vec()),
        vec![3],
    ));
    assert_eq!(table.column(0).unwrap().vla_unsigned().unwrap(), exact);
    assert_eq!(table.column(1).unwrap().vla_unsigned().unwrap(), exact);
    assert_eq!(
        table.column(0).unwrap().vla_physical().unwrap().row(0)[1],
        9_007_199_254_740_992.0
    );

    header
        .set_internal("TSCAL1", 2.0)
        .set_internal("TNULL2", i64::MIN);
    let non_convention = Table::from_data(&header, data).unwrap();
    assert_eq!(
        non_convention.column(0).unwrap().vla_unsigned().unwrap(),
        None
    );
    assert_eq!(
        non_convention.column(1).unwrap().vla_unsigned().unwrap(),
        None
    );
}

#[test]
fn vla_descriptor_overrunning_the_heap_is_refused() {
    // §6.6: a span must lie within the heap (`PCOUNT` bytes), not the block fill.
    // Heap is 8 bytes (PCOUNT=8) but the descriptor claims 3 f32 = 12 bytes.
    let mut header = table_header(8, 1, &["1PE(3)"]);
    header.set_internal(keywords::PCOUNT, 8);
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes()); // nelem = 3
    data.extend_from_slice(&0i32.to_be_bytes()); // offset = 0
    data.extend_from_slice(&[0u8; 8]); // only 8 heap bytes (then block fill)
    data.resize(2880, 0); // block-padded fill that must NOT be read as heap
    let table = Table::from_data(&header, data).unwrap();
    assert_matches!(
        table.column(0).unwrap().vla(),
        Err(FitsError::DescriptorPastHeap { column: 0, row: 0 })
    );
}

#[test]
fn out_of_bounds_column_is_refused() {
    let header = table_header(4, 1, &["1J"]);
    let table = Table::from_data(&header, vec![0u8; 4]).unwrap();
    assert_matches!(
        table.column(9),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Column,
            index: 9,
            len: 1
        })
    );
}

/// Each accessor takes the column kinds it reads and refuses the others, naming the kind
/// it needs: a fixed read refuses an array, a physical read a text, complex or bit column,
/// and an array read a fixed column. `unsigned` gives `None` for a column of no unsigned
/// convention, and a bit array has no physical value.
#[test]
fn each_accessor_refuses_the_column_kinds_it_cannot_read() {
    type Read = fn(&ColumnReader<'_>) -> Result<(), FitsError>;
    let header = table_header(36, 1, &["1J", "1E", "3A", "1C", "5X", "1PE", "1PX"]);
    let table = Table::from_data(&header, vec![0; 36]).unwrap();
    let ok = None;
    let [fixed, array, bits, complex, numeric] = [
        ExpectedColumn::Fixed,
        ExpectedColumn::Array,
        ExpectedColumn::Bits,
        ExpectedColumn::Complex,
        ExpectedColumn::Numeric,
    ]
    .map(Some);
    let reads: [(&str, Read, [Option<ExpectedColumn>; 7]); 8] = [
        (
            "raw",
            |c| c.raw().map(drop),
            [ok, ok, ok, ok, ok, fixed, fixed],
        ),
        (
            "physical",
            |c| c.physical().map(drop),
            [ok, ok, numeric, numeric, numeric, fixed, fixed],
        ),
        (
            "unsigned",
            |c| c.unsigned().map(drop),
            [ok, ok, ok, ok, ok, fixed, fixed],
        ),
        (
            "complex",
            |c| c.complex().map(drop),
            [complex, complex, complex, ok, complex, fixed, fixed],
        ),
        (
            "bits",
            |c| c.bits().map(drop),
            [bits, bits, bits, bits, ok, bits, bits],
        ),
        (
            "vla",
            |c| c.vla().map(drop),
            [array, array, array, array, array, ok, ok],
        ),
        (
            "vla_physical",
            |c| c.vla_physical().map(drop),
            [array, array, array, array, array, ok, numeric],
        ),
        (
            "vla_bits",
            |c| c.vla_bits().map(drop),
            [bits, bits, bits, bits, bits, bits, ok],
        ),
    ];
    for (name, read, expected) in reads {
        for (index, expected) in expected.into_iter().enumerate() {
            let result = read(&table.column(index).unwrap());
            match expected {
                None => result.unwrap_or_else(|error| panic!("{name} of column {index}: {error}")),
                Some(kind) => assert_matches!(
                    result,
                    Err(FitsError::ColumnKindMismatch { column, expected }) if column == index && expected == kind,
                    "{name} of column {index}"
                ),
            }
        }
    }
}
