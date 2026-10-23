use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use std::ops::Range;

use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::internals::table_header;
use crate::bintable::schema::Schema;
use crate::bintable::tform_kind::TformKind;
use crate::compress::table::internals::mixed_columns;
use crate::compress::*;
use crate::endian::write_pq_descriptor;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::fits_writer::internals::{round_trip, written};
use crate::fits_writer::render_header;
use crate::hdu::HduKind;
use crate::header_model::keywords;

use crate::column::ColumnSelector;
use crate::ragged::Ragged;
use crate::reader::FitsReader;
use crate::reader::internals::open_fixture;
use crate::reader::source::SliceSource;
use std::io::Cursor;

fn check_table_roundtrip(compression: Compression, rows_per_tile: usize) {
    let nrows = 10;
    let columns = mixed_columns(nrows);
    let algo = compression.name();

    // 1. Write an uncompressed table and read it back.
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let source = Builder::with_columns(nrows, columns.clone()).unwrap();
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
    let table::internals::HduParts {
        header: restored_header,
        data: restored_data,
    } = table::internals::uncompress_table(&compressed_header, compressed.view()).unwrap();
    for n in 1..=columns.len() {
        assert_eq!(
            restored_header
                .get(keywords::ZFORM.indexed(n).unwrap())
                .unwrap(),
            None
        );
        assert_eq!(
            restored_header
                .get(keywords::ZCTYP.indexed(n).unwrap())
                .unwrap(),
            None
        );
    }
    assert_eq!(
        restored_header.get_text("ZFORM01").unwrap(),
        Some("preserve")
    );
    for keyword in [
        "ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT", "ZHEAPPTR",
    ] {
        assert_eq!(restored_header.get(keyword).unwrap(), None);
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
        .read_table(1)
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
    match restored.column(1).unwrap().raw().unwrap() {
        ColumnData::I32(v) => assert_eq!(v[3], 3 * 100_000 - 5),
        other => panic!("expected I32, got {other:?}"),
    }
}

/// The container header and data unit `write_compressed_table` writes of `table`.
fn compressed(table: &Builder, config: &CompressionConfig) -> table::internals::HduParts {
    let mut reader = round_trip(|w| w.write_compressed_table(table, config, None));
    table::internals::HduParts {
        header: reader.hdus[1].header.clone(),
        data: reader.read_data_raw(1).unwrap().into_data(),
    }
}

/// The header and data unit `write_table` writes of `table`.
fn plain(table: &Builder) -> table::internals::HduParts {
    let mut reader = round_trip(|w| w.write_table(table, None));
    table::internals::HduParts {
        header: reader.hdus[1].header.clone(),
        data: reader.read_data_raw(1).unwrap().into_data(),
    }
}

/// §10.3.1: a container keeps the table's `THEAP`, `CHECKSUM` and `DATASUM` as
/// `ZTHEAP`, `ZHECKSUM` and `ZDATASUM`, as cfitsio writes them, and decompression gives
/// them back under their own names, comments included, in place of the container's.
#[test]
fn table_decompression_restores_reserved_metadata_exactly() {
    let table = Builder::with_columns(
        2,
        vec![WriteColumn::scalar("V", ColumnData::I16(vec![10, -20]))],
    )
    .unwrap();
    let table::internals::HduParts {
        header: mut container_header,
        data: container_data,
    } = compressed(
        &table,
        &CompressionConfig::new(Compression::GZIP).tiled([1]),
    );
    container_header
        .set_internal(keywords::ZTHEAP, 4)
        .set_comment_internal(keywords::ZTHEAP, "original heap start")
        .set_internal(keywords::ZHECKSUM, "0123456789ABCDEF")
        .set_comment_internal(keywords::ZHECKSUM, "original HDU checksum")
        .set_internal(keywords::ZDATASUM, "123456789")
        .set_comment_internal(keywords::ZDATASUM, "original data checksum");
    let container_main = container_header.get_integer("NAXIS1").unwrap().unwrap()
        * container_header.get_integer("NAXIS2").unwrap().unwrap();
    container_header
        .set_internal(keywords::THEAP, container_main)
        .set_comment_internal(keywords::THEAP, "compressed heap start")
        .set_internal(keywords::CHECKSUM, "FEDCBA9876543210")
        .set_comment_internal(keywords::CHECKSUM, "compressed HDU checksum")
        .set_internal(keywords::DATASUM, "987654321")
        .set_comment_internal(keywords::DATASUM, "compressed data checksum");
    let container = Table::from_data(&container_header, container_data).unwrap();
    let restored = table::internals::uncompress_table(&container_header, container.view()).unwrap();

    let mut original = plain(&table);
    original
        .header
        .set_internal(keywords::THEAP, 4)
        .set_comment_internal(keywords::THEAP, "original heap start")
        .set_internal(keywords::CHECKSUM, "0123456789ABCDEF")
        .set_comment_internal(keywords::CHECKSUM, "original HDU checksum")
        .set_internal(keywords::DATASUM, "123456789")
        .set_comment_internal(keywords::DATASUM, "original data checksum");
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
    let table = f.read_table(1).unwrap();
    assert_eq!(table.schema().columns[1].tform.kind, TformKind::ArrayDesc32);
    assert_eq!(
        table.schema().columns[1].tform.vla_elem,
        Some(TformKind::I32)
    );
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        ColumnData::I32((0..600).collect())
    );
    let arrays = table.column(1).unwrap().vla().unwrap();
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
    let table = Builder::with_columns(
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
            container.header.get_integer(keywords::ZPCOUNT).unwrap(),
            original.header.get_integer(keywords::PCOUNT).unwrap(),
            "{}",
            compression.name()
        );
        let container_table = Table::from_data(&container.header, container.data).unwrap();
        let restored =
            table::internals::uncompress_table(&container.header, container_table.view()).unwrap();
        assert_eq!(restored.data, original.data, "{}", compression.name());
    }
}

