use crate::compress::*;
use crate::endian::write_pq_descriptor;
use crate::error::FitsError;
use crate::header::Header;
use crate::header::value::Value;
use crate::keyword::key;
use crate::reader::FitsReader;
use crate::reader::internals::open_fixture;
use crate::table_impl::BinTable;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::descriptor;
use crate::table_impl::tform_kind::TformKind;
use crate::writer::FitsWriter;
use crate::writer::render_header;
use crate::writer::table::{TableBuilder, WriteColumn};
use std::fs::File;
use std::io::Cursor;

fn check_table_roundtrip(compression: Compression, rows_per_tile: usize) {
    let nrows = 10;
    let col = |name: &str, data, repeat| WriteColumn::fixed(name, data, repeat);
    let columns = vec![
        col(
            "SHORT",
            ColumnData::I16((0..nrows).map(|i| i as i16 * 7 - 30).collect()),
            1,
        ),
        col(
            "INT",
            ColumnData::I32((0..nrows).map(|i| (i as i32) * 100_000 - 5).collect()),
            1,
        ),
        col(
            "FLT",
            ColumnData::F32((0..nrows).map(|i| i as f32 * 1.5 - 3.25).collect()),
            1,
        ),
        col(
            "DBL",
            ColumnData::F64((0..nrows).map(|i| i as f64 * 0.1).collect()),
            1,
        ),
        col(
            "BYTE",
            ColumnData::Bytes((0..nrows).map(|i| (i * 3) as u8).collect()),
            1,
        ),
        // A multi-element (repeat=3) short column.
        col(
            "VEC",
            ColumnData::I16((0..nrows * 3).map(|i| (i * 2) as i16).collect()),
            3,
        ),
    ];
    let algo = compression.name();

    // 1. Write an uncompressed table and read it back.
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let source = TableBuilder::explicit(nrows, columns.clone()).unwrap();
    w.write_table(&source).unwrap();
    let bytes = w.into_inner().into_inner();
    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    let orig = r.read_table(1).unwrap();
    let orig_header = r.hdus[1].header.clone();

    // 2. Compress it, then read + uncompress.
    let mut cw = FitsWriter::new(Cursor::new(Vec::new()));
    cw.write_compressed_table(&orig_header, &orig, rows_per_tile, compression)
        .unwrap();
    let cbytes = cw.into_inner().into_inner();
    let mut cr = FitsReader::open(Cursor::new(cbytes)).unwrap();
    let compressed = cr.read_table(1).unwrap();
    let mut compressed_header = cr.hdus[1].header.clone();
    compressed_header.set_internal("ZFORM01", "preserve");
    let table::HduParts {
        header: restored_header,
        data: restored_data,
    } = table::uncompress_table(&compressed_header, &compressed).unwrap();
    for n in 1..=columns.len() {
        assert_eq!(restored_header.get(key!("ZFORM{n}").as_str()), None);
        assert_eq!(restored_header.get(key!("ZCTYP{n}").as_str()), None);
    }
    assert_eq!(
        restored_header.get_text("ZFORM01").unwrap(),
        Some("preserve")
    );
    for keyword in [
        "ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT", "ZHEAPPTR",
    ] {
        assert_eq!(restored_header.get(keyword), None);
    }
    let restored = BinTable::from_data(&restored_header, restored_data).unwrap();

    // 3. The uncompressed table must be byte-identical to the original.
    assert_eq!(
        restored.metadata().nrows,
        orig.metadata().nrows,
        "{algo}/{rows_per_tile} nrows"
    );
    assert_eq!(
        restored.schema.row_len, orig.schema.row_len,
        "{algo}/{rows_per_tile} row width"
    );
    assert_eq!(
        restored.raw_rows().unwrap(),
        orig.raw_rows().unwrap(),
        "{algo}/{rows_per_tile} data mismatch"
    );
}

#[test]
fn table_compression_round_trips() {
    // One tile, several tiles, and a tile smaller than the table — across codecs.
    for &rpt in &[10usize, 4, 1] {
        check_table_roundtrip(Compression::GZIP, rpt);
        check_table_roundtrip(Compression::GZIP_SHUFFLED, rpt);
        check_table_roundtrip(Compression::Rice, rpt);
        check_table_roundtrip(Compression::None, rpt);
    }
}

