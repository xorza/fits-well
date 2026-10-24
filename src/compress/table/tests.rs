use crate::error::CompressedDataDefect;
use crate::error::CompressionFeature;
use crate::error::ExpectedHdu;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use std::ops::Range;

use crate::bintable::Table;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor;
use crate::bintable::descriptor::{DescriptorWidth, PqDescriptor};
use crate::bintable::element_kind::ElementKind;
use crate::bintable::internals::table_header;
use crate::bintable::schema::Schema;
use crate::bintable::selection::{SelectedData, Selection};
use crate::bintable::tform::Storage;
use crate::column::ColumnSelector;
use crate::compress::table::ContainerTiles;
use crate::compress::table::internals::mixed_columns;
use crate::compress::*;
use crate::error::SizeUnit;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::hdu::HduKind;
use crate::header_model::{Signing, Space};
use crate::internals::cards::{assert_absent, assert_cards};
use crate::internals::fixture::Fixture;
use crate::internals::memory::{round_trip, written};
use crate::internals::rng::SplitMix;
use crate::ragged::Ragged;
use crate::reader::FitsReader;
use crate::reader::source::SliceSource;
use num_complex::Complex;
use std::io::Cursor;

/// The gzip codec of a descriptor block.
const GZIP: TileCodec = TileCodec::Gzip1 { level: 1 };

/// What the writer writes of one table: its plain HDU and its container.
#[derive(Debug)]
struct Encoded {
    plain: table::internals::HduParts,
    container: table::internals::HduParts,
}

impl Encoded {
    /// The header and data unit of `table` that `write_table` writes, and of its container
    /// that `write_compressed_table` writes under `config`.
    fn new(table: &Builder, config: &CompressionConfig) -> Self {
        let parts = |bytes: Vec<u8>| {
            let mut reader = FitsReader::from_bytes(&bytes).unwrap();
            table::internals::HduParts {
                header: reader.hdus[1].header.clone(),
                data: reader.read_data_unit(1).unwrap(),
            }
        };
        Self {
            plain: parts(written(|w| w.write_table(table, None))),
            container: parts(written(|w| w.write_compressed_table(table, config, None))),
        }
    }

    /// The container decompressed whole.
    fn restored(&self) -> table::internals::HduParts {
        let schema = Schema::parse_container(&self.container.header).unwrap();
        let container = Table::new(schema, self.container.data.clone()).unwrap();
        table::internals::uncompress_table(&self.container.header, container.view()).unwrap()
    }
}

/// Each codec restores the data unit `write_table` writes, in one tile, in several, and in
/// tiles of one row; the restored header keeps a keyword that only looks like a container's
/// and drops the container's own.
#[test]
fn table_compression_round_trips() {
    // A column of repeat 0 has cells of no bytes, beside the others.
    let columns = || {
        let mut columns = mixed_columns(10);
        columns.push(WriteColumn::fixed("EMPTY", ColumnData::I32(Vec::new()), 0));
        columns
    };
    let table = Builder::with_columns(10, columns()).unwrap();
    for rows in [10, 4, 1] {
        for compression in [
            Compression::GZIP,
            Compression::GZIP_SHUFFLED,
            Compression::Rice,
        ] {
            let context = format!("{} in tiles of {rows}", compression.name());
            let mut encoded = Encoded::new(
                &table,
                &CompressionConfig::new(compression).with_tiles([rows]),
            );
            encoded.container.header.set_internal("ZFORM01", "preserve");
            let restored = encoded.restored();
            assert_eq!(restored.data, encoded.plain.data, "{context}");
            let container_columns: Vec<String> = (1..=columns().len())
                .flat_map(|n| [format!("ZFORM{n}"), format!("ZCTYP{n}")])
                .collect();
            let mut absent: Vec<&str> = container_columns.iter().map(String::as_str).collect();
            absent.extend([
                "ZTABLE", "ZTILELEN", "ZNAXIS1", "ZNAXIS2", "ZPCOUNT", "ZHEAPPTR",
            ]);
            assert_absent(&restored.header, &absent);
            assert_cards(&restored.header, &[("ZFORM01", "preserve".into())]);
        }
    }
    let table = Builder::with_columns(10, mixed_columns(10)).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_table(&table, &CompressionConfig::new(Compression::None), None),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::TableWithoutCodec
        })
    ));
}