#[test]
fn compressed_table_decode_rejects_the_shared_malformed_pq_corpus() {
    for width in [DescriptorWidth::P, DescriptorWidth::Q] {
        let mut column = WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![ColumnData::U8(vec![7])]).unwrap(),
        );
        if width == DescriptorWidth::Q {
            column = column.wide();
        }
        let prefix = WriteColumn::scalar("PREFIX", ColumnData::U8(vec![3]));
        let builder = Builder::with_columns(1, vec![prefix, column]).unwrap();
        let table::internals::HduParts {
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
                keywords::PCOUNT,
                i64::try_from(heap_prefix.len() + cell.len()).unwrap(),
            );

            let mut file = FitsWriter::new(Cursor::new(Vec::new()));
            file.write_empty_primary(None).unwrap();
            file.write_raw_hdu(&header, &data).unwrap();
            let bytes = file.into_inner().into_inner();
            let mut reader = FitsReader::from_bytes(&bytes).unwrap();
            let error = reader.read_table(1).unwrap_err();
            case.assert_error(&error);
        }
    }
}

/// 50 rows in tiles of 7, the last tile 1 row: the mixed fixed-width columns and a
/// variable-length one, written plain and compressed, as the bytes of the two files.
fn tiled_pair() -> [Vec<u8>; 2] {
    let rows = 50;
    let mut columns = mixed_columns(rows);
    columns.push(WriteColumn::vla(
        "ARRAYS",
        Ragged::from_rows(
            (0..rows).map(|row| ColumnData::I32((0..i32::try_from(row % 5).unwrap()).collect())),
        )
        .unwrap(),
    ));
    let table = Builder::with_columns(rows, columns).unwrap();
    let plain = written(|w| w.write_table(&table, None));
    let compressed = written(|w| {
        w.write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::Rice).tiled([7]),
            None,
        )
    });
    [plain, compressed]
}

