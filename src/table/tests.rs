use super::*;
use crate::reader::FitsReader;
use std::fs::File;

fn table_header(naxis1: usize, naxis2: usize, tforms: &[&str]) -> Header {
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", naxis1 as i64)
        .set("NAXIS2", naxis2 as i64)
        .set("PCOUNT", 0)
        .set("GCOUNT", 1)
        .set("TFIELDS", tforms.len() as i64);
    for (i, tform) in tforms.iter().enumerate() {
        h.set(&format!("TFORM{}", i + 1), *tform);
    }
    h
}

fn tform(repeat: usize, kind: TformKind, vla_elem: Option<TformKind>) -> Tform {
    Tform {
        repeat,
        kind,
        vla_elem,
    }
}

#[test]
fn parses_tform_repeat_and_kind() {
    let cases = [
        ("8A", tform(8, TformKind::Char, None)),
        ("3D", tform(3, TformKind::F64, None)),
        ("0D", tform(0, TformKind::F64, None)),
        ("1J", tform(1, TformKind::I32, None)),
        ("E", tform(1, TformKind::F32, None)), // bare code ⇒ repeat 1
        ("16X", tform(16, TformKind::Bit, None)),
        // P/Q carry the heap element type.
        (
            "1PE(5)",
            tform(1, TformKind::ArrayDesc32, Some(TformKind::F32)),
        ),
        (
            "1QD",
            tform(1, TformKind::ArrayDesc64, Some(TformKind::F64)),
        ),
    ];
    for (s, expected) in cases {
        assert_eq!(Tform::parse(s).unwrap(), expected, "{s}");
    }
    for bad in ["9Z", "", "1P", "2PE(5)", "3QD"] {
        // "1P" lacks the heap element-type letter; "2PE"/"3QD" violate the §6.3
        // rule that a P/Q descriptor's repeat count is 0 or 1.
        assert!(
            matches!(Tform::parse(bad), Err(FitsError::InvalidTform { .. })),
            "{bad}"
        );
    }
}

#[test]
fn theap_below_the_main_table_is_rejected() {
    // §6.6: the heap follows the main table, so THEAP < NAXIS1·NAXIS2 is invalid.
    let mut header = table_header(4, 2, &["1J"]); // main table = 8 bytes
    header.set("PCOUNT", 4).set("THEAP", 4); // THEAP 4 < 8
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 12]),
        Err(FitsError::WrongValueType { name: "THEAP" })
    ));
}

#[test]
fn byte_width_handles_arrays_bits_and_descriptors() {
    assert_eq!(Tform::parse("8A").unwrap().byte_width(), 8);
    assert_eq!(Tform::parse("3D").unwrap().byte_width(), 24);
    assert_eq!(Tform::parse("0D").unwrap().byte_width(), 0);
    assert_eq!(Tform::parse("16X").unwrap().byte_width(), 2); // 16 bits = 2 bytes
    assert_eq!(Tform::parse("9X").unwrap().byte_width(), 2); //  9 bits = 2 bytes
    assert_eq!(Tform::parse("1PB").unwrap().byte_width(), 8); // 32-bit descriptor
    assert_eq!(Tform::parse("1QB").unwrap().byte_width(), 16); // 64-bit descriptor
}

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
    assert_eq!(table.nrows, 2);
    assert_eq!(
        table
            .columns
            .iter()
            .map(|c| c.byte_offset)
            .collect::<Vec<_>>(),
        vec![0, 4, 12]
    );
    assert_eq!(table.read_column(0).unwrap(), ColumnData::I32(vec![1, 2]));
    assert_eq!(
        table.read_column(1).unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0])
    );
    assert_eq!(
        table.read_column(2).unwrap(),
        ColumnData::Text(vec!["ABC".into(), "DE".into()]) // trailing space trimmed
    );
}

#[test]
fn zero_repeat_column_decodes_to_empty() {
    let header = table_header(4, 1, &["0D", "1J"]);
    let data = 7i32.to_be_bytes().to_vec();
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(table.read_column(0).unwrap(), ColumnData::F64(vec![]));
    assert_eq!(table.read_column(1).unwrap(), ColumnData::I32(vec![7]));
}

#[test]
fn read_column_physical_applies_tscal_tzero_and_tnull() {
    let mut header = table_header(2, 3, &["1I"]); // i16 column
    header
        .set("TSCAL1", 2.0)
        .set("TZERO1", 10.0)
        .set("TNULL1", 5);
    let mut data = Vec::new();
    for x in [3i16, 5, 7] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&header, data).unwrap();
    let phys = table.read_column_physical(0).unwrap();
    // 3 → 10 + 2·3 = 16 ; 5 == TNULL → NaN ; 7 → 10 + 2·7 = 24
    assert_eq!(phys[0], 16.0);
    assert!(phys[1].is_nan());
    assert_eq!(phys[2], 24.0);
}

