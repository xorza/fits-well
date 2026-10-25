//! The caller's header template: its informational cards kept, its structural ones
//! regenerated, on every write.

use crate::fits_writer::tests::*;

#[test]
fn typed_image_header_template_preserves_information_and_regenerates_structure() {
    let image = Image::new(vec![2], Data::I16(vec![10, 20]), Scaling::IDENTITY).unwrap();
    let template = informational_header_template();
    let file = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), Some(&template))
    });
    let mut reader = file.stream();
    let header = &reader.hdus[0].header;
    assert_informational_header(header);
    assert_eq!(header.axes().unwrap(), [2]);
    // The data unit is the one 32-bit word 0x000A0014 (i16 10 then 20), so its sum is
    // 10·65536 + 20 = 655380.
    cards::assert_cards(
        header,
        &[
            ("SIMPLE", true.into()),
            ("BITPIX", 16.into()),
            ("DATASUM", "655380".into()),
        ],
    );
    cards::assert_absent(header, &["XTENSION", "TFIELDS", "ZIMAGE"]);
    assert_ne!(header.get_text(keywords::CHECKSUM).unwrap(), Some("stale"));
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(
        memory::read_whole(&mut reader, 0).samples,
        Data::I16(vec![10, 20])
    );
}

#[test]
fn typed_table_header_templates_preserve_information_and_regenerate_structure() {
    let template = informational_header_template();
    let binary = [WriteColumn::fixed("VALUE", ColumnData::I32(vec![7, 8]), 1)];
    let ascii = [
        AsciiWriteColumn::text("TEXT", [Some("A"), Some("B")].into_iter().collect(), 3)
            .with_unit("label"),
    ];
    let file = memory::written(|w| {
        w.write_table(&memory::binary_table(2, &binary), Some(&template))?;
        w.write_ascii_table(&memory::ascii_table(2, &ascii), Some(&template))
    });
    let mut reader = file.stream();
    for index in [1, 2] {
        let header = &reader.hdus[index].header;
        assert_informational_header(header);
        cards::assert_cards(header, &[("NAXIS2", 2.into()), ("TFIELDS", 1.into())]);
        cards::assert_absent(header, &["CHECKSUM", "DATASUM"]);
    }
    assert_eq!(reader.hdus[1].kind, HduKind::BinTable);
    cards::assert_cards(
        &reader.hdus[1].header,
        &[("TFORM1", "1J".into()), ("TTYPE1", "VALUE".into())],
    );
    cards::assert_absent(&reader.hdus[1].header, &["TBCOL1"]);
    assert_eq!(reader.hdus[2].kind, HduKind::AsciiTable);
    cards::assert_cards(
        &reader.hdus[2].header,
        &[
            ("TFORM1", "A3".into()),
            ("TTYPE1", "TEXT".into()),
            ("TBCOL1", 1.into()),
        ],
    );
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::I32(vec![7, 8])
    );
    assert_eq!(
        reader
            .read_ascii_table(2)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        AsciiColumnData::Text([Some("A  "), Some("B  ")].into_iter().collect())
    );
}

#[test]
fn empty_primary_carries_the_template_only_as_the_first_hdu() {
    let template = informational_header_template();
    let table = memory::binary_table(1, &[WriteColumn::scalar("V", ColumnData::I16(vec![5]))]);
    let mut writer = memory::memory_writer().with_checksums();
    writer.write_empty_primary(Some(&template)).unwrap();
    writer.write_table(&table, None).unwrap();
    let written_len = writer.sink.get_ref().len();
    assert_matches!(
        writer.write_empty_primary(None),
        Err(FitsError::PrimaryAlreadyWritten)
    );
    assert_eq!(writer.sink.get_ref().len(), written_len);

    // The table follows the caller's primary, not an automatic second one.
    let mut reader = FitsReader::from_bytes(writer.sink.get_ref()).unwrap();
    assert_eq!(reader.hdus().len(), 2);
    let primary = &reader.hdus[0];
    assert_eq!(primary.kind, HduKind::Primary);
    assert_eq!(primary.header.naxis().unwrap(), 0);
    cards::assert_cards(
        &primary.header,
        &[
            ("SIMPLE", true.into()),
            ("EXTEND", true.into()),
            ("BITPIX", 8.into()),
        ],
    );
    assert_informational_header(&primary.header);
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(reader.hdus[1].kind, HduKind::BinTable);

    // An image written first is itself the primary.
    let image = Image::new(vec![1], vec![7u8], Scaling::IDENTITY).unwrap();
    let mut writer = memory::memory_writer();
    writer.write_image(image.view(), None).unwrap();
    assert_matches!(
        writer.write_empty_primary(None),
        Err(FitsError::PrimaryAlreadyWritten)
    );
}

#[cfg(feature = "compression")]
#[test]
fn compressed_header_templates_preserve_information_and_regenerate_structure() {
    let template = informational_header_template();
    let image = Image::new(vec![2], Data::I16(vec![10, 20]), Scaling::IDENTITY).unwrap();
    let mut writer = memory::memory_writer();
    writer
        .write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            Some(&template),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    let header = &reader.hdus[1].header;
    assert_informational_header(header);
    cards::assert_cards(
        header,
        &[
            ("ZIMAGE", true.into()),
            ("ZBITPIX", 16.into()),
            ("ZNAXIS", 1.into()),
            ("ZNAXIS1", 2.into()),
        ],
    );
    // The default tiling is one row: the whole 2-sample axis.
    assert_eq!(header.get_integer("ZTILE1").unwrap(), Some(2));
    cards::assert_absent(header, &["ZSIMPLE", "CHECKSUM"]);
    assert_eq!(
        memory::read_whole(&mut reader, 1).samples,
        Data::I16(vec![10, 20])
    );

    let columns = [WriteColumn::fixed(
        "VALUE",
        ColumnData::I32(vec![7, 8, 9]),
        1,
    )];
    let file = memory::written(|w| {
        w.write_compressed_table(
            &memory::binary_table(3, &columns),
            &CompressionConfig::new(Compression::GZIP).with_tiles([2]),
            Some(&template),
        )
    });
    let mut reader = file.stream();
    let header = &reader.hdus[1].header;
    assert_informational_header(header);
    cards::assert_cards(
        header,
        &[
            ("ZTABLE", true.into()),
            ("ZNAXIS2", 3.into()),
            ("ZFORM1", "1J".into()),
            ("TFORM1", "1QB".into()),
        ],
    );
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::I32(vec![7, 8, 9])
    );
}
