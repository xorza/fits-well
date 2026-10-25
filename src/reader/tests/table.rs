//! The reads of a table: by rows, columns and cells, and what each fetches.

use crate::reader::tests::*;

/// The index of `QVLA`, the last of the special columns.
const QVLA: usize = 13;

/// A table of 4 rows with a column of each special kind: logical, bit, character, complex,
/// scaled with a null, zero-width, and P and Q arrays.
fn special_columns() -> memory::Written {
    let columns = vec![
        WriteColumn::scalar("ID", ColumnData::I32(vec![10, 20, 30, 40])),
        WriteColumn::scalar(
            "LOGICAL",
            ColumnData::Logical(vec![Some(true), Some(false), None, Some(true)]),
        ),
        WriteColumn::scalar("BYTE", ColumnData::U8(vec![1, 2, 3, 4])),
        WriteColumn::bits(
            "FLAGS",
            [
                bitvec![u8, Msb0; 1, 0, 1, 0, 0],
                bitvec![u8, Msb0; 0, 1, 0, 1, 0],
                bitvec![u8, Msb0; 1, 1, 1, 0, 0],
                bitvec![u8, Msb0; 0, 0, 0, 1, 0],
            ],
            5,
        )
        .unwrap(),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Character(b"one two tri four".to_vec()),
            4,
        ),
        WriteColumn::scalar(
            "COMPLEX",
            ColumnData::ComplexF32(vec![
                Complex::new(1.0, -1.0),
                Complex::new(2.0, -2.0),
                Complex::new(3.0, -3.0),
                Complex::new(4.0, -4.0),
            ]),
        ),
        WriteColumn::scalar(
            "COMPLEX64",
            ColumnData::ComplexF64(vec![
                Complex::new(10.0, -10.0),
                Complex::new(20.0, -20.0),
                Complex::new(30.0, -30.0),
                Complex::new(40.0, -40.0),
            ]),
        ),
        WriteColumn::scalar("SCALED", ColumnData::I16(vec![1, -99, 2, 3]))
            .with_scaling(2.0, 10.0)
            .with_null(-99),
        WriteColumn::scalar("LONG", ColumnData::I64(vec![100, 200, 300, 400])),
        WriteColumn::scalar("FLOAT", ColumnData::F32(vec![1.5, 2.5, 3.5, 4.5])),
        WriteColumn::scalar("DOUBLE", ColumnData::F64(vec![5.5, 6.5, 7.5, 8.5])),
        WriteColumn::fixed("ZERO", ColumnData::I16(Vec::new()), 0),
        WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![
                ColumnData::I32(vec![1]),
                ColumnData::I32(vec![2, 3]),
                ColumnData::I32(Vec::new()),
                ColumnData::I32(vec![4, 5, 6]),
            ])
            .unwrap(),
            DescriptorWidth::P,
        ),
        WriteColumn::vla(
            "QVLA",
            Ragged::from_rows(vec![
                ColumnData::F64(vec![0.5]),
                ColumnData::F64(Vec::new()),
                ColumnData::F64(vec![2.5, 3.5]),
                ColumnData::F64(vec![4.5]),
            ])
            .unwrap(),
            DescriptorWidth::Q,
        ),
    ];
    let table = Builder::with_columns(4, columns).unwrap();
    memory::written(|w| w.write_table(&table, None))
}

/// The bytes of the `QVLA` descriptor of row 1 in `bytes`.
fn qvla_descriptor_of_row_1(bytes: &[u8]) -> Range<usize> {
    let reader = FitsReader::from_bytes(bytes).unwrap();
    let schema = reader.hdus[1].table_schema().unwrap();
    let qvla = &schema.columns[QVLA];
    assert_eq!(qvla.name().unwrap(), Some("QVLA"));
    let start =
        usize::try_from(reader.hdus[1].data_offset).unwrap() + schema.row_len() + qvla.byte_offset;
    start..start + qvla.tform.byte_width()
}