/// Emit our `write_compressed_table` output for external (cfitsio) validation of
/// the *encode* direction. After running, verify with cfitsio:
///   `funpack -O .tmp/my_unpk.fits .tmp/my_ctable.fits`
/// then compare `.tmp/my_unpk.fits` against `comp_table_ref.fits` — they match.
/// Run with `cargo test --features compression -- --ignored emit_compressed_table`.
#[test]
#[ignore]
fn emit_compressed_table_for_funpack() {
    let src = std::fs::read("tests/data/fits/comp_table_ref.fits").unwrap();
    let mut r = FitsReader::open(Cursor::new(src)).unwrap();
    let table = r.read_table(1).unwrap();
    let header = r.hdus[1].header.clone();
    let mut w = FitsWriter::new(File::create(".tmp/my_ctable.fits").unwrap());
    w.write_compressed_table(&header, &table, 100, Compression::Rice)
        .unwrap();
}

#[test]
fn decodes_a_cfitsio_compressed_table() {
    // Ground truth: `comp_table_cfitsio.fits` was produced by cfitsio's `fpack
    // -tableonly` from `comp_table_ref.fits` (500 rows, 6 fixed-width columns).
    // fpack chose a real mix of per-column codecs — GZIP_2 (short/float/double),
    // RICE_1 (the int32 columns), GZIP_1 (byte) — so this exercises every decode
    // path against an independent implementation. Our uncompressed output must be
    // byte-identical to the original table.
    let restored = open_fixture("comp_table_cfitsio.fits")
        .read_compressed_table(1)
        .unwrap();
    let original = open_fixture("comp_table_ref.fits").read_table(1).unwrap();

    assert_eq!(restored.metadata().nrows, 500);
    assert_eq!(restored.metadata().nrows, original.metadata().nrows);
    assert_eq!(restored.schema.row_len, original.schema.row_len);
    assert_eq!(restored.metadata().columns.len(), 6);
    assert_eq!(
        restored.raw_rows().unwrap(),
        original.raw_rows().unwrap(),
        "decoded cfitsio-compressed table must match the original bytes"
    );
    // Spot-check a decoded value against the known formula (INT = i·100000 − 5).
    match original.column_by_idx(1).unwrap().raw().unwrap() {
        ColumnData::I32(v) => assert_eq!(v[3], 3 * 100_000 - 5),
        other => panic!("expected I32, got {other:?}"),
    }
}

#[test]
fn table_compression_rejects_metadata_mismatches() {
    let mut header = Header::new();
    header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 2)
        .set_internal("NAXIS2", 2)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1I");
    let rows = [10i16, -20]
        .into_iter()
        .flat_map(i16::to_be_bytes)
        .collect::<Vec<_>>();
    let table = BinTable::from_data(&header, rows.clone()).unwrap();

    let cases = [
        ("XTENSION", Value::from("IMAGE")),
        ("BITPIX", Value::from(16)),
        ("NAXIS", Value::from(1)),
        ("NAXIS1", Value::from(4)),
        ("NAXIS2", Value::from(3)),
        ("PCOUNT", Value::from(1)),
        ("GCOUNT", Value::from(2)),
        ("TFIELDS", Value::from(2)),
        ("TFORM1", Value::from("1J")),
        ("THEAP", Value::from(5)),
    ];
    for (keyword, value) in cases {
        let mut mismatched = header.clone();
        mismatched.set_internal(keyword, value);
        let mut out = vec![0xA5];
        assert!(matches!(
            table::compress_table(&mismatched, &table, 1, Compression::GZIP, &mut out),
            Err(FitsError::TableMetadataMismatch { name }) if name == keyword
        ));
        assert_eq!(out, [0xA5], "{keyword} failure mutated the output");
    }

    let mut reserved = header.clone();
    reserved.set_internal("ZTABLE", true);
    assert!(matches!(
        table::compress_table(&reserved, &table, 1, Compression::GZIP, &mut Vec::new()),
        Err(FitsError::TableMetadataMismatch { name }) if name == "ZTABLE"
    ));
}