#[test]
fn read_column_physical_rejects_non_numeric_columns() {
    let header = table_header(3, 1, &["3A"]);
    let table = BinTable::from_data(&header, b"abc".to_vec()).unwrap();
    assert!(matches!(
        table.read_column_physical(0),
        Err(FitsError::NonNumericColumn { code: 'A' })
    ));
}

#[test]
fn read_column_on_a_vla_directs_to_read_vla_column() {
    let header = table_header(8, 1, &["1PE(3)"]);
    let table = BinTable::from_data(&header, vec![0u8; 8]).unwrap();
    assert!(matches!(
        table.read_column(0),
        Err(FitsError::VariableLengthColumn { code: 'P' })
    ));
}

#[test]
fn decodes_variable_length_arrays_from_the_heap() {
    // One `PE` column (f32 heap arrays), two rows of different lengths.
    // Main table = two 8-byte `P` descriptors; the heap follows at THEAP
    // (default = main size = 16).
    let mut header = table_header(8, 2, &["1PE(3)"]);
    header.set("PCOUNT", 12); // heap = 3 × f32
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
        table.read_vla_column(0).unwrap(),
        vec![ColumnData::F32(vec![1.0, 2.0]), ColumnData::F32(vec![3.0]),]
    );
}

#[test]
fn parses_tdisp_display_formats() {
    use TDispKind::*;
    let d = |kind, width, decimals, exponent| TDisp {
        kind,
        width,
        decimals,
        exponent,
    };
    let cases = [
        ("I5", d(Integer, 5, None, None)),
        ("F8.2", d(Float, 8, Some(2), None)),
        ("E12.5E3", d(Exponential, 12, Some(5), Some(3))),
        ("ES15.6", d(Scientific, 15, Some(6), None)),
        ("EN10.3", d(Engineering, 10, Some(3), None)),
        ("A20", d(Char, 20, None, None)),
        ("Z8", d(Hex, 8, None, None)),
    ];
    for (s, want) in cases {
        assert_eq!(TDisp::parse(s), Some(want), "{s}");
    }
    assert_eq!(TDisp::parse("Q5"), None); // unknown letter
    assert_eq!(TDisp::parse("F"), None); // missing width
    // The column reads TDISPn into a parsed TDisp.
    let mut header = table_header(4, 1, &["1J"]);
    header.set("TDISP1", "I5");
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert_eq!(table.columns[0].tdisp, Some(d(Integer, 5, None, None)));
}

#[test]
fn read_column_complex_widens_and_scales() {
    // `1C` (single-precision complex), TSCAL=2, TZERO=1 ⇒ each part scaled.
    let mut header = table_header(8, 1, &["1C"]);
    header.set("TSCAL1", 2.0).set("TZERO1", 1.0);
    let mut data = Vec::new();
    data.extend_from_slice(&3.0f32.to_be_bytes());
    data.extend_from_slice(&4.0f32.to_be_bytes());
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(table.read_column_complex(0).unwrap(), vec![(7.0, 9.0)]); // 1+2·3, 1+2·4
    // A non-complex column errors.
    let h2 = table_header(4, 1, &["1J"]);
    let t2 = BinTable::from_data(&h2, vec![0u8; 4]).unwrap();
    assert!(matches!(
        t2.read_column_complex(0),
        Err(FitsError::NotAComplexColumn { code: 'J' })
    ));
}

#[test]
fn read_column_unsigned_recovers_typed_values() {
    // `1I` with TZERO=2¹⁵ → u16; `1B` with TZERO=-128 → i8.
    let mut header = table_header(3, 1, &["1I", "1B"]);
    header.set("TZERO1", 32768.0).set("TZERO2", -128.0);
    let mut data = Vec::new();
    data.extend_from_slice(&((50000u16 ^ 0x8000) as i16).to_be_bytes());
    data.push(((-10i8) as u8) ^ 0x80);
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.read_column_unsigned(0).unwrap(),
        Some(UnsignedView::U16(vec![50000]))
    );
    assert_eq!(
        table.read_column_unsigned(1).unwrap(),
        Some(UnsignedView::I8(vec![-10]))
    );
}