/// Rows 1 and 2 of each column kind read as the same rows of the whole table, an empty
/// range as no row.
#[test]
fn row_range_reads_each_column_kind() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    let schema = reader.hdu(1).unwrap().table_schema().unwrap().clone();
    assert_eq!(schema.row_count(), 4);
    assert_eq!(schema.columns.len(), 14);
    let empty = reader.read_table_rows(1, 2..2).unwrap();
    assert_eq!(empty.schema().row_count(), 0);
    assert_eq!(
        empty.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(Vec::new()), Vec::new())
    );
    let full = reader.read_table_rows(1, 0..4).unwrap();
    assert_eq!(
        full.column("QVLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]), vec![1, 1, 3, 4])
    );
    let ranged = reader.read_table_rows(1, 1..3).unwrap();
    assert_eq!(
        ranged.column("ID").unwrap().raw().unwrap(),
        ColumnData::I32(vec![20, 30])
    );
    let flags = ranged.column("FLAGS").unwrap().bits().unwrap();
    assert_eq!(
        (0..5)
            .map(|column| flags.get(0, column).unwrap())
            .collect::<Vec<_>>(),
        [false, true, false, true, false]
    );
    assert_eq!(
        ranged.column("NAME").unwrap().raw().unwrap(),
        ColumnData::Character(b"two tri ".to_vec())
    );
    assert_eq!(
        ranged.column("COMPLEX").unwrap().complex().unwrap(),
        [Complex::new(2.0, -2.0), Complex::new(3.0, -3.0)]
    );
    let physical = ranged.column("SCALED").unwrap().physical().unwrap();
    assert!(physical[0].is_nan());
    assert_eq!(physical[1], 14.0);
    assert_eq!(
        ranged.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2, 2])
    );
    assert_eq!(
        ranged.column("QVLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::F64(vec![2.5, 3.5]), vec![0, 2])
    );
}

/// Any range form takes the rows it bounds: an open end the table's, an inclusive end its
/// row too.
#[test]
fn every_range_form_takes_the_rows_it_bounds() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    for (table, expected) in [
        (reader.read_table_rows(1, ..).unwrap(), vec![10, 20, 30, 40]),
        (reader.read_table_rows(1, 2..).unwrap(), vec![30, 40]),
        (reader.read_table_rows(1, 1..=2).unwrap(), vec![20, 30]),
        (reader.read_table_rows(1, ..1).unwrap(), vec![10]),
    ] {
        assert_eq!(
            table.column("ID").unwrap().raw().unwrap(),
            ColumnData::I32(expected)
        );
    }
    let selection = reader.read_table_columns(1, 3.., ["ID"]).unwrap();
    assert_eq!(selection.rows, 3..4);
}

#[test]
fn column_selection_reads_its_columns_and_keeps_their_numbers() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    let selection = reader.read_table_columns(1, 1..3, ["ID", "VLA"]).unwrap();
    assert_eq!(selection.rows, 1..3);
    assert_matches!(
        &selection.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values == &[20, 30]
    );
    assert_matches!(
        &selection.columns[1].data,
        SelectedData::Variable(values)
            if values == &Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2, 2])
    );
    // `ID` is column 0 and `VLA` column 12 of the table; `QVLA`, column 13, is not selected.
    let number = |column: Option<&SelectedColumn>| column.map(|column| column.descriptor.number);
    assert_eq!(number(selection.column("id").unwrap()), Some(1));
    assert_eq!(number(selection.column(0).unwrap()), Some(1));
    assert_eq!(number(selection.column(12).unwrap()), Some(13));
    assert!(selection.column(13).unwrap().is_none());
    assert!(selection.column("QVLA").unwrap().is_none());
}

