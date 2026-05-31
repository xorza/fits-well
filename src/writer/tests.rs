use super::*;
use crate::block::ZERO_FILL;
use crate::data::{ImageData, Scaling, UnsignedView};
use crate::hdu::HduKind;
use crate::header::from_card_lines as header;
use crate::reader::FitsReader;
use crate::table::ColumnData;
use std::io::Cursor;

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
    w.write_image(&ext).unwrap(); // second image ⇒ IMAGE extension
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus.len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[1].kind, HduKind::Image);
    assert_eq!(
        r.read_image(0).unwrap().samples,
        ImageData::U8(vec![1, 2, 3, 4])
    );
    assert_eq!(
        r.read_image(1).unwrap().samples,
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
        WriteColumn::vla("DATA", vla_rows.clone()),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(3, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let table = r.read_table(1).unwrap();
    // TFORM2 should be a P descriptor sized to the longest row (5).
    assert_eq!(table.columns[1].tform.kind.code(), 'P');
    let got = table.read_vla_column(1).unwrap();
    assert_eq!(got.len(), 3);
    for (g, want) in got.iter().zip(&vla_rows) {
        match (g, want) {
            (ColumnData::I32(a), ColumnData::I32(b)) => assert_eq!(a, b),
            _ => panic!("expected I32 VLA cell, got {g:?}"),
        }
    }
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
            vec![ColumnData::I16(vec![7, 8, 9]), ColumnData::I16(vec![1])],
        )
        .wide(),
        // 12-bit X column: 2 bytes/row.
        WriteColumn::bits("FLAGS", ColumnData::Bytes(vec![0xAB, 0xC0, 0x12, 0x30]), 12),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(2, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let t = r.read_table(1).unwrap();

    // TDIM parsed back as a shape.
    assert_eq!(t.columns[0].tdim, Some(vec![2, 2]));
    // Q descriptor type, and the VLA reads back.
    assert_eq!(t.columns[1].tform.kind, TformKind::ArrayDesc64);
    match &t.read_vla_column(1).unwrap()[0] {
        ColumnData::I16(v) => assert_eq!(v, &[7, 8, 9]),
        other => panic!("{other:?}"),
    }
    // X column: TFORM 12X, packed bytes preserved.
    assert_eq!(t.columns[2].tform.kind, TformKind::Bit);
    assert_eq!(t.columns[2].tform.repeat, 12);
    match t.read_column(2).unwrap() {
        ColumnData::Bytes(b) => assert_eq!(b, vec![0xAB, 0xC0, 0x12, 0x30]),
        other => panic!("{other:?}"),
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
    assert_eq!(t.read_column(0).unwrap(), ColumnData::I32(vec![1, 2, 3]));
    assert_eq!(
        t.read_column(1).unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
    );
    assert_eq!(
        t.read_column(2).unwrap(),
        ColumnData::Text(vec!["AB".into(), "CDE".into(), "F".into()])
    );
}

#[test]
fn pad_to_block_rounds_up_with_the_fill_byte() {
    let mut empty = Vec::new();
    pad_to_block(&mut empty, ZERO_FILL);
    assert_eq!(empty.len(), 0);

    let mut one = vec![1u8];
    pad_to_block(&mut one, ZERO_FILL);
    assert_eq!(one.len(), BLOCK_SIZE);
    assert_eq!(one[0], 1);
    assert!(one[1..].iter().all(|&b| b == ZERO_FILL));

    let mut exact = vec![7u8; BLOCK_SIZE];
    pad_to_block(&mut exact, ZERO_FILL);
    assert_eq!(exact.len(), BLOCK_SIZE);

    let mut over = vec![0u8; BLOCK_SIZE + 1];
    pad_to_block(&mut over, ZERO_FILL);
    assert_eq!(over.len(), 2 * BLOCK_SIZE);
}

#[test]
fn rendered_header_is_block_aligned_and_ends_in_end_then_spaces() {
    let unit = render_header(&header(&[
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
    let reparsed = Header::parse(&render_header(&original)).unwrap();
    assert_eq!(reparsed.cards, original.cards);
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
    assert_eq!(back.samples, ImageData::I16(vec![1, -2, 3, -4, 5, -6]));
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
    assert_eq!(r.hdus[0].header.get_real("BZERO"), Some(32768.0));
    assert_eq!(r.hdus[0].header.get_real("BSCALE"), Some(1.0));
    let back = r.read_image(0).unwrap();
    assert_eq!(back.samples, ImageData::I16(vec![-32768, 0, 32767]));
    assert_eq!(back.physical(), vec![0.0, 32768.0, 65535.0]);

    // The `from_u16` helper produces the identical storage and round-trips back to
    // the exact `u16` values via the typed `unsigned()` view.
    let built = Image::from_u16(vec![3], &[0, 32768, 65535]);
    assert_eq!(built.samples, ImageData::I16(vec![-32768, 0, 32767]));
    let mut r2 = FitsReader::open(Cursor::new(write_to_vec(&built))).unwrap();
    assert_eq!(
        r2.read_image(0).unwrap().unsigned(),
        Some(UnsignedView::U16(vec![0, 32768, 65535]))
    );
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
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let report = r.verify_checksum(0).unwrap();
    assert_eq!(report.datasum_ok, Some(true));
    assert_eq!(report.checksum_ok, Some(true)); // whole-HDU sum is −0
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
    assert_eq!(f.hdus[0].data_len, BLOCK_SIZE as u64);
    assert_eq!(f.hdus[0].header.axes().unwrap(), vec![10]);
}