/// `comp_table_nocompress.fits` holds `ZCTYPn = 'NOCOMPRESS'` columns (§10.4, Table 36),
/// fixed and variable-length, written by fits-well before its writer refused them
/// (cfitsio cannot read them), its descriptor blocks since moved to the cfitsio order;
/// the reader still restores such a table exactly.
#[test]
fn reads_nocompress_table_columns() {
    let rows = 12;
    let arrays =
        Ragged::from_rows((0..rows).map(|r| ColumnData::I16((0..r as i16).collect()))).unwrap();
    let table = Builder::with_columns(
        rows,
        vec![
            WriteColumn::scalar("ID", ColumnData::I32((0..rows as i32).collect())),
            WriteColumn::scalar(
                "FLUX",
                ColumnData::F64((0..rows).map(|r| r as f64 * 1.5 - 3.0).collect()),
            ),
            WriteColumn::vla("ARRAY", arrays, DescriptorWidth::P),
        ],
    )
    .unwrap();
    let mut reader = Fixture::CompTableNocompress.stream();
    for n in 1..=3 {
        assert_eq!(
            reader.hdus[1].header.get_text(format!("ZCTYP{n}")).unwrap(),
            Some("NOCOMPRESS")
        );
    }
    let container = reader.read_container_table(1).unwrap();
    let restored =
        table::internals::uncompress_table(&reader.hdus[1].header, container.view()).unwrap();
    let plain = round_trip(|w| w.write_table(&table, None))
        .read_data_unit(1)
        .unwrap();
    assert_eq!(restored.data, plain);
}