/// Every table read gives a compressed table as the table it restores: the same schema,
/// and the same rows, columns and cells as the plain file, for ranges inside one tile,
/// across tiles, at the last partial tile, and empty.
#[test]
fn table_reads_give_a_compressed_table_as_the_table_it_restores() {
    let [plain, compressed] = tiled_pair();
    let mut plain = FitsReader::from_bytes(&plain).unwrap();
    let mut compressed = FitsReader::from_bytes(&compressed).unwrap();
    assert_eq!(compressed.hdus()[1].kind, HduKind::CompressedTable);
    let schema = compressed.table_schema(1).unwrap();
    assert_eq!(schema.nrows, 50);
    assert_eq!(schema.row_len, plain.table_schema(1).unwrap().row_len);
    let tforms = |schema: &Schema| schema.columns.iter().map(|c| c.tform).collect::<Vec<_>>();
    assert_eq!(tforms(schema), tforms(plain.table_schema(1).unwrap()));

    let selected = |reader: &mut FitsReader<SliceSource<'_>>, rows: Range<usize>| {
        let selection = reader.read_table_columns(1, rows, 0..7).unwrap();
        selection
            .columns
            .into_iter()
            .map(|column| column.data)
            .collect::<Vec<_>>()
    };
    for rows in [3..5, 5..16, 0..50, 49..50, 43..50, 20..20] {
        assert_eq!(
            selected(&mut compressed, rows.clone()),
            selected(&mut plain, rows.clone()),
            "{rows:?}"
        );
        let [whole, part] = [
            plain.read_table_rows(1, rows.clone()),
            compressed.read_table_rows(1, rows.clone()),
        ]
        .map(Result::unwrap);
        assert_eq!(part.view().raw_rows(), whole.view().raw_rows(), "{rows:?}");
    }
    assert_eq!(
        compressed.read_table(1).unwrap().view().raw_rows(),
        plain.read_table(1).unwrap().view().raw_rows()
    );
    for (row, column) in [(0, 0), (8, 6), (49, 5), (13, 3)] {
        let column = ColumnSelector::Index(column);
        assert_eq!(
            compressed.read_table_cell(1, row, column).unwrap(),
            plain.read_table_cell(1, row, column).unwrap(),
            "row {row}"
        );
    }
    assert!(matches!(
        compressed.read_table_rows(1, 40..51),
        Err(FitsError::RowRangeOutOfBounds {
            start: 40,
            end: 51,
            len: 50
        })
    ));
    assert!(matches!(
        compressed.read_table_cell(1, 50, ColumnSelector::Index(0)),
        Err(FitsError::RowRangeOutOfBounds {
            start: 50,
            end: 51,
            len: 50
        })
    ));

    // cfitsio's own compressed table restores to the table it came from.
    let cfitsio = open_fixture("comp_table_cfitsio.fits")
        .read_table(1)
        .unwrap();
    let reference = open_fixture("comp_table_ref.fits").read_table(1).unwrap();
    assert_eq!(cfitsio.view().raw_rows(), reference.view().raw_rows());

    // A compressed image is no table.
    let mut image = open_fixture("comp_gzip_i16.fits");
    let errors = [
        image.table_schema(1).map(|_| ()).unwrap_err(),
        image.read_table(1).map(|_| ()).unwrap_err(),
        image.read_table_rows(1, 0..1).map(|_| ()).unwrap_err(),
        image
            .read_table_columns(1, 0..1, [ColumnSelector::Index(0)])
            .map(|_| ())
            .unwrap_err(),
        image
            .read_table_cell(1, 0, ColumnSelector::Index(0))
            .map(|_| ())
            .unwrap_err(),
    ];
    for error in errors {
        assert!(matches!(error, FitsError::NotABinTable), "{error:?}");
    }
}

/// A partial read decodes only the tiles its rows touch and the columns it selects: with
/// the last tile's last column corrupt, a read that touches neither succeeds, and the
/// reads that do fail.
#[test]
fn a_partial_read_decodes_only_the_tiles_and_columns_it_reads() {
    let [_, mut compressed] = tiled_pair();
    {
        let mut reader = FitsReader::from_bytes(&compressed).unwrap();
        let container = reader.read_container_table(1).unwrap();
        let hdu = &reader.hdus()[1];
        let tiled = hdu.tiled_table().unwrap().unwrap();
        let row_len = tiled.schema.row_len;
        let heap_len = tiled.schema.heap_end - 50 * row_len;
        assert!(heap_len > 0);
        // Tiles of 7 rows: row 9 lies in tile 1, rows 7..14; row 49 is the last tile,
        // alone; an empty range holds no tile. Each decode holds its rows and the heap.
        for (rows, first_row, row_count) in [
            (9..10, 7, 7),
            (5..16, 0, 21),
            (49..50, 49, 1),
            (20..20, 20, 0),
        ] {
            let decoded = tiled
                .decode(&hdu.header, container.view(), &rows, None)
                .unwrap();
            assert_eq!(decoded.first_row, first_row, "{rows:?}");
            assert_eq!(decoded.schema.nrows, row_count, "{rows:?}");
            assert_eq!(
                decoded.data.len(),
                row_count * row_len + heap_len,
                "{rows:?}"
            );
        }
        let whole = tiled
            .decode(&hdu.header, container.view(), &(0..50), None)
            .unwrap();
        assert_eq!(whole.schema.heap_offset, tiled.schema.heap_offset);
        assert_eq!(whole.data.len(), tiled.schema.heap_end);
    }
    let data_end = {
        let reader = FitsReader::from_bytes(&compressed).unwrap();
        let hdu = &reader.hdus()[1];
        usize::try_from(hdu.data_offset + hdu.data_bytes).unwrap()
    };
    // The heap ends with the last tile's last column, whose gzip trailer this breaks.
    compressed[data_end - 1] ^= 0xFF;
    let mut reader = FitsReader::from_bytes(&compressed).unwrap();
    assert!(reader.read_table_rows(1, 0..49).is_ok());
    assert!(
        reader
            .read_table_columns(1, 49..50, [ColumnSelector::Index(0)])
            .is_ok()
    );
    assert!(
        reader
            .read_table_cell(1, 49, ColumnSelector::Index(5))
            .is_ok()
    );
    assert!(reader.read_table_rows(1, 49..50).is_err());
    assert!(
        reader
            .read_table_columns(1, 49..50, [ColumnSelector::Index(6)])
            .is_err()
    );
    assert!(reader.read_table(1).is_err());
}