#[test]
fn read_column_unsigned_is_exact_for_u64_and_none_otherwise() {
    // `1K` with TZERO=2⁶³ → u64, exact past 2⁵³; a plain `1J` (TZERO=0) is not
    // an unsigned column.
    let mut header = table_header(12, 1, &["1K", "1J"]);
    header.set("TZERO1", 9_223_372_036_854_775_808.0); // 2⁶³
    let mut data = Vec::new();
    data.extend_from_slice(&((u64::MAX ^ 0x8000_0000_0000_0000) as i64).to_be_bytes());
    data.extend_from_slice(&7i32.to_be_bytes());
    let table = BinTable::from_data(&header, data).unwrap();
    assert_eq!(
        table.read_column_unsigned(0).unwrap(),
        Some(UnsignedView::U64(vec![u64::MAX]))
    );
    assert_eq!(table.read_column_unsigned(1).unwrap(), None); // TZERO=0
}

#[test]
fn read_vla_column_physical_scales_heap_arrays_and_nulls() {
    // 1PJ column, TSCAL=2, TZERO=10, TNULL=99. Row 0 = [5, 99(null)], row 1 = [3].
    let mut header = table_header(8, 2, &["1PJ(2)"]);
    header
        .set("PCOUNT", 12)
        .set("TSCAL1", 2.0)
        .set("TZERO1", 10.0)
        .set("TNULL1", 99);
    let mut data = Vec::new();
    for (nelem, offset) in [(2i32, 0i32), (1, 8)] {
        data.extend_from_slice(&nelem.to_be_bytes());
        data.extend_from_slice(&offset.to_be_bytes());
    }
    for x in [5i32, 99, 3] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&header, data).unwrap();
    let phys = table.read_vla_column_physical(0).unwrap();
    assert_eq!(phys[0][0], 20.0); // 10 + 2·5
    assert!(phys[0][1].is_nan()); // 99 == TNULL
    assert_eq!(phys[1], vec![16.0]); // 10 + 2·3
}

#[test]
fn vla_descriptor_overrunning_the_heap_is_rejected() {
    // §6.6: a span must lie within the heap (`PCOUNT` bytes), not the block fill.
    // Heap is 8 bytes (PCOUNT=8) but the descriptor claims 3 f32 = 12 bytes.
    let mut header = table_header(8, 1, &["1PE(3)"]);
    header.set("PCOUNT", 8);
    let mut data = Vec::new();
    data.extend_from_slice(&3i32.to_be_bytes()); // nelem = 3
    data.extend_from_slice(&0i32.to_be_bytes()); // offset = 0
    data.extend_from_slice(&[0u8; 8]); // only 8 heap bytes (then block fill)
    data.resize(2880, 0); // block-padded fill that must NOT be read as heap
    let table = BinTable::from_data(&header, data).unwrap();
    assert!(matches!(
        table.read_vla_column(0),
        Err(FitsError::UnexpectedEof)
    ));
}

#[test]
fn x_bit_column_unpacks_msb_first() {
    // One `12X` column, 2 bytes/row. 0xAB 0xC0 = 1010_1011 1100_0000; the first
    // 12 bits MSB-first are 1010_1011_1100.
    let header = table_header(2, 1, &["12X"]);
    let table = BinTable::from_data(&header, vec![0xAB, 0xC0]).unwrap();
    let bits = table.read_bit_column(0).unwrap();
    let expect = [
        true, false, true, false, true, false, true, true, true, true, false, false,
    ];
    assert_eq!(bits, vec![expect.to_vec()]);
    // read_column still yields the raw packed bytes.
    assert_eq!(
        table.read_column(0).unwrap(),
        ColumnData::Bytes(vec![0xAB, 0xC0])
    );
}

#[test]
fn read_bit_column_on_a_non_bit_column_errors() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.read_bit_column(0),
        Err(FitsError::NotABitColumn { code: 'J' })
    ));
}

#[test]
fn column_index_is_case_insensitive() {
    let mut header = table_header(4, 1, &["1J"]);
    header.set("TTYPE1", "Flux");
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert_eq!(table.column_index("FLUX"), Some(0));
    assert_eq!(table.column_index("flux"), Some(0));
    assert_eq!(table.column_index("missing"), None);
}

#[test]
fn a_column_terminates_at_the_first_nul() {
    // §6.3: an embedded NUL ends the `A` string; bytes after it are dropped.
    assert_eq!(trim_text(b"AB\0CD\0\0"), "AB");
    assert_eq!(trim_text(b"hello   "), "hello"); // trailing spaces still trimmed
    assert_eq!(trim_text(b"\0junk"), ""); // leading NUL → empty
}