/// A read into a selection reuses its buffers, takes the columns asked even of another
/// kind, and leaves it empty on a failure.
#[test]
fn selection_read_into_reuses_its_buffers() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    let selection = reader.read_table_columns(1, 1..3, ["ID", "VLA"]).unwrap();
    let mut reused = selection.clone();
    let id_buffer = match &reused.columns[0].data {
        SelectedData::Fixed(ColumnData::I32(values)) => values.as_ptr(),
        other => panic!("{other:?}"),
    };
    reader
        .read_table_columns_into(1, 1..3, ["ID", "VLA"], &mut reused)
        .unwrap();
    assert_eq!(reused.rows, selection.rows);
    assert_eq!(reused.columns[0].data, selection.columns[0].data);
    assert_eq!(reused.columns[1].data, selection.columns[1].data);
    assert_matches!(
        &reused.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values.as_ptr() == id_buffer
    );
    // The fixed `ID` column becomes the variable `QVLA` column, and `VLA` is dropped.
    reader
        .read_table_columns_into(1, 0..4, ["QVLA"], &mut reused)
        .unwrap();
    assert_eq!(reused.rows, 0..4);
    assert_eq!(reused.columns.len(), 1);
    assert_eq!(reused.columns[0].descriptor.name().unwrap(), Some("QVLA"));
    assert_eq!(
        reused.columns[0].data,
        SelectedData::Variable(Ragged::<ColumnData>::new(
            ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]),
            vec![1, 1, 3, 4]
        ))
    );
    assert_matches!(
        reader.read_table_columns_into(1, 0..4, ["ID", "MISSING"], &mut reused),
        Err(FitsError::ColumnNotFound { name }) if name == "MISSING"
    );
    assert_eq!(reused.rows, 0..0);
    assert!(reused.columns.is_empty());
}

#[test]
fn cell_read_gives_each_column_kind() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    assert_eq!(
        reader.read_table_cell(1, 3, "QVLA").unwrap(),
        ColumnData::F64(vec![4.5])
    );
    for (column, expected) in [
        ("ID", ColumnData::I32(vec![30])),
        ("LOGICAL", ColumnData::Logical(vec![None])),
        ("BYTE", ColumnData::U8(vec![3])),
        ("FLAGS", ColumnData::Bit(bitvec![u8, Msb0; 1, 1, 1, 0, 0])),
        ("NAME", ColumnData::Character(b"tri ".to_vec())),
        (
            "COMPLEX",
            ColumnData::ComplexF32(vec![Complex::new(3.0, -3.0)]),
        ),
        (
            "COMPLEX64",
            ColumnData::ComplexF64(vec![Complex::new(30.0, -30.0)]),
        ),
        ("SCALED", ColumnData::I16(vec![2])),
        ("LONG", ColumnData::I64(vec![300])),
        ("FLOAT", ColumnData::F32(vec![3.5])),
        ("DOUBLE", ColumnData::F64(vec![7.5])),
        ("ZERO", ColumnData::I16(Vec::new())),
        ("VLA", ColumnData::I32(Vec::new())),
        ("QVLA", ColumnData::F64(vec![2.5, 3.5])),
    ] {
        assert_eq!(
            reader.read_table_cell(1, 2, column).unwrap(),
            expected,
            "{column}"
        );
    }
}

#[test]
fn rows_past_the_table_are_refused() {
    let bytes = special_columns();
    let mut reader = bytes.slice();
    assert_matches!(
        reader.read_table_rows(1, 3..5),
        Err(FitsError::RowRangeOutOfBounds {
            start: 3,
            end: 5,
            len: 4,
        })
    );
    assert_matches!(
        reader.read_table_rows(1, 2..=usize::MAX),
        Err(FitsError::RowRangeOutOfBounds {
            start: 2,
            end: usize::MAX,
            len: 4,
        })
    );
    assert_matches!(
        reader.read_table_cell(1, 4, "ID"),
        Err(FitsError::RowRangeOutOfBounds {
            start: 4,
            end: 5,
            len: 4,
        })
    );
}

