use crate::block::ZERO_FILL;
use crate::block::padded_len;
#[cfg(feature = "compression")]
use crate::compress::CompressOptions;
use crate::data::{ImageData, Scaling, UnsignedView};
use crate::hdu::{HduKind, MAX_TABLE_FIELDS};
use crate::header::from_card_lines as header;
use crate::header::value::Value;
use crate::reader::ChecksumReport;
use crate::reader::FitsReader;
use crate::table::{CharacterField, ColumnData};
use crate::writer::*;
use bitvec::bitvec;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use num_complex::Complex;
use std::io;
use std::io::Cursor;
use std::io::Write;

#[derive(Debug, Default)]
struct FailMidHdu {
    bytes: Vec<u8>,
    failed: bool,
}

impl Write for FailMidHdu {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.is_empty() {
            let written = buf.len().min(127);
            self.bytes.extend_from_slice(&buf[..written]);
            return Ok(written);
        }
        if !self.failed {
            self.failed = true;
            return Err(io::Error::other("injected write failure"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn write_to_vec(image: &Image) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_image(image).unwrap();
    w.into_inner().into_inner()
}

fn identity() -> Scaling {
    Scaling {
        bscale: 1.0,
        bzero: 0.0,
        blank: None,
    }
}

#[derive(Debug)]
struct TypedWriteColumn {
    stored_type: char,
    column: WriteColumn,
}

fn fixed_columns_by_stored_type() -> Vec<TypedWriteColumn> {
    vec![
        TypedWriteColumn {
            stored_type: 'L',
            column: WriteColumn::fixed("L", ColumnData::Logical(vec![Some(true)]), 1),
        },
        TypedWriteColumn {
            stored_type: 'B',
            column: WriteColumn::fixed("B", ColumnData::Bytes(vec![1]), 1),
        },
        TypedWriteColumn {
            stored_type: 'I',
            column: WriteColumn::fixed("I", ColumnData::I16(vec![1]), 1),
        },
        TypedWriteColumn {
            stored_type: 'J',
            column: WriteColumn::fixed("J", ColumnData::I32(vec![1]), 1),
        },
        TypedWriteColumn {
            stored_type: 'K',
            column: WriteColumn::fixed("K", ColumnData::I64(vec![1]), 1),
        },
        TypedWriteColumn {
            stored_type: 'E',
            column: WriteColumn::fixed("E", ColumnData::F32(vec![1.0]), 1),
        },
        TypedWriteColumn {
            stored_type: 'D',
            column: WriteColumn::fixed("D", ColumnData::F64(vec![1.0]), 1),
        },
        TypedWriteColumn {
            stored_type: 'C',
            column: WriteColumn::fixed(
                "C",
                ColumnData::ComplexF32(vec![Complex::new(1.0, 2.0)]),
                1,
            ),
        },
        TypedWriteColumn {
            stored_type: 'M',
            column: WriteColumn::fixed(
                "M",
                ColumnData::ComplexF64(vec![Complex::new(1.0, 2.0)]),
                1,
            ),
        },
        TypedWriteColumn {
            stored_type: 'A',
            column: WriteColumn::fixed("A", ColumnData::Character(vec!["a".into()]), 1),
        },
        TypedWriteColumn {
            stored_type: 'X',
            column: WriteColumn::bits("X", vec![0x80], 1),
        },
    ]
}

fn vla_columns_by_stored_type() -> Vec<TypedWriteColumn> {
    vec![
        TypedWriteColumn {
            stored_type: 'L',
            column: WriteColumn::vla(
                "PL",
                ColumnType::Logical,
                vec![ColumnData::Logical(vec![Some(true)])],
            ),
        },
        TypedWriteColumn {
            stored_type: 'B',
            column: WriteColumn::vla("PB", ColumnType::Byte, vec![ColumnData::Bytes(vec![1])]),
        },
        TypedWriteColumn {
            stored_type: 'I',
            column: WriteColumn::vla("PI", ColumnType::I16, vec![ColumnData::I16(vec![1])]),
        },
        TypedWriteColumn {
            stored_type: 'J',
            column: WriteColumn::vla("PJ", ColumnType::I32, vec![ColumnData::I32(vec![1])]),
        },
        TypedWriteColumn {
            stored_type: 'K',
            column: WriteColumn::vla("PK", ColumnType::I64, vec![ColumnData::I64(vec![1])]),
        },
        TypedWriteColumn {
            stored_type: 'E',
            column: WriteColumn::vla("PE", ColumnType::F32, vec![ColumnData::F32(vec![1.0])]),
        },
        TypedWriteColumn {
            stored_type: 'D',
            column: WriteColumn::vla("PD", ColumnType::F64, vec![ColumnData::F64(vec![1.0])]),
        },
        TypedWriteColumn {
            stored_type: 'C',
            column: WriteColumn::vla(
                "PC",
                ColumnType::ComplexF32,
                vec![ColumnData::ComplexF32(vec![Complex::new(1.0, 2.0)])],
            ),
        },
        TypedWriteColumn {
            stored_type: 'M',
            column: WriteColumn::vla(
                "PM",
                ColumnType::ComplexF64,
                vec![ColumnData::ComplexF64(vec![Complex::new(1.0, 2.0)])],
            ),
        },
        TypedWriteColumn {
            stored_type: 'A',
            column: WriteColumn::vla(
                "PA",
                ColumnType::Character,
                vec![ColumnData::Character(vec!["a".into()])],
            ),
        },
        TypedWriteColumn {
            stored_type: 'X',
            column: WriteColumn::vla_bits("PX", vec![bitvec![u8, Msb0; 1]]),
        },
    ]
}

fn assert_table_column_writes(column: WriteColumn) {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(1, &[column]).unwrap();
}

fn assert_table_column_rejected(column: WriteColumn, keyword: &'static str) {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[column]),
        Err(FitsError::KeywordOutOfRange { name }) if name == keyword
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

fn empty_binary_columns(count: usize) -> Vec<WriteColumn> {
    (1..=count)
        .map(|n| WriteColumn::fixed(format!("C{n}"), ColumnData::Bytes(Vec::new()), 1))
        .collect()
}

fn empty_ascii_columns(count: usize) -> Vec<AsciiWriteColumn> {
    (1..=count)
        .map(|n| AsciiWriteColumn {
            name: format!("C{n}"),
            unit: None,
            data: AsciiColumnData::Text(Vec::new()),
            width: 1,
            decimals: 0,
            tscale: None,
            tzero: None,
            tnull: None,
        })
        .collect()
}

fn rendered_header(header: &Header) -> Vec<u8> {
    let mut bytes = Vec::new();
    render_header(header, &mut bytes).unwrap();
    bytes
}

#[test]
fn writer_rejects_invalid_or_overflowing_layouts() {
    let mismatched = Image {
        shape: vec![2],
        samples: ImageData::U8(vec![1]),
        scaling: identity(),
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_image(&mismatched),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let image = Image {
        shape: vec![usize::MAX, 2],
        samples: ImageData::U8(Vec::new()),
        scaling: identity(),
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_image(&image),
        Err(FitsError::DataUnitOverflow)
    ));

    let fixed = WriteColumn::fixed("X", ColumnData::Bytes(Vec::new()), usize::MAX);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(2, &[fixed]),
        Err(FitsError::DataUnitOverflow)
    ));

    let ascii = |name: &str, width| AsciiWriteColumn {
        name: name.to_string(),
        unit: None,
        data: AsciiColumnData::Text(Vec::new()),
        width,
        decimals: 0,
        tscale: None,
        tzero: None,
        tnull: None,
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(0, &[ascii("A", usize::MAX), ascii("B", 1)]),
        Err(FitsError::DataUnitOverflow)
    ));

    let invalid_bits = WriteColumn::bits("FLAGS", vec![0; 3], 12);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(2, &[invalid_bits]),
        Err(FitsError::RowWidthMismatch {
            computed: 3,
            declared: 4
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let invalid_vla_bits = WriteColumn::vla_bits("FLAGS", vec![bitvec![u8, Msb0; 1, 0, 1]]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(2, &[invalid_vla_bits]),
        Err(FitsError::RowWidthMismatch {
            computed: 1,
            declared: 2
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let invalid_vla_bits =
        WriteColumn::vla_bits("FLAGS", vec![bitvec![u8, Msb0; 1, 0, 1]]).with_tdim(vec![2, 2]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[invalid_vla_bits]),
        Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let invalid_tdim =
        WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4).with_tdim(vec![5]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[invalid_tdim]),
        Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    for shape in [vec![], vec![0]] {
        let invalid_empty_vla =
            WriteColumn::vla("VEC", ColumnType::I32, vec![ColumnData::I32(vec![])])
                .with_tdim(shape);
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_table(1, &[invalid_empty_vla]),
            Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
        ));
        assert!(writer.into_inner().into_inner().is_empty());
    }

    let invalid_text = WriteColumn::fixed(
        "NAME",
        ColumnData::Character(vec![CharacterField::new("Véga".as_bytes().to_vec())]),
        4,
    );
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[invalid_text]),
        Err(FitsError::InvalidAscii {
            context: "binary character cell"
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    #[cfg(target_pointer_width = "64")]
    {
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_table(usize::MAX, &[]),
            Err(FitsError::DataUnitOverflow)
        ));
    }
}

#[test]
fn table_writers_enforce_the_exact_tfields_limit_before_output() {
    let binary = empty_binary_columns(MAX_TABLE_FIELDS);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(0, &binary).unwrap();
    let reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader.hdus[1].header.get_integer("TFIELDS").unwrap(),
        Some(MAX_TABLE_FIELDS as i64)
    );

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_ascii_table(0, &ascii).unwrap();
    let reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader.hdus[1].header.get_integer("TFIELDS").unwrap(),
        Some(MAX_TABLE_FIELDS as i64)
    );