/// Each column type takes the codec cfitsio reads for it: `GZIP_2` only on `I`, `J`,
/// `K`, `E` and `D`, `RICE_1` only on `B`, `I` and `J`, and `GZIP_1` on the rest.
#[test]
fn table_codecs_are_the_ones_cfitsio_reads() {
    let rows = 3;
    let bits: [BitVec<u8, Msb0>; 3] = [
        BitVec::repeat(true, 5),
        BitVec::repeat(false, 5),
        BitVec::repeat(true, 5),
    ];
    let table = Builder::with_columns(
        rows,
        vec![
            WriteColumn::scalar(
                "L",
                ColumnData::Logical(vec![Some(true), None, Some(false)]),
            ),
            WriteColumn::bits("X", &bits, 5).unwrap(),
            WriteColumn::characters("A", ["a", "bc", "def"], 3).unwrap(),
            WriteColumn::scalar("B", ColumnData::U8(vec![1, 2, 3])),
            WriteColumn::scalar("I", ColumnData::I16(vec![1, 2, 3])),
            WriteColumn::scalar("J", ColumnData::I32(vec![1, 2, 3])),
            WriteColumn::scalar("K", ColumnData::I64(vec![1, 2, 3])),
            WriteColumn::scalar("E", ColumnData::F32(vec![1.0, 2.0, 3.0])),
            WriteColumn::scalar("D", ColumnData::F64(vec![1.0, 2.0, 3.0])),
            WriteColumn::scalar("C", ColumnData::ComplexF32(vec![Complex::new(1.0, 2.0); 3])),
            WriteColumn::scalar("M", ColumnData::ComplexF64(vec![Complex::new(1.0, 2.0); 3])),
        ],
    )
    .unwrap();
    for (compression, codecs) in [
        (
            Compression::GZIP_SHUFFLED,
            [
                "GZIP_1", "GZIP_1", "GZIP_1", "GZIP_1", "GZIP_2", "GZIP_2", "GZIP_2", "GZIP_2",
                "GZIP_2", "GZIP_1", "GZIP_1",
            ],
        ),
        (
            Compression::Rice,
            [
                "GZIP_1", "GZIP_1", "GZIP_1", "RICE_1", "RICE_1", "RICE_1", "GZIP_2", "GZIP_2",
                "GZIP_2", "GZIP_1", "GZIP_1",
            ],
        ),
        (Compression::GZIP, ["GZIP_1"; 11]),
    ] {
        let encoded = Encoded::new(&table, &CompressionConfig::new(compression));
        for (n, codec) in codecs.iter().enumerate() {
            assert_eq!(
                encoded
                    .container
                    .header
                    .get_text(format!("ZCTYP{}", n + 1))
                    .unwrap(),
                Some(*codec),
                "{} column {}",
                compression.name(),
                n + 1
            );
        }
        assert_eq!(
            encoded.restored().data,
            encoded.plain.data,
            "{}",
            compression.name()
        );
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
    let restored = Fixture::CompTableCfitsio.stream().read_table(1).unwrap();
    let original = Fixture::CompTableRef.stream().read_table(1).unwrap();

    assert_eq!(restored.schema().row_count(), 500);
    assert_eq!(restored.schema().row_count(), original.schema().row_count());
    assert_eq!(restored.schema.row_len(), original.schema.row_len());
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
    let mut encoded = Encoded::new(
        &table,
        &CompressionConfig::new(Compression::GZIP).with_tiles([1]),
    );
    let container_header = &mut encoded.container.header;
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
    let restored = encoded.restored();

    let original = &mut encoded.plain;
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
    original
        .header
        .render_into(Signing::Unsigned, Space::Spare(0), &mut original_bytes)
        .unwrap();
    restored
        .header
        .render_into(Signing::Unsigned, Space::Spare(0), &mut restored_bytes)
        .unwrap();
    assert_eq!(restored_bytes, original_bytes);
    assert_eq!(restored.data, original.data);
}

#[test]
fn decodes_a_cfitsio_compressed_table_with_a_vla_column() {
    let mut f = Fixture::CompTableVla.stream();
    let table = f.read_table(1).unwrap();
    assert_eq!(
        table.schema().columns[1].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(table.schema().columns[1].tform.element(), ElementKind::I32);
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
    let bits = Ragged::from_rows((0..rows).map(|row| {
        ColumnData::Bit(
            (0..row % 11)
                .map(|bit| bit % 3 == 0)
                .collect::<BitVec<u8, Msb0>>(),
        )
    }))
    .unwrap();
    let table = Builder::with_columns(
        rows,
        vec![
            WriteColumn::scalar("ID", ColumnData::I32((0..600).collect())),
            WriteColumn::vla("NARROW", arrays(), DescriptorWidth::P),
            WriteColumn::vla("WIDE", arrays(), DescriptorWidth::Q),
            WriteColumn::vla("FLAGS", bits, DescriptorWidth::P),
        ],
    )
    .unwrap();
    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
    ] {
        let encoded = Encoded::new(
            &table,
            &CompressionConfig::new(compression).with_tiles([127]),
        );
        assert_eq!(
            encoded
                .container
                .header
                .get_integer(keywords::ZPCOUNT)
                .unwrap(),
            encoded.plain.header.get_integer(keywords::PCOUNT).unwrap(),
            "{}",
            compression.name()
        );
        assert_eq!(
            encoded.restored().data,
            encoded.plain.data,
            "{}",
            compression.name()
        );
    }
}

#[test]
fn compressed_table_decode_rejects_the_shared_malformed_pq_corpus() {
    for width in [DescriptorWidth::P, DescriptorWidth::Q] {
        let column = WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![ColumnData::U8(vec![7])]).unwrap(),
            width,
        );
        let prefix = WriteColumn::scalar("PREFIX", ColumnData::U8(vec![3]));
        let builder = Builder::with_columns(1, vec![prefix, column]).unwrap();
        let table::internals::HduParts {
            header: compressed_header,
            data: encoded,
        } = Encoded::new(
            &builder,
            &CompressionConfig::new(Compression::GZIP).with_tiles([1]),
        )
        .container;
        let outer = PqDescriptor::decode(&encoded[16..32], DescriptorWidth::Q).unwrap();
        let stream_range = outer
            .heap_range(ElementKind::U8, 32, encoded.len())
            .unwrap();
        let heap_prefix = encoded[32..stream_range.start].to_vec();
        let combined_len = 16 + width.bytes();
        let combined = GZIP.decoded_bytes(&encoded[stream_range], combined_len);

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.width == width)
        {
            let mut malformed = combined.clone();
            malformed[..case.bytes.len()].copy_from_slice(&case.bytes);
            let cell = GZIP.encoded_bytes(&malformed);
            let mut data = encoded[..32].to_vec();
            data.extend_from_slice(&heap_prefix);
            PqDescriptor {
                count: cell.len(),
                offset: heap_prefix.len(),
            }
            .write(DescriptorWidth::Q, &mut data[16..32])
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
        DescriptorWidth::P,
    ));
    let table = Builder::with_columns(rows, columns).unwrap();
    let plain = written(|w| w.write_table(&table, None));
    let compressed = written(|w| {
        w.write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::Rice).with_tiles([7]),
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
    let schema = compressed.hdu(1).unwrap().table_schema().unwrap();
    assert_eq!(schema.row_count(), 50);
    assert_eq!(
        schema.row_len(),
        plain.hdu(1).unwrap().table_schema().unwrap().row_len()
    );
    let tforms = |schema: &Schema| schema.columns.iter().map(|c| c.tform).collect::<Vec<_>>();
    assert_eq!(
        tforms(schema),
        tforms(plain.hdu(1).unwrap().table_schema().unwrap())
    );

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
    let cfitsio = Fixture::CompTableCfitsio.stream().read_table(1).unwrap();
    let reference = Fixture::CompTableRef.stream().read_table(1).unwrap();
    assert_eq!(cfitsio.view().raw_rows(), reference.view().raw_rows());

    // A compressed image is no table.
    let mut image = Fixture::CompGzipI16.stream();
    let errors = [
        image
            .hdu(1)
            .unwrap()
            .table_schema()
            .map(|_| ())
            .unwrap_err(),
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
        assert!(
            matches!(
                error,
                FitsError::HduKindMismatch {
                    expected: ExpectedHdu::BinTable,
                    got: HduKind::CompressedImage
                }
            ),
            "{error:?}"
        );
    }
}