/// A defective `QVLA` descriptor fails the reads of its row that take `QVLA` alone.
#[test]
fn defective_descriptor_fails_only_the_reads_of_its_column() {
    let bytes = special_columns();
    let mut corrupted = bytes.clone();
    corrupted[qvla_descriptor_of_row_1(&bytes)].fill(0xff);
    let mut selective = FitsReader::from_bytes(&corrupted).unwrap();
    let selection = selective
        .read_table_columns(1, 1..2, ["ID", "VLA"])
        .unwrap();
    assert_matches!(
        &selection.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values == &[20]
    );
    assert_matches!(
        &selection.columns[1].data,
        SelectedData::Variable(values)
            if values == &Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2])
    );
    assert_eq!(
        selective.read_table_cell(1, 1, "ID").unwrap(),
        ColumnData::I32(vec![20])
    );
    assert_matches!(
        selective.read_table_rows(1, 1..2),
        Err(FitsError::InvalidPqDescriptor {
            field: DescriptorField::Count,
            value: -1
        })
    );
}

/// A count the heap cannot hold names the column and the table's row, through a selection
/// of rows as through a cell.
#[test]
fn count_past_the_heap_names_its_column_and_row() {
    let bytes = special_columns();
    let mut past_heap = bytes.clone();
    let count = qvla_descriptor_of_row_1(&bytes).start;
    past_heap[count..count + 8].copy_from_slice(&1_000_000i64.to_be_bytes());
    let mut overrun = FitsReader::from_bytes(&past_heap).unwrap();
    for error in [
        overrun.read_table_rows(1, 1..2).unwrap_err(),
        overrun.read_table_cell(1, 1, "QVLA").unwrap_err(),
    ] {
        assert_matches!(
            error,
            FitsError::DescriptorPastHeap { column, row: 1 } if column == QVLA
        );
    }
}

#[test]
fn stream_reads_rows_and_selections_as_a_slice_does() {
    let bytes = special_columns();
    let mut stream = bytes.stream();
    let one = stream.read_table_rows(1, 3..4).unwrap();
    assert_eq!(
        one.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(vec![4, 5, 6]), vec![3])
    );
    assert_eq!(
        stream
            .read_table_columns(1, 0..4, ["QVLA"])
            .unwrap()
            .columns[0]
            .data,
        SelectedData::Variable(Ragged::<ColumnData>::new(
            ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]),
            vec![1, 1, 3, 4]
        ))
    );
}

#[test]
fn malformed_pq_descriptors_match_across_table_read_paths() {
    for width in [DescriptorWidth::P, DescriptorWidth::Q] {
        let column = WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![ColumnData::U8(vec![7])]).unwrap(),
            width,
        );
        let table = Builder::with_columns(1, vec![column]).unwrap();
        let bytes = memory::written(|w| w.write_table(&table, None));
        let base_reader = bytes.slice();
        let schema = base_reader.hdu(1).unwrap().table_schema().unwrap();
        let slot = usize::try_from(base_reader.hdus[1].data_offset).unwrap()
            + schema.columns[0].byte_offset;

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.width == width)
        {
            let mut corrupted = bytes.clone();
            corrupted[slot..slot + case.bytes.len()].copy_from_slice(&case.bytes);
            let mut reader = FitsReader::from_bytes(&corrupted).unwrap();

            let whole = reader.read_table(1).unwrap();
            case.assert_error(&whole.column("VLA").unwrap().vla().unwrap_err());
            case.assert_error(&reader.read_table_cell(1, 0, "VLA").unwrap_err());
            case.assert_error(&reader.read_table_rows(1, 0..1).unwrap_err());
            case.assert_error(&reader.read_table_columns(1, 0..1, ["VLA"]).unwrap_err());
        }
    }
}

/// A partial read of a plain table of fixed columns fetches its rows and no other byte:
/// rows 9 and 10 of rows of 4 + 8 bytes are 24 bytes.
#[test]
fn partial_read_of_a_plain_table_fetches_its_rows() {
    let columns = [
        WriteColumn::scalar("ID", ColumnData::I32((0..50).collect())),
        WriteColumn::scalar("FLUX", ColumnData::F64((0..50).map(f64::from).collect())),
    ];
    let bytes = memory::written(|w| w.write_table(&memory::binary_table(50, &columns), None));
    let mut counted = CountedReader::open(&bytes);
    let rows = counted.reader.read_table_rows(1, 9..11).unwrap();
    assert_eq!(rows.schema().row_count(), 2);
    assert_eq!(counted.fetched(), 2 * (4 + 8));
}