#[test]
fn read_vla_on_a_fixed_column_is_an_error() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.read_vla_column(0),
        Err(FitsError::NotAVla { code: 'J' })
    ));
}

#[test]
fn row_width_mismatch_is_an_error() {
    // Declared NAXIS1 = 99 but the one column is only 4 bytes wide.
    let header = table_header(99, 1, &["1J"]);
    assert!(matches!(
        BinTable::from_data(&header, vec![0u8; 4]),
        Err(FitsError::RowWidthMismatch {
            computed: 4,
            declared: 99
        })
    ));
}

#[test]
fn out_of_bounds_column_is_an_error() {
    let header = table_header(4, 1, &["1J"]);
    let table = BinTable::from_data(&header, vec![0u8; 4]).unwrap();
    assert!(matches!(
        table.read_column(9),
        Err(FitsError::ColumnIndexOutOfBounds { index: 9, len: 1 })
    ));
}

#[test]
fn reads_the_real_aips_antenna_table() {
    let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
    let mut reader = FitsReader::open(file).unwrap();
    let table = reader.read_table(1).unwrap();

    assert_eq!(table.nrows, 28);
    assert_eq!(table.columns.len(), 12);
    // ANNAME = 8A, STABXYZ = 3D, ORBPARM = 0D, NOSTA = 1J ...
    assert_eq!(table.columns[0].name.as_deref(), Some("ANNAME"));
    assert_eq!(table.columns[0].tform, tform(8, TformKind::Char, None));
    assert_eq!(table.columns[1].tform, tform(3, TformKind::F64, None));
    assert_eq!(table.columns[2].tform, tform(0, TformKind::F64, None));
    // The 0D ORBPARM column contributes no width, so NOSTA shares its offset.
    assert_eq!(table.columns[2].byte_offset, 32);
    assert_eq!(table.columns[3].byte_offset, 32);
    assert_eq!(table.columns[1].unit.as_deref(), Some("METERS"));

    // Decoded element counts: one ANNAME string per row, 3 doubles per row, none for 0D.
    match table.read_column(0).unwrap() {
        ColumnData::Text(v) => assert_eq!(v.len(), 28),
        other => panic!("ANNAME should be Text, got {other:?}"),
    }
    match table.read_column(1).unwrap() {
        ColumnData::F64(v) => assert_eq!(v.len(), 28 * 3),
        other => panic!("STABXYZ should be F64, got {other:?}"),
    }
    assert_eq!(table.read_column(2).unwrap(), ColumnData::F64(vec![]));
    assert_eq!(table.column_index("NOSTA"), Some(3));
}

#[test]
fn read_table_rejects_non_bintable_hdus() {
    let file = File::open("tests/data/fits/DDTSUVDATA.fits").unwrap();
    let mut reader = FitsReader::open(file).unwrap();
    // HDU 0 is a random-groups primary, not a binary table.
    assert!(matches!(reader.read_table(0), Err(FitsError::NotABinTable)));
}

#[test]
fn vla_bit_column_unpacks_msb_first() {
    // A `1PX` column: row 0 = 12 bits (0xAB 0xC0), row 1 = 4 bits (0xF0), MSB-first.
    let mut header = Header::new();
    header
        .set("XTENSION", "BINTABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 8) // one P descriptor (2 × i32) per row
        .set("NAXIS2", 2)
        .set("PCOUNT", 3) // heap bytes
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TFORM1", "1PX");
    let mut data = Vec::new();
    data.extend_from_slice(&12i32.to_be_bytes()); // row 0: 12 bits …
    data.extend_from_slice(&0i32.to_be_bytes()); //        … at heap offset 0
    data.extend_from_slice(&4i32.to_be_bytes()); // row 1: 4 bits …
    data.extend_from_slice(&2i32.to_be_bytes()); //        … at heap offset 2
    data.extend_from_slice(&[0xAB, 0xC0, 0xF0]); // heap
    let table = BinTable::from_data(&header, data).unwrap();

    let rows = table.read_vla_bit_column(0).unwrap();
    assert_eq!(
        rows[0],
        vec![
            true, false, true, false, true, false, true, true, true, true, false, false
        ]
    );
    assert_eq!(rows[1], vec![true, true, true, true]);
}

#[test]
fn tfields_beyond_999_is_rejected() {
    // §7.3.1 caps TFIELDS at 999; an absurd value must error, not size a huge Vec.
    let mut header = table_header(0, 0, &[]);
    header.set("TFIELDS", 1000);
    assert!(matches!(
        BinTable::from_data(&header, vec![]),
        Err(FitsError::WrongValueType { name: "TFIELDS" })
    ));
}
