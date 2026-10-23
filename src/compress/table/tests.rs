use bitvec::order::Msb0;
use bitvec::vec::BitVec;

use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::internals::table_header;
use crate::bintable::tform_kind::TformKind;
use crate::compress::table::internals::mixed_columns;
use crate::compress::*;
use crate::endian::write_pq_descriptor;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::fits_writer::internals::round_trip;
use crate::fits_writer::render_header;
use crate::keyword::key;
use crate::ragged::Ragged;
use crate::reader::internals::open_fixture;
use crate::reader::{ColumnSelector, FitsReader};
use std::io::Cursor;

fn check_table_roundtrip(compression: Compression, rows_per_tile: usize) {
    let nrows = 10;
    let columns = mixed_columns(nrows);
    let algo = compression.name();

    // 1. Write an uncompressed table and read it back.
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let source = Builder::explicit(nrows, columns.clone()).unwrap();
    w.write_table(&source, None).unwrap();
    let bytes = w.into_inner().into_inner();
    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    let orig = r.read_table(1).unwrap();

    // 2. Compress the same builder, then read + uncompress.
    let mut cw = FitsWriter::new(Cursor::new(Vec::new()));
    let config = CompressionConfig::new(compression).tiled([rows_per_tile]);
    cw.write_compressed_table(&source, &config, None).unwrap();
    let cbytes = cw.into_inner().into_inner();
    let mut cr = FitsReader::open(Cursor::new(cbytes)).unwrap();
    let compressed = cr.read_container_table(1).unwrap();
    let mut compressed_header = cr.hdus[1].header.clone();
    compressed_header.set_internal("ZFORM01", "preserve");
    let table::HduParts {
        header: restored_header,
        data: restored_data,
    } = table::uncompress_table(&compressed_header, compressed.view()).unwrap();
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
    let restored = Table::from_data(&restored_header, restored_data).unwrap();

    // 3. The uncompressed table must be byte-identical to the original.
    assert_eq!(
        restored.schema().nrows,
        orig.schema().nrows,
        "{algo}/{rows_per_tile} nrows"
    );
    assert_eq!(
        restored.schema.row_len, orig.schema.row_len,
        "{algo}/{rows_per_tile} row width"
    );
    assert_eq!(
        restored.view().raw_rows(),
        orig.view().raw_rows(),
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

    assert_eq!(restored.schema().nrows, 500);
    assert_eq!(restored.schema().nrows, original.schema().nrows);
    assert_eq!(restored.schema.row_len, original.schema.row_len);
    assert_eq!(restored.schema().columns.len(), 6);
    assert_eq!(
        restored.view().raw_rows(),
        original.view().raw_rows(),
        "decoded cfitsio-compressed table must match the original bytes"
    );
    // Spot-check a decoded value against the known formula (INT = i·100000 − 5).
    match restored.column_by_idx(1).unwrap().raw().unwrap() {
        ColumnData::I32(v) => assert_eq!(v[3], 3 * 100_000 - 5),
        other => panic!("expected I32, got {other:?}"),
    }
}

/// The container header and data unit `write_compressed_table` writes of `table`.
fn compressed(table: &Builder, config: &CompressionConfig) -> table::HduParts {
    let mut reader = round_trip(|w| w.write_compressed_table(table, config, None));
    table::HduParts {
        header: reader.hdus[1].header.clone(),
        data: reader.read_data_raw(1).unwrap().into_data(),
    }
}

/// The header and data unit `write_table` writes of `table`.
fn plain(table: &Builder) -> table::HduParts {
    let mut reader = round_trip(|w| w.write_table(table, None));
    table::HduParts {
        header: reader.hdus[1].header.clone(),
        data: reader.read_data_raw(1).unwrap().into_data(),
    }
}

/// §10.3.1: a container keeps the table's `THEAP`, `CHECKSUM` and `DATASUM` as
/// `ZTHEAP`, `ZHECKSUM` and `ZDATASUM`, as cfitsio writes them, and decompression gives
/// them back under their own names, comments included, in place of the container's.
#[test]
fn table_decompression_restores_reserved_metadata_exactly() {
    let table = Builder::explicit(
        2,
        vec![WriteColumn::scalar("V", ColumnData::I16(vec![10, -20]))],
    )
    .unwrap();
    let table::HduParts {
        header: mut container_header,
        data: container_data,
    } = compressed(
        &table,
        &CompressionConfig::new(Compression::GZIP).tiled([1]),
    );
    container_header
        .set_internal("ZTHEAP", 4)
        .comment_internal("ZTHEAP", "original heap start")
        .set_internal("ZHECKSUM", "0123456789ABCDEF")
        .comment_internal("ZHECKSUM", "original HDU checksum")
        .set_internal("ZDATASUM", "123456789")
        .comment_internal("ZDATASUM", "original data checksum");
    let container_main = container_header.get_integer("NAXIS1").unwrap().unwrap()
        * container_header.get_integer("NAXIS2").unwrap().unwrap();
    container_header
        .set_internal("THEAP", container_main)
        .comment_internal("THEAP", "compressed heap start")
        .set_internal("CHECKSUM", "FEDCBA9876543210")
        .comment_internal("CHECKSUM", "compressed HDU checksum")
        .set_internal("DATASUM", "987654321")
        .comment_internal("DATASUM", "compressed data checksum");
    let container = Table::from_data(&container_header, container_data).unwrap();
    let restored = table::uncompress_table(&container_header, container.view()).unwrap();

    let mut original = plain(&table);
    original
        .header
        .set_internal("THEAP", 4)
        .comment_internal("THEAP", "original heap start")
        .set_internal("CHECKSUM", "0123456789ABCDEF")
        .comment_internal("CHECKSUM", "original HDU checksum")
        .set_internal("DATASUM", "123456789")
        .comment_internal("DATASUM", "original data checksum");
    let mut original_bytes = Vec::new();
    let mut restored_bytes = Vec::new();
    render_header(&original.header, &mut original_bytes).unwrap();
    render_header(&restored.header, &mut restored_bytes).unwrap();
    assert_eq!(restored_bytes, original_bytes);
    assert_eq!(restored.data, original.data);
}

#[test]
fn decodes_a_cfitsio_compressed_table_with_a_vla_column() {
    let mut f = open_fixture("comp_table_vla.fits");
    let table = f.read_compressed_table(1).unwrap();
    assert_eq!(table.schema().columns[1].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(
        table.schema().columns[1].tform.vla_elem,
        Some(TformKind::I32)
    );
    assert_eq!(
        table.column_by_idx(0).unwrap().raw().unwrap(),
        ColumnData::I32((0..600).collect())
    );
    let arrays = table.column_by_idx(1).unwrap().vla().unwrap();
    assert_eq!(arrays.len(), 600);
    let ColumnData::I32(values) = arrays.values() else {
        panic!("a J heap decodes to i32 values");
    };
    for row in 0..arrays.len() {
        assert_eq!(
            values[arrays.range(row)],
            (0..i32::try_from(row % 7).unwrap()).collect::<Vec<_>>(),
            "row {row}"
        );
    }
}

/// Each table codec restores the data unit `write_table` writes, its heap and every
/// array descriptor included: `P` and `Q` arrays of `J` values, `PX` bit arrays, empty
/// arrays, and a last tile shorter than the rest (600 rows in tiles of 127).
#[test]
fn compressed_table_vla_round_trips_all_table_codecs() {
    let rows = 600;
    let arrays = || {
        Ragged::from_rows(
            (0..rows).map(|row| ColumnData::I32((0..i32::try_from(row % 7).unwrap()).collect())),
        )
        .unwrap()
    };
    let bits = (0..rows)
        .map(|row| {
            (0..row % 11)
                .map(|bit| bit % 3 == 0)
                .collect::<BitVec<u8, Msb0>>()
        })
        .collect();
    let table = Builder::explicit(
        rows,
        vec![
            WriteColumn::scalar("ID", ColumnData::I32((0..600).collect())),
            WriteColumn::vla("NARROW", arrays()),
            WriteColumn::vla("WIDE", arrays()).wide(),
            WriteColumn::vla_bits("FLAGS", bits),
        ],
    )
    .unwrap();
    let original = plain(&table);

    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
        Compression::None,
    ] {
        let container = compressed(&table, &CompressionConfig::new(compression).tiled([127]));
        assert_eq!(
            container.header.get_integer("ZPCOUNT").unwrap(),
            original.header.get_integer("PCOUNT").unwrap(),
            "{}",
            compression.name()
        );
        let container_table = Table::from_data(&container.header, container.data).unwrap();
        let restored = table::uncompress_table(&container.header, container_table.view()).unwrap();
        assert_eq!(restored.data, original.data, "{}", compression.name());
    }
}

#[test]
fn compressed_table_decode_rejects_the_shared_malformed_pq_corpus() {
    for width in [DescriptorWidth::P, DescriptorWidth::Q] {
        let mut column = WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![ColumnData::Bytes(vec![7])]).unwrap(),
        );
        if width == DescriptorWidth::Q {
            column = column.wide();
        }
        let prefix = WriteColumn::scalar("PREFIX", ColumnData::Bytes(vec![3]));
        let builder = Builder::explicit(1, vec![prefix, column]).unwrap();
        let table::HduParts {
            header: compressed_header,
            data: encoded,
        } = compressed(
            &builder,
            &CompressionConfig::new(Compression::GZIP).tiled([1]),
        );
        let outer = PqDescriptor::decode(&encoded[16..32], DescriptorWidth::Q).unwrap();
        let stream_range = outer
            .heap_range(TformKind::Byte, 32, encoded.len())
            .unwrap();
        let heap_prefix = encoded[32..stream_range.start].to_vec();
        let combined_len = 16 + width.bytes();
        let mut combined = Vec::new();
        gzip::gunzip_into(&encoded[stream_range], combined_len, &mut combined).unwrap();

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.width == width)
        {
            let mut malformed = combined.clone();
            malformed[16..16 + case.bytes.len()].copy_from_slice(&case.bytes);
            let cell = gzip::gzip_encode(&malformed, gzip::DEFAULT_GZIP_LEVEL);
            let mut data = encoded[..32].to_vec();
            data.extend_from_slice(&heap_prefix);
            write_pq_descriptor(
                &mut data[16..32],
                DescriptorWidth::Q,
                cell.len() as u64,
                heap_prefix.len() as u64,
            )
            .unwrap();
            data.extend_from_slice(&cell);
            let mut header = compressed_header.clone();
            header.set_internal(
                "PCOUNT",
                i64::try_from(heap_prefix.len() + cell.len()).unwrap(),
            );

            let mut file = FitsWriter::new(Cursor::new(Vec::new()));
            file.write_empty_primary(None).unwrap();
            file.write_raw_hdu(&header, &data).unwrap();
            let bytes = file.into_inner().into_inner();
            let mut reader = FitsReader::from_bytes(&bytes).unwrap();
            let error = reader.read_compressed_table(1).unwrap_err();
            case.assert_error(&error);
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

/// The binary-table reads of a tiled-compressed table or image refuse it rather than
/// hand back the container's compressed cells.
#[test]
fn table_reads_refuse_a_compressed_container() {
    fn refused(fixture: &str, expected: fn(&FitsError) -> bool) {
        let mut f = open_fixture(fixture);
        let errors = [
            f.table_schema(1).map(|_| ()).unwrap_err(),
            f.hdus()[1].table_schema().map(|_| ()).unwrap_err(),
            f.read_table(1).map(|_| ()).unwrap_err(),
            f.read_table_rows(1, 0..1).map(|_| ()).unwrap_err(),
            f.read_table_columns(1, 0..1, &[ColumnSelector::Index(0)])
                .map(|_| ())
                .unwrap_err(),
            f.read_table_cell(1, 0, &ColumnSelector::Index(0))
                .map(|_| ())
                .unwrap_err(),
        ];
        for error in errors {
            assert!(expected(&error), "{fixture}: {error:?}");
        }
    }
    refused("comp_table_cfitsio.fits", |e| {
        matches!(e, FitsError::TableIsCompressed)
    });
    refused("comp_gzip_i16.fits", |e| {
        matches!(e, FitsError::NotABinTable)
    });
}

#[test]
fn uncompress_table_rejects_overflowing_row_product() {
    // ZNAXIS2·ZNAXIS1 = 3e18·8 = 2.4e19 overflows usize; uncompress must reject the
    // header before allocating the row buffer.
    // One 1QB descriptor row.
    let mut h = table_header(16, 1, &["1QB"]);
    h.set_internal("TTYPE1", "C1")
        .set_internal("ZTABLE", true)
        .set_internal("ZTILELEN", 1)
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZNAXIS2", 3_000_000_000_000_000_000i64)
        .set_internal("ZPCOUNT", 0)
        .set_internal("ZFORM1", "1K");
    let mut data = Vec::new();
    data.extend_from_slice(&0i64.to_be_bytes()); // Q descriptor: nelem
    data.extend_from_slice(&0i64.to_be_bytes()); // offset
    let table = Table::from_data(&h, data).unwrap();
    assert!(matches!(
        table::uncompress_table(&h, table.view()),
        Err(FitsError::DataUnitOverflow)
    ));

    h.set_internal("ZNAXIS1", 8).set_internal("ZNAXIS2", 2);
    assert!(matches!(
        table::uncompress_table(&h, table.view()),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));

    for keyword in ["ZNAXIS1", "ZNAXIS2", "ZTILELEN", "TFIELDS"] {
        h.set_internal(keyword, -1);
        assert!(matches!(
            table::uncompress_table(&h, table.view()),
            Err(FitsError::KeywordOutOfRange { name }) if name == keyword
        ));
        h.set_internal(keyword, 1);
    }

    // ZTILELEN alone also rejects zero: unlike the other three (where zero is a
    // legitimate empty count), a zero tile height would make the row-tile count
    // diverge rather than merely being out of range.
    h.set_internal("ZTILELEN", 0);
    assert!(matches!(
        table::uncompress_table(&h, table.view()),
        Err(FitsError::KeywordOutOfRange { name: "ZTILELEN" })
    ));
    h.set_internal("ZTILELEN", 1);

    h.set_internal("TFIELDS", 2)
        .set_internal("NAXIS1", 32)
        .set_internal("TFORM2", "1QB")
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZFORM1", format!("{}K", usize::MAX))
        .set_internal("ZFORM2", "1K");
    let two_column_table = Table::from_data(&h, vec![0; 32]).unwrap();
    assert!(matches!(
        table::uncompress_table(&h, two_column_table.view()),
        Err(FitsError::DataUnitOverflow)
    ));
}

/// Compresses `columns` as one table through the writer and reads them back through
/// `read_compressed_table`.
fn compressed_round_trip(
    columns: Vec<WriteColumn>,
    nrows: usize,
    compression: Compression,
) -> (Table, Table) {
    let table = Builder::explicit(nrows, columns).unwrap();
    let original = round_trip(|w| w.write_table(&table, None))
        .read_table(1)
        .unwrap();
    let mut cw = FitsWriter::new(Cursor::new(Vec::new()));
    let config = CompressionConfig::new(compression).tiled([nrows]);
    cw.write_compressed_table(&table, &config, None).unwrap();
    let mut cr = FitsReader::open(Cursor::new(cw.into_inner().into_inner())).unwrap();
    // §10.3.1: the copied value keywords describe the uncompressed columns, so the
    // container's own schema reads none of them.
    let container = cr.hdus[1].container_schema().unwrap();
    assert!(container.columns.iter().all(|column| column.tdim.is_none()
        && column.tscale == 1.0
        && column.tzero == 0.0
        && column.tnull.is_none()));
    (original, cr.read_compressed_table(1).unwrap())
}

/// §10.3.1: the copied `TDIMn` describes the uncompressed column, not the compressed
/// container cell. A constant `100J` column shaped `(10,10)` compresses to far fewer than
/// 100 elements and once failed the `TDIMn` check on read.
#[test]
fn a_shaped_column_reads_back_from_a_compressed_table() {
    for compression in [
        Compression::Rice,
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
    ] {
        let column = WriteColumn::fixed("CUBE", ColumnData::I32(vec![7; 2 * 100]), 100)
            .with_tdim(vec![10, 10]);
        let (original, restored) = compressed_round_trip(vec![column], 2, compression);
        assert_eq!(
            restored.column_by_idx(0).unwrap().raw().unwrap(),
            original.column_by_idx(0).unwrap().raw().unwrap(),
            "{}",
            compression.name()
        );
        assert_eq!(restored.schema().columns[0].tdim, Some(vec![10, 10]));
    }
}

/// A variable-length array is stored compressed when its codec shrinks it and as is
/// otherwise — cfitsio's rule, which makes a stored length equal to the raw length the
/// mark of a raw array. Pseudo-random bytes do not deflate; a run of zeros does.
#[test]
fn a_variable_length_array_is_stored_compressed_only_when_it_shrinks() {
    let mut state = 0x2545_f491_4f6c_dd1du64;
    let mut noise = |len: usize| -> Vec<u8> {
        (0..len)
            .map(|_| {
                state ^= state << 13;
                state ^= state >> 7;
                state ^= state << 17;
                state as u8
            })
            .collect()
    };
    let noise_row = noise(40);
    let rows = vec![
        ColumnData::Bytes(noise_row.clone()),
        ColumnData::Bytes(Vec::new()),
        ColumnData::Bytes(vec![0; 400]),
    ];
    let column = WriteColumn::vla("ARRAYS", Ragged::from_rows(rows.clone()).unwrap());
    let table::HduParts {
        header: encoded_header,
        data: encoded,
    } = compressed(
        &Builder::explicit(3, vec![column]).unwrap(),
        &CompressionConfig::new(Compression::GZIP).tiled([3]),
    );
    let encoded_table = Table::from_data(&encoded_header, encoded).unwrap();

    // The container's one cell gunzips to 3 compressed-array descriptors followed by the
    // 3 original ones.
    let cell = encoded_table
        .column_by_idx(0)
        .unwrap()
        .vla_column()
        .unwrap();
    let mut combined = Vec::new();
    gzip::gunzip_into(
        convert::byte_cell(cell.cell(0).unwrap()).unwrap(),
        3 * 16 + 3 * 8,
        &mut combined,
    )
    .unwrap();
    let stored = |row: usize| {
        let descriptor = &combined[row * 16..(row + 1) * 16];
        let descriptor = PqDescriptor {
            count: usize::try_from(u64::from_be_bytes(descriptor[..8].try_into().unwrap()))
                .unwrap(),
            offset: usize::try_from(u64::from_be_bytes(descriptor[8..].try_into().unwrap()))
                .unwrap(),
        };
        encoded_table
            .view()
            .pq_payload(descriptor, TformKind::Byte)
            .unwrap()
            .to_vec()
    };
    assert_eq!(stored(0), noise_row, "incompressible: stored as is");
    assert!(stored(1).is_empty(), "empty: nothing stored");
    let zeros = stored(2);
    assert!(
        zeros.len() < 400 && zeros[..2] == [0x1f, 0x8b],
        "compressible: a gzip member"
    );

    let restored = table::uncompress_table(&encoded_header, encoded_table.view()).unwrap();
    let restored = Table::from_data(&restored.header, restored.data).unwrap();
    assert_eq!(
        restored.column_by_idx(0).unwrap().vla().unwrap(),
        Ragged::from_rows(rows).unwrap()
    );
}

/// A declared `ZCTYPn` is honoured as written, so `RICE_1` on a 64-bit column —
/// which no conforming writer emits — is refused rather than read as `GZIP_2`.
#[test]
fn a_declared_rice_codec_on_a_64_bit_column_is_refused() {
    let column = WriteColumn::scalar("BIG", ColumnData::I64(vec![1, 2, 3]));
    let table = Builder::explicit(3, vec![column]).unwrap();
    let original = round_trip(|w| w.write_table(&table, None))
        .read_table(1)
        .unwrap();
    let mut cw = FitsWriter::new(Cursor::new(Vec::new()));
    cw.write_compressed_table(&table, &CompressionConfig::new(Compression::GZIP), None)
        .unwrap();
    let mut cr = FitsReader::open(Cursor::new(cw.into_inner().into_inner())).unwrap();
    assert_eq!(
        cr.hdus[1].header.get_text("ZCTYP1").unwrap(),
        Some("GZIP_1")
    );
    assert_eq!(
        cr.read_compressed_table(1).unwrap().view().raw_rows(),
        original.view().raw_rows()
    );

    let mut tampered = cr.hdus[1].header.clone();
    tampered.set_internal("ZCTYP1", "RICE_1");
    let data = cr.read_data_raw(1).unwrap().into_data();
    let mut file = FitsWriter::new(Cursor::new(Vec::new()));
    file.write_empty_primary(None).unwrap();
    file.write_raw_hdu(&tampered, &data).unwrap();
    let bytes = file.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        reader.read_compressed_table(1),
        Err(FitsError::UnsupportedCompression { name }) if name == "RICE_1 on a K column"
    ));
}
