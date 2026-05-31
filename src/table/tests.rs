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
    for bad in ["9Z", "", "1P"] {
        // "1P" lacks the required heap element-type letter.
        assert!(
            matches!(Tform::parse(bad), Err(FitsError::InvalidTform { .. })),
            "{bad}"
        );
    }
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
    let header = table_header(8, 2, &["1PE(3)"]);
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