    let binary = empty_binary_columns(MAX_TABLE_FIELDS + 1);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(0, &binary),
        Err(FitsError::KeywordOutOfRange { name: "TFIELDS" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS + 1);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_ascii_table(0, &ascii),
        Err(FitsError::KeywordOutOfRange { name: "TFIELDS" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

#[test]
fn partial_hdu_failure_permanently_rejects_subsequent_writes() {
    let image = Image {
        shape: vec![1],
        samples: ImageData::U8(vec![7]),
        scaling: identity(),
    };
    let mut writer = FitsWriter::new(FailMidHdu::default());
    assert!(matches!(writer.write_image(&image), Err(FitsError::Io(_))));
    assert_eq!(writer.state, WriterState::Failed);
    assert_eq!(writer.sink.bytes.len(), 127);

    assert!(matches!(
        writer.write_image(&image),
        Err(FitsError::WriterFailed)
    ));
    assert_eq!(writer.into_inner().bytes.len(), 127);
}

#[test]
fn writes_a_multi_hdu_image_file() {
    let primary = Image {
        shape: vec![2, 2],
        samples: ImageData::U8(vec![1, 2, 3, 4]),
        scaling: identity(),
    };
    let ext = Image {
        shape: vec![3],
        samples: ImageData::I16(vec![10, 20, 30]),
        scaling: identity(),
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_image(&primary).unwrap();
    let header_ptr = w.header_scratch.as_ptr();
    let header_capacity = w.header_scratch.capacity();
    w.write_image(&ext).unwrap(); // second image ⇒ IMAGE extension
    assert_eq!(w.header_scratch.as_ptr(), header_ptr);
    assert_eq!(w.header_scratch.capacity(), header_capacity);
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus.len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[1].kind, HduKind::Image);
    assert_eq!(
        r.read_image(0).unwrap().decode(),
        ImageData::U8(vec![1, 2, 3, 4])
    );
    assert_eq!(
        r.read_image(1).unwrap().decode(),
        ImageData::I16(vec![10, 20, 30])
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
        WriteColumn::vla("DATA", ColumnType::I32, vla_rows.clone()),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(3, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let table = r.read_table(1).unwrap();
    // TFORM2 should be a P descriptor sized to the longest row (5).
    assert_eq!(table.columns[1].tform.kind.code(), 'P');
    let got = table.column_by_idx(1).unwrap().vla().unwrap();
    assert_eq!(got.len(), 3);
    for (g, want) in got.iter().zip(&vla_rows) {
        match (g, want) {
            (ColumnData::I32(a), ColumnData::I32(b)) => assert_eq!(a, b),
            _ => panic!("expected I32 VLA cell, got {g:?}"),
        }
    }

    let empty = [WriteColumn::vla("EMPTY", ColumnType::I64, Vec::new())];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(0, &empty).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let table = r.read_table(1).unwrap();
    assert_eq!(
        table.columns[0].tform.vla_elem,
        Some(crate::table::TformKind::I64)
    );
    assert!(table.column_by_idx(0).unwrap().vla().unwrap().is_empty());
}

#[test]
#[should_panic(expected = "VLA column cells must match the declared ColumnType")]
fn vla_constructor_rejects_mixed_element_types() {
    WriteColumn::vla("BAD", ColumnType::I16, vec![ColumnData::I32(vec![1])]);
}

#[test]
fn writes_tdim_p_q_vla_and_bit_columns() {
    use crate::table::TformKind;
    let columns = vec![
        // 2×2 multidimensional column (TDIM '(2,2)'), 4 elements/row.
        WriteColumn::fixed("MAT", ColumnData::I32((1..=8).collect()), 4).with_tdim(vec![2, 2]),
        // 64-bit Q VLA column.
        WriteColumn::vla(
            "QV",
            ColumnType::I16,
            vec![ColumnData::I16(vec![]), ColumnData::I16(vec![7, 8, 9, 10])],
        )
        .with_tdim(vec![2, 2])
        .wide(),
        WriteColumn::vla(
            "PV",
            ColumnType::I16,
            vec![ColumnData::I16(vec![]), ColumnData::I16(vec![1, 2, 3, 4])],
        )
        .with_tdim(vec![2, 2]),
        WriteColumn::vla(
            "TXT",
            ColumnType::Character,
            vec![
                ColumnData::Character(vec!["hello".into()]),
                ColumnData::Character(vec!["abc".into()]),
            ],
        ),
        // 12-bit X column: 2 bytes/row.
        WriteColumn::bits("FLAGS", vec![0xAB, 0xCF, 0x12, 0x3F], 12),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let t = r.read_table(1).unwrap();

    // TDIM parsed back as a shape.
    assert_eq!(t.columns[0].tdim, Some(vec![2, 2]));
    // P/Q descriptors with TDIM accept an empty row and validate the nonempty row.
    assert_eq!(t.columns[1].tform.kind, TformKind::ArrayDesc64);
    assert_eq!(t.columns[1].tdim, Some(vec![2, 2]));
    assert_eq!(
        t.column_by_idx(1).unwrap().vla().unwrap(),
        vec![ColumnData::I16(vec![]), ColumnData::I16(vec![7, 8, 9, 10])]
    );
    assert_eq!(t.columns[2].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(t.columns[2].tdim, Some(vec![2, 2]));
    assert_eq!(
        t.column_by_idx(2).unwrap().vla().unwrap(),
        vec![ColumnData::I16(vec![]), ColumnData::I16(vec![1, 2, 3, 4])]
    );
    assert_eq!(r.hdus[1].header.get_text("TFORM4").unwrap(), Some("1PA(5)"));
    assert_eq!(t.columns[3].tform.repeat, 1);
    assert_eq!(t.columns[3].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(t.columns[3].tform.vla_elem, Some(TformKind::Char));
    assert_eq!(
        t.column_by_idx(3).unwrap().vla().unwrap(),
        vec![
            ColumnData::Character(vec!["hello".into()]),
            ColumnData::Character(vec!["abc".into()])
        ]
    );
    assert_eq!(t.columns[4].tform.kind, TformKind::Bit);
    assert_eq!(t.columns[4].tform.repeat, 12);
    match t.column_by_idx(4).unwrap().raw().unwrap() {
        ColumnData::Bytes(b) => assert_eq!(b, vec![0xAB, 0xC0, 0x12, 0x30]),
        other => panic!("{other:?}"),
    }
}

#[test]
fn writes_p_q_variable_length_bit_arrays_with_exact_counts_and_padding() {
    use crate::table::TformKind;

    let mut one_bit = bitvec![u8, Msb0; 1; 8];
    one_bit.truncate(1);
    let mut p_nine = bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1];
    p_nine.truncate(9);
    let mut q_nine = bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1];
    q_nine.truncate(9);
    assert_eq!(one_bit.as_raw_slice(), &[0xFF]);
    assert_eq!(p_nine.as_raw_slice(), &[0xAA, 0xFF]);
    assert_eq!(q_nine.as_raw_slice(), &[0x55, 0x7F]);
    let p_rows = vec![BitVec::<u8, Msb0>::new(), one_bit.clone(), p_nine];
    let q_rows = vec![BitVec::<u8, Msb0>::new(), one_bit, q_nine];
    let columns = [
        WriteColumn::vla_bits("PFLAGS", p_rows.clone()),
        WriteColumn::vla_bits("QFLAGS", q_rows.clone()).wide(),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(3, &columns).unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();

    assert_eq!(
        reader.hdus[1].header.get_text("TFORM1").unwrap(),
        Some("1PX(9)")
    );
    assert_eq!(
        reader.hdus[1].header.get_text("TFORM2").unwrap(),
        Some("1QX(9)")
    );
    assert_eq!(
        reader.hdus[1].header.get_integer("PCOUNT").unwrap(),
        Some(6)
    );

    let data = reader.read_data_raw(1).unwrap();
    let raw = data.data();
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
    assert_eq!(table.columns[0].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(table.columns[0].tform.vla_elem, Some(TformKind::Bit));
    assert_eq!(table.columns[1].tform.kind, TformKind::ArrayDesc64);
    assert_eq!(table.columns[1].tform.vla_elem, Some(TformKind::Bit));
    let p = table.column_by_idx(0).unwrap().vla_bits().unwrap();
    let q = table.column_by_idx(1).unwrap().vla_bits().unwrap();
    for row in 0..3 {
        assert_eq!(p.row(row), p_rows[row].as_bitslice());
        assert_eq!(q.row(row), q_rows[row].as_bitslice());
    }
}

#[test]
fn writes_tscal_tzero_tnull_and_reads_back_physical() {
    // Stored [5, 99] with TSCAL=2, TZERO=10, TNULL=99 ⇒ physical [20, NaN].
    let columns = vec![
        WriteColumn::fixed("X", ColumnData::I32(vec![5, 99]), 1)
            .scaled(2.0, 10.0)
            .with_null(99),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    assert_eq!(r.hdus[1].header.get_real("TSCAL1").unwrap(), Some(2.0));
    assert_eq!(r.hdus[1].header.get_real("TZERO1").unwrap(), Some(10.0));
    assert_eq!(r.hdus[1].header.get_integer("TNULL1").unwrap(), Some(99));
    let t = r.read_table(1).unwrap();
    let phys = t.column_by_idx(0).unwrap().physical().unwrap();
    assert_eq!(phys[0], 20.0);
    assert!(phys[1].is_nan());
}

#[test]
fn binary_metadata_is_validated_for_every_fixed_and_vla_stored_type() {
    for typed in fixed_columns_by_stored_type()
        .into_iter()
        .chain(vla_columns_by_stored_type())
    {
        if matches!(typed.stored_type, 'A' | 'L' | 'X') {
            assert_table_column_rejected(typed.column.clone().scaled(2.0, 3.0), "TSCALn");

            let mut tzero_only = typed.column.clone();
            tzero_only.tzero = Some(3.0);
            assert_table_column_rejected(tzero_only, "TZEROn");
        } else {
            assert_table_column_writes(typed.column.clone().scaled(2.0, 3.0));
        }

        if matches!(typed.stored_type, 'B' | 'I' | 'J' | 'K') {
            assert_table_column_writes(typed.column.with_null(0));
        } else {
            assert_table_column_rejected(typed.column.with_null(0), "TNULLn");
        }
    }
}

#[derive(Debug)]
struct NullBoundary {
    kind: ColumnType,
    valid: &'static [i64],
    invalid: &'static [i64],
}

fn integer_column(kind: ColumnType, vla: bool) -> WriteColumn {
    let data = match kind {
        ColumnType::Byte => ColumnData::Bytes(vec![0]),
        ColumnType::I16 => ColumnData::I16(vec![0]),
        ColumnType::I32 => ColumnData::I32(vec![0]),
        ColumnType::I64 => ColumnData::I64(vec![0]),
        _ => panic!("integer_column requires an integer stored type"),
    };
    if vla {
        WriteColumn::vla("PINT", kind, vec![data])
    } else {
        WriteColumn::fixed("INT", data, 1)
    }
}

#[test]
fn binary_tnull_is_range_checked_for_fixed_and_vla_integer_types() {
    let boundaries = [
        NullBoundary {
            kind: ColumnType::Byte,
            valid: &[0, u8::MAX as i64],
            invalid: &[-1, u8::MAX as i64 + 1],
        },
        NullBoundary {
            kind: ColumnType::I16,
            valid: &[i16::MIN as i64, i16::MAX as i64],
            invalid: &[i16::MIN as i64 - 1, i16::MAX as i64 + 1],
        },
        NullBoundary {
            kind: ColumnType::I32,
            valid: &[i32::MIN as i64, i32::MAX as i64],
            invalid: &[i32::MIN as i64 - 1, i32::MAX as i64 + 1],
        },
        NullBoundary {
            kind: ColumnType::I64,
            valid: &[i64::MIN, i64::MAX],
            invalid: &[],
        },
    ];
    for boundary in boundaries {
        for vla in [false, true] {
            for &tnull in boundary.valid {
                assert_table_column_writes(integer_column(boundary.kind, vla).with_null(tnull));
            }
            for &tnull in boundary.invalid {
                assert_table_column_rejected(
                    integer_column(boundary.kind, vla).with_null(tnull),
                    "TNULLn",
                );
            }
        }
    }
}

#[derive(Debug)]
struct NonfiniteTableScale {
    keyword: &'static str,
    tscale: Option<f64>,
    tzero: Option<f64>,
}

#[test]
fn binary_nonfinite_scaling_is_rejected_before_output() {
    let cases = [
        NonfiniteTableScale {
            keyword: "TSCALn",
            tscale: Some(f64::NAN),
            tzero: None,
        },
        NonfiniteTableScale {
            keyword: "TSCALn",
            tscale: Some(f64::INFINITY),
            tzero: None,
        },
        NonfiniteTableScale {
            keyword: "TZEROn",
            tscale: None,
            tzero: Some(f64::NAN),
        },
        NonfiniteTableScale {
            keyword: "TZEROn",
            tscale: None,
            tzero: Some(f64::NEG_INFINITY),
        },
    ];
    for vla in [false, true] {
        for case in &cases {
            let mut column = integer_column(ColumnType::I32, vla);
            column.tscale = case.tscale;
            column.tzero = case.tzero;
            assert_table_column_rejected(column, case.keyword);
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
            ColumnData::Character(vec!["AB".into(), "CDE".into(), "F".into()]),
            3, // 3-char field
        ),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(3, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    // A dataless primary is auto-written before the table extension.
    assert_eq!(r.hdus.len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[0].header.naxis().unwrap(), 0);
    assert_eq!(r.hdus[1].kind, HduKind::BinTable);

    let t = r.read_table(1).unwrap();
    assert_eq!(t.nrows, 3);
    assert_eq!(t.columns.len(), 3);
    assert_eq!(t.columns[0].name.as_deref(), Some("NOSTA"));
    assert_eq!(t.columns[1].unit.as_deref(), Some("m"));
    assert_eq!(
        t.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2, 3])
    );
    assert_eq!(
        t.column_by_idx(1).unwrap().raw().unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
    );
    assert_eq!(
        t.column_by_idx(2).unwrap().raw().unwrap(),
        ColumnData::Character(vec![
            CharacterField::new(b"AB ".to_vec()),
            CharacterField::new(b"CDE".to_vec()),
            CharacterField::new(b"F  ".to_vec())
        ])
    );
}

#[test]
fn binary_character_columns_round_trip_exactly_and_reject_over_width() {
    let fields = vec![
        CharacterField::new(b"AB  ".to_vec()),
        CharacterField::new(b"AB\0x".to_vec()),
        CharacterField::new(b"\0xyz".to_vec()),
        CharacterField::new(b"    ".to_vec()),
    ];
    let vla_rows: Vec<_> = fields
        .iter()
        .cloned()
        .map(|field| ColumnData::Character(vec![field]))
        .collect();
    let columns = [
        WriteColumn::fixed("FIXED", ColumnData::Character(fields.clone()), 4),
        WriteColumn::vla("PCHAR", ColumnType::Character, vla_rows.clone()),
        WriteColumn::vla("QCHAR", ColumnType::Character, vla_rows.clone()).wide(),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(4, &columns).unwrap();
    let bytes = writer.into_inner().into_inner();

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(
        reader.hdus[1].header.get_text("TFORM2").unwrap(),
        Some("1PA(4)")
    );
    assert_eq!(
        reader.hdus[1].header.get_text("TFORM3").unwrap(),
        Some("1QA(4)")
    );
    let table = reader.read_table(1).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::Character(fields.clone())
    );
    assert_eq!(table.column_by_idx(1).unwrap().vla().unwrap(), vla_rows);
    assert_eq!(table.column_by_idx(2).unwrap().vla().unwrap(), vla_rows);
    assert_eq!(fields[0].members(), b"AB  ");
    assert_eq!(fields[1].members(), b"AB");
    assert!(fields[2].is_null());
    assert!(!fields[3].is_null());
    assert_eq!(fields[3].members(), b"    ");

    let too_wide = WriteColumn::fixed(
        "BAD",
        ColumnData::Character(vec![CharacterField::new(b"ABCDE".to_vec())]),
        4,
    );
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[too_wide]),
        Err(FitsError::RowWidthMismatch {
            computed: 5,
            declared: 4
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

#[test]
fn pad_to_block_rounds_up_with_the_fill_byte() {
    let mut empty = Vec::new();
    pad_to_block(&mut empty, ZERO_FILL).unwrap();
    assert_eq!(empty.len(), 0);

    let mut one = vec![1u8];
    pad_to_block(&mut one, ZERO_FILL).unwrap();
    assert_eq!(one.len(), BLOCK_SIZE);
    assert_eq!(one[0], 1);
    assert!(one[1..].iter().all(|&b| b == ZERO_FILL));

    let mut exact = vec![7u8; BLOCK_SIZE];
    pad_to_block(&mut exact, ZERO_FILL).unwrap();
    assert_eq!(exact.len(), BLOCK_SIZE);

    let mut over = vec![0u8; BLOCK_SIZE + 1];
    pad_to_block(&mut over, ZERO_FILL).unwrap();
    assert_eq!(over.len(), 2 * BLOCK_SIZE);
}

#[test]
fn rendered_header_is_block_aligned_and_ends_in_end_then_spaces() {
    let unit = rendered_header(&header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
    ]));
    assert_eq!(unit.len() % BLOCK_SIZE, 0);
    assert_eq!(unit.len(), BLOCK_SIZE); // 4 cards fit in one block

    // The 4th card (index 3) is END, followed by space padding.
    assert_eq!(&unit[3 * CARD_SIZE..3 * CARD_SIZE + 3], b"END");
    assert!(unit[4 * CARD_SIZE..].iter().all(|&b| b == SPACE_FILL));
}

#[test]
fn header_round_trips_through_render_and_parse() {
    let original = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                  -32",
        "NAXIS   =                    2",
        "NAXIS1  =                  100",
        "NAXIS2  =                   50",
        "OBJECT  = 'O''Brien'",
        "COMMENT  a remark",
    ]);
    let reparsed = Header::parse(&rendered_header(&original)).unwrap();
    assert_eq!(reparsed.cards, original.cards);

    let mut invalid = Header::new();
    assert!(matches!(
        invalid.try_set("OBJECT", "Véga"),
        Err(FitsError::InvalidAscii {
            context: "header text value"
        })
    ));
    assert!(invalid.cards.is_empty());
}

#[test]
fn image_round_trips_through_write_image_and_read_image() {
    let image = Image {
        shape: vec![2, 3],
        samples: ImageData::I16(vec![1, -2, 3, -4, 5, -6]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let bytes = write_to_vec(&image);
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE); // one header block + one data block

    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(r.hdus.len(), 1);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    let back = r.read_image(0).unwrap();
    assert_eq!(back.shape, vec![2, 3]);
    assert_eq!(back.decode(), ImageData::I16(vec![1, -2, 3, -4, 5, -6]));
}

#[test]
fn write_image_emits_scaling_keywords_and_preserves_unsigned_values() {
    // u16 data stored as signed-16 with BZERO = 32768.
    let image = Image {
        shape: vec![3],
        samples: ImageData::I16(vec![-32768, 0, 32767]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 32768.0,
            blank: None,
        },
    };
    let mut r = FitsReader::open(Cursor::new(write_to_vec(&image))).unwrap();
    assert_eq!(r.hdus[0].header.get_real("BZERO").unwrap(), Some(32768.0));
    assert_eq!(r.hdus[0].header.get_real("BSCALE").unwrap(), Some(1.0));
    let back = r.read_image(0).unwrap();
    assert_eq!(back.physical(), vec![0.0, 32768.0, 65535.0]);
    assert_eq!(back.decode(), ImageData::I16(vec![-32768, 0, 32767]));
}

#[test]
fn from_u16_round_trips_through_write_and_read() {
    // The `from_u16` constructor + writer emit BZERO=32768 so the exact u16 values
    // come back via the typed `unsigned()` view.
    let built = Image::from_u16(vec![3], &[0, 32768, 65535]).unwrap();
    let mut r = FitsReader::open(Cursor::new(write_to_vec(&built))).unwrap();
    assert_eq!(r.hdus[0].header.get_real("BZERO").unwrap(), Some(32768.0));
    assert_eq!(
        r.read_image(0).unwrap().unsigned(),
        Some(UnsignedView::U16(vec![0, 32768, 65535]))
    );
}

#[test]
fn from_u64_writes_the_exact_offset_and_round_trips_extremes() {
    let built = Image::from_u64(vec![2], &[u64::MIN, u64::MAX]).unwrap();
    let bytes = write_to_vec(&built);
    let bzero = bytes[..BLOCK_SIZE]
        .chunks_exact(CARD_SIZE)
        .find(|card| &card[..8] == b"BZERO   ")
        .unwrap();
    assert_eq!(&bzero[10..30], b" 9223372036854775808");

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    let Value::Integer(offset) = reader.hdus[0].header.get("BZERO").unwrap() else {
        panic!("unsigned-64 BZERO was not stored as an exact integer");
    };
    assert_eq!(offset.to_string(), "9223372036854775808");
    assert_eq!(
        reader.read_image(0).unwrap().unsigned(),
        Some(UnsignedView::U64(vec![u64::MIN, u64::MAX]))
    );
}

#[test]
fn i64_table_scaling_writes_the_exact_unsigned_offset() {
    let rows = vec![
        ColumnData::I64(vec![i64::MIN]),
        ColumnData::I64(vec![i64::MAX]),
    ];
    let columns = [
        WriteColumn::fixed("U64", ColumnData::I64(vec![i64::MIN, i64::MAX]), 1)
            .scaled(1.0, 9_223_372_036_854_775_808.0),
        WriteColumn::vla("PU64", ColumnType::I64, rows.clone())
            .scaled(1.0, 9_223_372_036_854_775_808.0),
        WriteColumn::vla("QU64", ColumnType::I64, rows.clone())
            .wide()
            .scaled(1.0, 9_223_372_036_854_775_808.0),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(2, &columns).unwrap();
    let bytes = writer.into_inner().into_inner();
    for keyword in ["TZERO1  ", "TZERO2  ", "TZERO3  "] {
        let tzero = bytes
            .chunks_exact(CARD_SIZE)
            .find(|card| &card[..8] == keyword.as_bytes())
            .unwrap();
        assert_eq!(&tzero[10..30], b" 9223372036854775808");
    }

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(
        reader.hdus[1].header.get_text("TFORM2").unwrap(),
        Some("1PK(1)")
    );
    assert_eq!(
        reader.hdus[1].header.get_text("TFORM3").unwrap(),
        Some("1QK(1)")
    );
    let table = reader.read_table(1).unwrap();
    assert_eq!(
        table.column_by_idx(0).unwrap().unsigned().unwrap(),
        Some(UnsignedView::U64(vec![u64::MIN, u64::MAX]))
    );
    assert_eq!(table.column_by_idx(1).unwrap().vla().unwrap(), rows);
    assert_eq!(table.column_by_idx(2).unwrap().vla().unwrap(), rows);
}

#[test]
fn checksums_round_trip_and_verify() {
    let image = Image {
        shape: vec![2, 2],
        samples: ImageData::I16(vec![1, 2, 3, 4]),
        scaling: identity(),
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    w.write_image(&image).unwrap();
    let bytes = w.into_inner().into_inner();
    let mut r = FitsReader::open(Cursor::new(bytes.clone())).unwrap();
    let report = r.verify_checksum(0).unwrap();
    assert_eq!(report.datasum_ok, Some(true));
    assert_eq!(report.checksum_ok, Some(true)); // whole-HDU sum is −0

    let mut slice = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(slice.verify_checksum(0).unwrap(), report);
}

#[test]
fn corrupted_data_fails_checksum() {
    let image = Image {
        shape: vec![2, 2],
        samples: ImageData::I16(vec![1, 2, 3, 4]),
        scaling: identity(),
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    w.write_image(&image).unwrap();
    let mut bytes = w.into_inner().into_inner();
    bytes[BLOCK_SIZE] ^= 0xFF; // flip the first data byte (data starts at block 1)

    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    let report = r.verify_checksum(0).unwrap();
    assert_eq!(report.datasum_ok, Some(false));
    assert_eq!(report.checksum_ok, Some(false));
}

#[test]
fn corrupted_header_padding_fails_only_the_whole_hdu_checksum() {
    let image = Image {
        shape: vec![2, 2],
        samples: ImageData::I16(vec![1, 2, 3, 4]),
        scaling: identity(),
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    w.write_image(&image).unwrap();
    let mut bytes = w.into_inner().into_inner();
    let end = bytes[..BLOCK_SIZE]
        .chunks_exact(CARD_SIZE)
        .position(|card| &card[..3] == b"END")
        .unwrap();
    bytes[(end + 1) * CARD_SIZE] ^= 1;

    let mut r = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        r.verify_checksum(0).unwrap(),
        ChecksumReport {
            datasum_ok: Some(true),
            checksum_ok: Some(false),
        }
    );
}

#[test]
fn verify_is_none_when_checksum_keywords_are_absent() {
    let image = Image {
        shape: vec![2, 2],
        samples: ImageData::U8(vec![0, 0, 0, 0]),
        scaling: identity(),
    };
    let mut r = FitsReader::open(Cursor::new(write_to_vec(&image))).unwrap();
    let report = r.verify_checksum(0).unwrap();
    assert_eq!(report.datasum_ok, None);
    assert_eq!(report.checksum_ok, None);
}

#[test]
fn written_file_reads_back_with_matching_boundaries() {
    let header = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    1",
        "NAXIS1  =                   10",
    ]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&header, &[0u8; 10]).unwrap();
    let bytes = writer.into_inner().into_inner();

    // Header block + one padded data block.
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE);

    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus.len(), 1);
    assert_eq!(f.hdus[0].data_offset, BLOCK_SIZE as u64);
    assert_eq!(padded_len(f.hdus[0].data_bytes), BLOCK_SIZE as u64);
    assert_eq!(f.hdus[0].header.axes().unwrap(), vec![10]);
}

#[test]
fn raw_hdu_validates_the_complete_unit_before_output() {
    let header = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    1",
        "NAXIS1  =                    2",
    ]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_raw_hdu(&header, &[0, 1]),
        Err(FitsError::DataSizeMismatch {
            expected: 4,
            got: 2
        })
    ));
    assert_eq!(writer.state, WriterState::Empty);
    assert!(writer.into_inner().into_inner().is_empty());
}

#[test]
fn vla_descriptor_q_form_carries_full_64_bit_count_and_offset() {
    // A `Q` (wide) descriptor must not truncate count/offset to 32 bits — that is
    // the whole reason to choose `Q` over `P` (heaps/counts beyond i32::MAX).
    let count = u32::MAX as u64 + 5; // does not fit in u32
    let offset = 0x3_0000_0002u64;
    let mut q = vec![0; 16];
    write_pq_descriptor(&mut q, true, count, offset).unwrap();
    assert_eq!(q.len(), 16);
    assert_eq!(
        i64::from_be_bytes(q[0..8].try_into().unwrap()),
        count as i64
    );
    assert_eq!(
        i64::from_be_bytes(q[8..16].try_into().unwrap()),
        offset as i64
    );

    // The 32-bit `P` form packs two i32s.
    let mut p = vec![0; 8];
    write_pq_descriptor(&mut p, false, 7, 40).unwrap();
    assert_eq!(p.len(), 8);
    assert_eq!(i32::from_be_bytes(p[0..4].try_into().unwrap()), 7);
    assert_eq!(i32::from_be_bytes(p[4..8].try_into().unwrap()), 40);

    let mut rejected = [0; 8];
    assert!(matches!(
        write_pq_descriptor(&mut rejected, false, i32::MAX as u64 + 1, 0),
        Err(FitsError::DataUnitOverflow)
    ));
    let mut rejected = [0; 16];
    assert!(matches!(
        write_pq_descriptor(&mut rejected, true, i64::MAX as u64 + 1, 0),
        Err(FitsError::DataUnitOverflow)
    ));
}

#[derive(Debug)]
struct ImageBlankBoundary {
    samples: ImageData,
    valid: &'static [i64],
    invalid: &'static [i64],
}

#[test]
fn image_blank_is_type_and_range_checked_before_output() {
    let boundaries = [
        ImageBlankBoundary {
            samples: ImageData::U8(vec![0]),
            valid: &[0, u8::MAX as i64],
            invalid: &[-1, u8::MAX as i64 + 1],
        },
        ImageBlankBoundary {
            samples: ImageData::I16(vec![0]),
            valid: &[i16::MIN as i64, i16::MAX as i64],
            invalid: &[i16::MIN as i64 - 1, i16::MAX as i64 + 1],
        },
        ImageBlankBoundary {
            samples: ImageData::I32(vec![0]),
            valid: &[i32::MIN as i64, i32::MAX as i64],
            invalid: &[i32::MIN as i64 - 1, i32::MAX as i64 + 1],
        },
        ImageBlankBoundary {
            samples: ImageData::I64(vec![0]),
            valid: &[i64::MIN, i64::MAX],
            invalid: &[],
        },
        ImageBlankBoundary {
            samples: ImageData::F32(vec![0.0]),
            valid: &[],
            invalid: &[0],
        },
        ImageBlankBoundary {
            samples: ImageData::F64(vec![0.0]),
            valid: &[],
            invalid: &[0],
        },
    ];
    for boundary in boundaries {
        for &blank in boundary.valid {
            let image = Image {
                shape: vec![1],
                samples: boundary.samples.clone(),
                scaling: Scaling {
                    blank: Some(blank),
                    ..identity()
                },
            };
            let mut reader = FitsReader::open(Cursor::new(write_to_vec(&image))).unwrap();
            assert_eq!(
                reader.hdus[0].header.get_integer("BLANK").unwrap(),
                Some(blank)
            );
            assert_eq!(reader.read_image(0).unwrap().scaling.blank, Some(blank));
        }
        for &blank in boundary.invalid {
            let image = Image {
                shape: vec![1],
                samples: boundary.samples.clone(),
                scaling: Scaling {
                    blank: Some(blank),
                    ..identity()
                },
            };
            let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
            assert!(matches!(
                writer.write_image(&image),
                Err(FitsError::KeywordOutOfRange { name: "BLANK" })
            ));
            assert!(writer.into_inner().into_inner().is_empty());
        }
    }
}

#[derive(Debug)]
struct NonfiniteImageScale {
    keyword: &'static str,
    bscale: f64,
    bzero: f64,
}

#[test]
fn image_nonfinite_scaling_is_rejected_before_output() {
    let cases = [
        NonfiniteImageScale {
            keyword: "BSCALE",
            bscale: f64::NAN,
            bzero: 0.0,
        },
        NonfiniteImageScale {
            keyword: "BSCALE",
            bscale: f64::INFINITY,
            bzero: 0.0,
        },
        NonfiniteImageScale {
            keyword: "BZERO",
            bscale: 1.0,
            bzero: f64::NAN,
        },
        NonfiniteImageScale {
            keyword: "BZERO",
            bscale: 1.0,
            bzero: f64::NEG_INFINITY,
        },
    ];
    for case in cases {
        let image = Image {
            shape: vec![1],
            samples: ImageData::U8(vec![0]),
            scaling: Scaling {
                bscale: case.bscale,
                bzero: case.bzero,
                blank: None,
            },
        };
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        assert!(matches!(
            writer.write_image(&image),
            Err(FitsError::KeywordOutOfRange { name }) if name == case.keyword
        ));
        assert!(writer.into_inner().into_inner().is_empty());
    }
}

#[cfg(feature = "compression")]
#[test]
fn compressed_image_metadata_is_validated_before_automatic_primary() {
    let image = Image {
        shape: vec![1],
        samples: ImageData::F32(vec![0.0]),
        scaling: Scaling {
            blank: Some(0),
            ..identity()
        },
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_image(&image, "GZIP_1", &CompressOptions::default()),
        Err(FitsError::KeywordOutOfRange { name: "BLANK" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let image = Image {
        shape: vec![1],
        samples: ImageData::I64(vec![1]),
        scaling: identity(),
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_image(&image, "RICE_1", &CompressOptions::default()),
        Err(FitsError::UnsupportedCompression { .. })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
}

#[test]
fn logical_column_round_trips_with_null_state() {
    // §7.3.3: `L` is three-state — `T`/`F`/`0x00` (null); the null must survive.
    let columns = vec![WriteColumn::fixed(
        "FLAG",
        ColumnData::Logical(vec![Some(true), None, Some(false)]),
        1,
    )];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(3, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    assert_eq!(
        r.read_table(1)
            .unwrap()
            .column_by_idx(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::Logical(vec![Some(true), None, Some(false)])
    );
}