#[test]
fn uncompress_table_rejects_overflowing_row_product() {
    // ZNAXIS2·ZNAXIS1 = 3e18·8 = 2.4e19 overflows usize; uncompress must reject the
    // header before allocating the row buffer.
    // One 1QB descriptor row.
    let mut h = table_header(16, 1, &["1QB"]);
    h.set_internal("TTYPE1", "C1")
        .set_internal(keywords::ZTABLE, true)
        .set_internal(keywords::ZTILELEN, 1)
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZNAXIS2", 3_000_000_000_000_000_000i64)
        .set_internal(keywords::ZPCOUNT, 0)
        .set_internal("ZFORM1", "1K");
    let mut data = Vec::new();
    data.extend_from_slice(&0i64.to_be_bytes()); // Q descriptor: nelem
    data.extend_from_slice(&0i64.to_be_bytes()); // offset
    let table = Table::from_data(&h, data).unwrap();
    assert!(matches!(
        table::internals::uncompress_table(&h, table.view()),
        Err(FitsError::DataUnitOverflow)
    ));

    h.set_internal("ZNAXIS1", 8).set_internal("ZNAXIS2", 2);
    assert!(matches!(
        table::internals::uncompress_table(&h, table.view()),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));

    for keyword in ["ZNAXIS1", "ZNAXIS2", "ZTILELEN", "TFIELDS"] {
        h.set_internal(keyword, -1);
        assert!(matches!(
            table::internals::uncompress_table(&h, table.view()),
            Err(FitsError::KeywordOutOfRange { keyword }) if keyword == keyword
        ));
        h.set_internal(keyword, 1);
    }

    // ZTILELEN alone also rejects zero: unlike the other three (where zero is a
    // legitimate empty count), a zero tile height would make the row-tile count
    // diverge rather than merely being out of range.
    h.set_internal(keywords::ZTILELEN, 0);
    assert!(matches!(
        table::internals::uncompress_table(&h, table.view()),
        Err(FitsError::KeywordOutOfRange {
            keyword: keywords::ZTILELEN
        })
    ));
    h.set_internal(keywords::ZTILELEN, 1);

    h.set_internal(keywords::TFIELDS, 2)
        .set_internal("NAXIS1", 32)
        .set_internal("TFORM2", "1QB")
        .set_internal("ZNAXIS1", 8)
        .set_internal("ZFORM1", format!("{}K", usize::MAX))
        .set_internal("ZFORM2", "1K");
    let two_column_table = Table::from_data(&h, vec![0; 32]).unwrap();
    assert!(matches!(
        table::internals::uncompress_table(&h, two_column_table.view()),
        Err(FitsError::DataUnitOverflow)
    ));
}

/// Compresses `columns` as one table through the writer and reads them back through
/// `read_table`.
fn compressed_round_trip(
    columns: Vec<WriteColumn>,
    nrows: usize,
    compression: Compression,
) -> (Table, Table) {
    let table = Builder::with_columns(nrows, columns).unwrap();
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
    (original, cr.read_table(1).unwrap())
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
            restored.column(0).unwrap().raw().unwrap(),
            original.column(0).unwrap().raw().unwrap(),
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
        ColumnData::U8(noise_row.clone()),
        ColumnData::U8(Vec::new()),
        ColumnData::U8(vec![0; 400]),
    ];
    let column = WriteColumn::vla("ARRAYS", Ragged::from_rows(rows.clone()).unwrap());
    let table::internals::HduParts {
        header: encoded_header,
        data: encoded,
    } = compressed(
        &Builder::with_columns(3, vec![column]).unwrap(),
        &CompressionConfig::new(Compression::GZIP).tiled([3]),
    );
    let encoded_table = Table::from_data(&encoded_header, encoded).unwrap();

    // The container's one cell gunzips to 3 compressed-array descriptors followed by the
    // 3 original ones.
    let cell = encoded_table.column(0).unwrap().vla_column().unwrap();
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

    let restored =
        table::internals::uncompress_table(&encoded_header, encoded_table.view()).unwrap();
    let restored = Table::from_data(&restored.header, restored.data).unwrap();
    assert_eq!(
        restored.column(0).unwrap().vla().unwrap(),
        Ragged::from_rows(rows).unwrap()
    );
}

/// A declared `ZCTYPn` is honoured as written, so `RICE_1` on a 64-bit column —
/// which no conforming writer emits — is refused rather than read as `GZIP_2`.
#[test]
fn a_declared_rice_codec_on_a_64_bit_column_is_refused() {
    let column = WriteColumn::scalar("BIG", ColumnData::I64(vec![1, 2, 3]));
    let table = Builder::with_columns(3, vec![column]).unwrap();
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
        cr.read_table(1).unwrap().view().raw_rows(),
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
        reader.read_table(1),
        Err(FitsError::UnsupportedCompression { name }) if name == "RICE_1 on a K column"
    ));
}