/// A partial read of a tiled-compressed table with no arrays fetches only the tiles its
/// rows touch: row 9 lies in tile 1 of tiles of 7 rows, and its read fetches what a
/// selection of container row 1 fetches; a read of every row fetches the container.
#[cfg(feature = "compression")]
#[test]
fn partial_read_of_a_compressed_table_fetches_its_tiles() {
    let table = Builder::with_columns(50, mixed_columns(50)).unwrap();
    let bytes = memory::written(|w| {
        w.write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::GZIP).with_tiles([7]),
            None,
        )
    });
    let fetched = |container_rows: bool, rows: Range<usize>| {
        let mut counted = CountedReader::open(&bytes);
        if container_rows {
            counted.reader.read_container_rows(1, rows).unwrap();
        } else {
            counted.reader.read_table_rows(1, rows).unwrap();
        }
        (counted.fetched(), counted.reader.hdus()[1].data_bytes)
    };
    let (part, container) = fetched(false, 9..10);
    assert_eq!(part, fetched(true, 1..2).0);
    assert!((part as u64) < container);
    assert_eq!(fetched(false, 0..50).0 as u64, container);

    // Beside an array column, a selection of a fixed column still fetches only its
    // tiles; a selection of the array column fetches the whole container, as its
    // descriptors address the container's heap.
    let mut columns = mixed_columns(50);
    let arrays =
        (0..50).map(|row: i16| ColumnData::I16(vec![row; usize::try_from(row % 4).unwrap()]));
    columns.push(WriteColumn::vla(
        "ARRAY",
        Ragged::<ColumnData>::from_rows(arrays).unwrap(),
        DescriptorWidth::P,
    ));
    let table = Builder::with_columns(50, columns).unwrap();
    let bytes = memory::written(|w| {
        w.write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::GZIP).with_tiles([7]),
            None,
        )
    });
    let fetched = |column: &str| {
        let mut counted = CountedReader::open(&bytes);
        let selection = counted
            .reader
            .read_table_columns(1, 9..10, [column])
            .unwrap();
        assert_eq!(selection.rows, 9..10);
        (counted.fetched(), counted.reader.hdus()[1].data_bytes)
    };
    let mut counted = CountedReader::open(&bytes);
    counted.reader.read_container_rows(1, 1..2).unwrap();
    let tile = counted.fetched();
    assert_eq!(fetched("SHORT").0, tile, "a fixed column fetches its tile");
    let (whole, container) = fetched("ARRAY");
    assert_eq!(
        whole as u64, container,
        "the array column fetches the container"
    );

    // The reader keeps the rows a cell read decoded: a second cell of the same tile and
    // column fetches nothing more, and a cell of another column decodes again. Each gives
    // the value the whole table holds.
    let restored = bytes.slice().read_table(1).unwrap();
    let mut counted = CountedReader::open(&bytes);
    assert_eq!(
        counted.reader.read_table_cell(1, 8, "SHORT").unwrap(),
        ColumnData::I16(vec![8 * 7 - 30])
    );
    let first = counted.fetched();
    assert_eq!(
        counted.reader.read_table_cell(1, 9, "SHORT").unwrap(),
        ColumnData::I16(vec![9 * 7 - 30])
    );
    assert_eq!(counted.fetched(), first, "the tile is decoded once");
    assert_eq!(
        counted.reader.read_table_cell(1, 9, "ARRAY").unwrap(),
        restored
            .column("ARRAY")
            .unwrap()
            .vla()
            .unwrap()
            .row(9)
            .to_data()
    );
    assert!(counted.fetched() > first, "another column decodes again");
}