/// A partial read decodes only the tiles its rows touch and the columns it selects: with
/// the last tile's last column corrupt, a read that touches neither succeeds, and the
/// reads that do fail.
#[test]
fn a_partial_read_decodes_only_the_tiles_and_columns_it_reads() {
    let [plain, mut compressed] = tiled_pair();
    {
        let mut reader = FitsReader::from_bytes(&compressed).unwrap();
        let container = reader.read_container_table(1).unwrap();
        let hdu = &reader.hdus()[1];
        let tiled = hdu.tiled_table().unwrap().unwrap();
        let row_len = tiled.schema.row_len();
        // Tiles of 7 rows: row 9 lies in tile 1, rows 7..14; row 49 is the last tile,
        // alone; an empty range holds no tile. Each decode holds its rows and the arrays
        // they reference alone: row r holds r % 5 `i32`s, so rows 7..14 hold
        // 2 + 3 + 4 + 0 + 1 + 2 + 3 = 15 of them, 60 bytes; rows 0..21 hold 4 × 10 + 0,
        // 160 bytes; row 49 holds 4, 16 bytes.
        for (rows, first_row, row_count, heap_len) in [
            (9..10, 7, 7, 60),
            (5..16, 0, 21, 160),
            (49..50, 49, 1, 16),
            (20..20, 20, 0, 0),
        ] {
            let decoded = tiled
                .decode(
                    ContainerTiles {
                        view: container.view(),
                        first_tile: 0,
                    },
                    &rows,
                    None,
                )
                .unwrap();
            assert_eq!(decoded.first_row, first_row, "{rows:?}");
            assert_eq!(decoded.schema.row_count(), row_count, "{rows:?}");
            assert_eq!(
                decoded.data.len(),
                row_count * row_len + heap_len,
                "{rows:?}"
            );
        }
        // The 50 rows hold 10 × 10 `i32`s, 400 bytes, after the main table.
        let whole = tiled
            .decode(
                ContainerTiles {
                    view: container.view(),
                    first_tile: 0,
                },
                &(0..50),
                None,
            )
            .unwrap();
        assert_eq!(whole.schema.extent.heap_offset, 50 * row_len);
        assert_eq!(whole.data.len(), 50 * row_len + 400);
    }
    let data_end = {
        let reader = FitsReader::from_bytes(&compressed).unwrap();
        let hdu = &reader.hdus()[1];
        usize::try_from(hdu.data_offset + hdu.data_bytes).unwrap()
    };
    // The heap ends with the last tile's last column, whose gzip trailer this breaks.
    compressed[data_end - 1] ^= 0xFF;
    let mut reader = FitsReader::from_bytes(&compressed).unwrap();
    let mut expected = FitsReader::from_bytes(&plain).unwrap();
    let columns = |table: Table| -> Vec<SelectedData> {
        (0..table.schema().columns.len())
            .map(|index| {
                let column = table.column(index).unwrap();
                if table.schema().columns[index]
                    .tform
                    .descriptor_width()
                    .is_some()
                {
                    SelectedData::Variable(column.vla().unwrap())
                } else {
                    SelectedData::Fixed(column.raw().unwrap())
                }
            })
            .collect()
    };
    let data = |selection: Selection| -> Vec<SelectedData> {
        selection
            .columns
            .into_iter()
            .map(|column| column.data)
            .collect()
    };
    assert_eq!(
        columns(reader.read_table_rows(1, 0..49).unwrap()),
        columns(expected.read_table_rows(1, 0..49).unwrap())
    );
    assert_eq!(
        data(
            reader
                .read_table_columns(1, 49..50, [ColumnSelector::Index(0)])
                .unwrap()
        ),
        data(
            expected
                .read_table_columns(1, 49..50, [ColumnSelector::Index(0)])
                .unwrap()
        )
    );
    assert_eq!(
        reader
            .read_table_cell(1, 49, ColumnSelector::Index(5))
            .unwrap(),
        expected
            .read_table_cell(1, 49, ColumnSelector::Index(5))
            .unwrap()
    );
    let errors = [
        reader.read_table_rows(1, 49..50).map(|_| ()).unwrap_err(),
        reader
            .read_table_columns(1, 49..50, [ColumnSelector::Index(6)])
            .map(|_| ())
            .unwrap_err(),
        reader.read_table(1).map(|_| ()).unwrap_err(),
    ];
    for error in errors {
        assert!(
            matches!(
                error,
                FitsError::CorruptCompressedData {
                    defect: CompressedDataDefect::GzipStream(_)
                }
            ),
            "{error:?}"
        );
    }
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
            unit: SizeUnit::Rows,
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
        let table = Builder::with_columns(2, vec![column]).unwrap();
        let encoded = Encoded::new(&table, &CompressionConfig::new(compression).with_tiles([2]));
        let container = Schema::parse_container(&encoded.container.header).unwrap();
        assert!(
            container
                .columns
                .iter()
                .all(|column| column.tdim().unwrap().is_none()
                    && column.tscale().unwrap() == 1.0
                    && column.tzero().unwrap() == 0.0
                    && column.tnull().unwrap().is_none())
        );
        let restored = encoded.restored();
        assert_eq!(restored.data, encoded.plain.data, "{}", compression.name());
        let restored = Table::from_data(&restored.header, restored.data).unwrap();
        assert_eq!(
            restored.schema().columns[0].tdim().unwrap(),
            Some(&[10, 10][..])
        );
    }
}