#[test]
fn table_compression_restores_reserved_metadata_exactly() {
    let mut original_header = Header::new();
    original_header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 2)
        .set_internal("NAXIS2", 2)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1I")
        .set_internal("THEAP", 4)
        .comment_internal("THEAP", "original heap start")
        .set_internal("CHECKSUM", "0123456789ABCDEF")
        .comment_internal("CHECKSUM", "original HDU checksum")
        .set_internal("DATASUM", "123456789")
        .comment_internal("DATASUM", "original data checksum");
    let original_data = [10i16, -20]
        .into_iter()
        .flat_map(i16::to_be_bytes)
        .collect::<Vec<_>>();
    let original_table = BinTable::from_data(&original_header, original_data.clone()).unwrap();

    let mut compressed_data = Vec::new();
    let mut compressed_header = table::compress_table(
        &original_header,
        &original_table,
        1,
        Compression::GZIP,
        &mut compressed_data,
    )
    .unwrap();
    assert_eq!(compressed_header.get("THEAP"), None);
    assert_eq!(compressed_header.get("CHECKSUM"), None);
    assert_eq!(compressed_header.get("DATASUM"), None);
    assert_eq!(compressed_header.get_integer("ZTHEAP").unwrap(), Some(4));
    assert_eq!(
        compressed_header.get_text("ZHECKSUM").unwrap(),
        Some("0123456789ABCDEF")
    );
    assert_eq!(
        compressed_header.get_text("ZDATASUM").unwrap(),
        Some("123456789")
    );

    let compressed_main = compressed_header.get_integer("NAXIS1").unwrap().unwrap()
        * compressed_header.get_integer("NAXIS2").unwrap().unwrap();
    compressed_header
        .set_internal("THEAP", compressed_main)
        .comment_internal("THEAP", "compressed heap start")
        .set_internal("CHECKSUM", "FEDCBA9876543210")
        .comment_internal("CHECKSUM", "compressed HDU checksum")
        .set_internal("DATASUM", "987654321")
        .comment_internal("DATASUM", "compressed data checksum");
    let compressed_table = BinTable::from_data(&compressed_header, compressed_data).unwrap();
    let restored = table::uncompress_table(&compressed_header, &compressed_table).unwrap();

    let mut original_bytes = Vec::new();
    let mut restored_bytes = Vec::new();
    render_header(&original_header, &mut original_bytes).unwrap();
    render_header(&restored.header, &mut restored_bytes).unwrap();
    assert_eq!(restored_bytes, original_bytes);
    assert_eq!(restored.data, original_data);
}

#[test]
fn decodes_a_cfitsio_compressed_table_with_a_vla_column() {
    let mut f = open_fixture("comp_table_vla.fits");
    let table = f.read_compressed_table(1).unwrap();
    assert_eq!(
        table.metadata().columns[1].tform.kind,
        TformKind::ArrayDesc32
    );
    assert_eq!(
        table.metadata().columns[1].tform.vla_elem,
        Some(TformKind::I32)
    );
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I32((0..600).collect())
    );
    let arrays = table.column_by_idx(1).unwrap().vla().unwrap();
    assert_eq!(arrays.len(), 600);
    for (row, array) in arrays.into_iter().enumerate() {
        assert_eq!(
            array,
            ColumnData::I32((0..(row % 7) as i32).collect()),
            "row {row}"
        );
    }
}

#[test]
fn compressed_table_vla_round_trips_all_table_codecs() {
    let mut source = open_fixture("comp_table_vla.fits");
    let compressed = source.read_table(1).unwrap();
    let mut original_parts = table::uncompress_table(&source.hdus[1].header, &compressed).unwrap();
    assert_eq!(
        u32::from_be_bytes(original_parts.data[4..8].try_into().unwrap()),
        0
    );
    original_parts.data[8..12].copy_from_slice(&i32::MAX.to_be_bytes());
    let original =
        BinTable::from_data(&original_parts.header, original_parts.data.clone()).unwrap();

    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
        Compression::None,
    ] {
        let mut encoded = Vec::new();
        let encoded_header = table::compress_table(
            &original_parts.header,
            &original,
            127,
            compression,
            &mut encoded,
        )
        .unwrap();
        assert_eq!(
            encoded_header.get_integer("ZPCOUNT").unwrap(),
            original_parts.header.get_integer("PCOUNT").unwrap(),
            "{}",
            compression.name()
        );
        let encoded_table = BinTable::from_data(&encoded_header, encoded).unwrap();
        let restored = table::uncompress_table(&encoded_header, &encoded_table).unwrap();
        assert_eq!(restored.data, original_parts.data, "{}", compression.name());
    }
}

