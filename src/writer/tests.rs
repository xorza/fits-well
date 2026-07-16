use crate::block::ZERO_FILL;
use crate::block::padded_len;
use crate::data::{ImageData, Scaling, UnsignedView};
use crate::hdu::HduKind;
use crate::header::from_card_lines as header;
use crate::reader::ChecksumReport;
use crate::reader::FitsReader;
use crate::table::ColumnData;
use crate::writer::*;
use std::io;
use std::io::Cursor;
use std::io::Write;

#[derive(Debug, Default)]
struct FailFirstWrite {
    bytes: Vec<u8>,
    failed: bool,
}

impl Write for FailFirstWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
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

fn rendered_header(header: &Header) -> Vec<u8> {
    let mut bytes = Vec::new();
    render_header(header, &mut bytes).unwrap();
    bytes
}

#[test]
fn writer_rejects_invalid_or_overflowing_layouts() {
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
        data: ColumnData::Text(Vec::new()),
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

    let invalid_tdim =
        WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4).with_tdim(vec![5]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[invalid_tdim]),
        Err(FitsError::KeywordOutOfRange { name: "TDIMn" })
    ));
    assert!(writer.into_inner().into_inner().is_empty());

    let invalid_text = WriteColumn::fixed("NAME", ColumnData::Text(vec!["Véga".to_string()]), 4);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_table(1, &[invalid_text]),
        Err(FitsError::InvalidAscii {
            context: "binary text cell"
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
fn writer_state_is_committed_only_after_an_hdu_write_succeeds() {
    let image = Image {
        shape: vec![1],
        samples: ImageData::U8(vec![7]),
        scaling: identity(),
    };
    let mut writer = FitsWriter::new(FailFirstWrite::default());
    assert!(matches!(writer.write_image(&image), Err(FitsError::Io(_))));
    assert!(!writer.has_primary);

    writer.write_image(&image).unwrap();
    let bytes = writer.into_inner().bytes;
    let reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.hdus.len(), 1);
    assert_eq!(reader.hdus[0].kind, HduKind::Primary);
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
fn writes_tdim_q_vla_and_bit_columns() {
    use crate::table::TformKind;
    let columns = vec![
        // 2×2 multidimensional column (TDIM '(2,2)'), 4 elements/row.
        WriteColumn::fixed("MAT", ColumnData::I32((1..=8).collect()), 4).with_tdim(vec![2, 2]),
        // 64-bit Q VLA column.
        WriteColumn::vla(
            "QV",
            ColumnType::I16,
            vec![ColumnData::I16(vec![7, 8, 9]), ColumnData::I16(vec![1])],
        )
        .wide(),
        WriteColumn::vla(
            "TXT",
            ColumnType::Text,
            vec![
                ColumnData::Text(vec!["hello".into()]),
                ColumnData::Text(vec!["a".into(), "bc".into()]),
            ],
        ),
        // 12-bit X column: 2 bytes/row.
        WriteColumn::bits("FLAGS", vec![0xAB, 0xC0, 0x12, 0x30], 12),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let t = r.read_table(1).unwrap();

    // TDIM parsed back as a shape.
    assert_eq!(t.columns[0].tdim, Some(vec![2, 2]));
    // Q descriptor type, and the VLA reads back.
    assert_eq!(t.columns[1].tform.kind, TformKind::ArrayDesc64);
    match &t.column_by_idx(1).unwrap().vla().unwrap()[0] {
        ColumnData::I16(v) => assert_eq!(v, &[7, 8, 9]),
        other => panic!("{other:?}"),
    }
    // X column: TFORM 12X, packed bytes preserved.
    assert_eq!(r.hdus[1].header.get_text("TFORM3").unwrap(), Some("1PA(5)"));
    assert_eq!(t.columns[2].tform.repeat, 1);
    assert_eq!(t.columns[2].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(t.columns[2].tform.vla_elem, Some(TformKind::Char));
    assert_eq!(
        t.column_by_idx(2).unwrap().vla().unwrap(),
        vec![
            ColumnData::Text(vec!["hello".into()]),
            ColumnData::Text(vec!["abc".into()])
        ]
    );
    assert_eq!(t.columns[3].tform.kind, TformKind::Bit);
    assert_eq!(t.columns[3].tform.repeat, 12);
    match t.column_by_idx(3).unwrap().raw().unwrap() {
        ColumnData::Bytes(b) => assert_eq!(b, vec![0xAB, 0xC0, 0x12, 0x30]),
        other => panic!("{other:?}"),
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
            ColumnData::Text(vec!["AB".into(), "CDE".into(), "F".into()]),
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
        ColumnData::Text(vec!["AB".into(), "CDE".into(), "F".into()])
    );
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
    invalid.set("OBJECT", "Véga");
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_header(&invalid),
        Err(FitsError::InvalidAscii {
            context: "header text value"
        })
    ));
    assert!(writer.into_inner().into_inner().is_empty());
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
    let built = Image::from_u16(vec![3], &[0, 32768, 65535]);
    let mut r = FitsReader::open(Cursor::new(write_to_vec(&built))).unwrap();
    assert_eq!(r.hdus[0].header.get_real("BZERO").unwrap(), Some(32768.0));
    assert_eq!(
        r.read_image(0).unwrap().unsigned(),
        Some(UnsignedView::U16(vec![0, 32768, 65535]))
    );
}

#[test]
fn from_u64_writes_the_exact_offset_and_round_trips_extremes() {
    let built = Image::from_u64(vec![2], &[u64::MIN, u64::MAX]);
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
    writer.write_header(&header).unwrap();
    writer.write_data_unit(&[0u8; 10], ZERO_FILL).unwrap();
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

#[test]
fn blank_is_emitted_only_for_integer_images() {
    // §4.4.2.5: BLANK applies only to integer (positive-BITPIX) images.
    let int_img = Image {
        shape: vec![2],
        samples: ImageData::I16(vec![1, 2]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: Some(-32768),
        },
    };
    let mut h = Header::new();
    int_img
        .scaling
        .add_to_header(&mut h, int_img.samples.bitpix());
    assert_eq!(h.get_integer("BLANK").unwrap(), Some(-32768));

    let float_img = Image {
        shape: vec![2],
        samples: ImageData::F32(vec![1.0, 2.0]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: Some(-32768),
        },
    };
    let mut h2 = Header::new();
    float_img
        .scaling
        .add_to_header(&mut h2, float_img.samples.bitpix());
    assert_eq!(h2.get_integer("BLANK").unwrap(), None);
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