/// A variable-length array is stored compressed when its codec shrinks it and as is
/// otherwise — cfitsio's rule, which makes a stored length equal to the raw length the
/// mark of a raw array. Pseudo-random bytes do not deflate; a run of zeros does.
#[test]
fn a_variable_length_array_is_stored_compressed_only_when_it_shrinks() {
    let mut draws = SplitMix::new(1);
    let mut noise = |len: usize| -> Vec<u8> { (0..len).map(|_| draws.next() as u8).collect() };
    let noise_row = noise(40);
    let rows = vec![
        ColumnData::U8(noise_row.clone()),
        ColumnData::U8(Vec::new()),
        ColumnData::U8(vec![0; 400]),
    ];
    let column = WriteColumn::vla(
        "ARRAYS",
        Ragged::from_rows(rows.clone()).unwrap(),
        DescriptorWidth::P,
    );
    let table::internals::HduParts {
        header: encoded_header,
        data: encoded,
    } = Encoded::new(
        &Builder::with_columns(3, vec![column]).unwrap(),
        &CompressionConfig::new(Compression::GZIP).with_tiles([3]),
    )
    .container;
    let encoded_table = Table::from_data(&encoded_header, encoded).unwrap();

    // The container's one cell gunzips to the 3 original descriptors followed by the 3 of
    // their compressed arrays.
    let cell = encoded_table.column(0).unwrap().vla_column().unwrap();
    let combined = GZIP.decoded_bytes(
        convert::byte_cell(cell.cell(0).unwrap()).unwrap(),
        3 * 16 + 3 * 8,
    );
    let stored = |row: usize| {
        let descriptor = &combined[3 * 8 + row * 16..3 * 8 + (row + 1) * 16];
        let descriptor = PqDescriptor {
            count: usize::try_from(u64::from_be_bytes(descriptor[..8].try_into().unwrap()))
                .unwrap(),
            offset: usize::try_from(u64::from_be_bytes(descriptor[8..].try_into().unwrap()))
                .unwrap(),
        };
        encoded_table
            .view()
            .pq_payload(descriptor, ElementKind::U8)
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
    let data = cr.read_data_unit(1).unwrap();
    let mut file = FitsWriter::new(Cursor::new(Vec::new()));
    file.write_empty_primary(None).unwrap();
    file.write_raw_hdu(&tampered, &data).unwrap();
    let bytes = file.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        reader.read_table(1),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::RiceColumn { code: 'K' }
        })
    ));
}