#[test]
fn compressed_table_decode_rejects_the_shared_malformed_pq_corpus() {
    for wide in [false, true] {
        let mut column = WriteColumn::vla("VLA", vec![ColumnData::Bytes(vec![7])]).unwrap();
        if wide {
            column = column.wide().unwrap();
        }
        let prefix = WriteColumn::scalar("PREFIX", ColumnData::Bytes(vec![3]));
        let builder = TableBuilder::explicit(1, vec![prefix, column]).unwrap();
        let mut source_writer = FitsWriter::new(Cursor::new(Vec::new()));
        source_writer.write_table(&builder).unwrap();
        let source_bytes = source_writer.into_inner().into_inner();
        let mut source = FitsReader::from_bytes(&source_bytes).unwrap();
        let original_header = source.hdus[1].header.clone();
        let original = source.read_table(1).unwrap();
        let mut encoded = Vec::new();
        let compressed_header = table::compress_table(
            &original_header,
            &original,
            1,
            Compression::GZIP,
            &mut encoded,
        )
        .unwrap();
        let outer = descriptor::PqDescriptor::decode(&encoded[16..32], true).unwrap();
        let stream_range = outer
            .heap_range(TformKind::Byte, 32, encoded.len())
            .unwrap();
        let heap_prefix = encoded[32..stream_range.start].to_vec();
        let combined_len = 16 + if wide { 16 } else { 8 };
        let mut combined = Vec::new();
        gzip::gunzip_into(&encoded[stream_range], combined_len, &mut combined).unwrap();

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.wide == wide)
        {
            let mut malformed = combined.clone();
            malformed[16..16 + case.bytes.len()].copy_from_slice(&case.bytes);
            let cell = gzip::gzip_encode(&malformed, gzip::DEFAULT_GZIP_LEVEL);
            let mut data = encoded[..32].to_vec();
            data.extend_from_slice(&heap_prefix);
            write_pq_descriptor(
                &mut data[16..32],
                true,
                cell.len() as u64,
                heap_prefix.len() as u64,
            )
            .unwrap();
            data.extend_from_slice(&cell);
            let mut header = compressed_header.clone();
            header.set_internal("PCOUNT", (heap_prefix.len() + cell.len()) as i64);

            let mut primary = Header::new();
            primary
                .set_internal("SIMPLE", true)
                .set_internal("BITPIX", 8)
                .set_internal("NAXIS", 0);
            let mut file = FitsWriter::new(Cursor::new(Vec::new()));
            file.write_raw_hdu(&primary, &[]).unwrap();
            file.write_raw_hdu(&header, &data).unwrap();
            let bytes = file.into_inner().into_inner();
            let mut reader = FitsReader::from_bytes(&bytes).unwrap();
            let error = reader.read_compressed_table(1).unwrap_err();
            case.assert_error(error);
        }
    }
}

#[test]
fn read_compressed_table_rejects_a_plain_bintable() {
    let mut f = open_fixture("DDTSUVDATA.fits");
    assert!(matches!(
        f.read_compressed_table(1),
        Err(FitsError::NotCompressedTable)
    ));
}

#[test]
fn uncompress_table_rejects_overflowing_row_product() {
    // ZNAXIS2·ZNAXIS1 = 3e18·8 = 2.4e19 overflows usize; uncompress must reject the
    // header before allocating the row buffer (R2-3).
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 16) // one 1QB descriptor row
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1QB")
        .set_internal("TTYPE1", "C1")
        .set_internal("ZTABLE", true)
        .set_internal("ZTILELEN", 1)
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZNAXIS2", 3_000_000_000_000_000_000i64)
        .set_internal("ZPCOUNT", 0)
        .set_internal("ZFORM1", "1K");
    let mut data = Vec::new();
    data.extend_from_slice(&0i64.to_be_bytes()); // Q descriptor: nelem
    data.extend_from_slice(&0i64.to_be_bytes()); // offset
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        table::uncompress_table(&h, &table),
        Err(FitsError::DataUnitOverflow)
    ));

    h.set_internal("ZNAXIS1", 8).set_internal("ZNAXIS2", 2);
    assert!(matches!(
        table::uncompress_table(&h, &table),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));

    for keyword in ["ZNAXIS1", "ZNAXIS2", "ZTILELEN", "TFIELDS"] {
        h.set_internal(keyword, -1);
        assert!(matches!(
            table::uncompress_table(&h, &table),
            Err(FitsError::KeywordOutOfRange { name }) if name == keyword
        ));
        h.set_internal(keyword, 1);
    }

    // ZTILELEN alone also rejects zero: unlike the other three (where zero is a
    // legitimate empty count), a zero tile height would make the row-tile count
    // diverge rather than merely being out of range.
    h.set_internal("ZTILELEN", 0);
    assert!(matches!(
        table::uncompress_table(&h, &table),
        Err(FitsError::KeywordOutOfRange { name: "ZTILELEN" })
    ));
    h.set_internal("ZTILELEN", 1);

    h.set_internal("TFIELDS", 2)
        .set_internal("NAXIS1", 32)
        .set_internal("TFORM2", "1QB")
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZFORM1", format!("{}K", usize::MAX))
        .set_internal("ZFORM2", "1K");
    let two_column_table = BinTable::from_data(&h, vec![0; 32]).unwrap();
    assert!(matches!(
        table::uncompress_table(&h, &two_column_table),
        Err(FitsError::DataUnitOverflow)
    ));
}
