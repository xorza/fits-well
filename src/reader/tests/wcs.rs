//! The reader's WCS, with its `-TAB` arrays read from their extensions.

use crate::reader::tests::*;

fn write_tab_lookup(
    writer: &mut FitsWriter<Cursor<Vec<u8>>>,
    version: i64,
    level: i64,
    coordinates: &[f64],
) {
    let shape = format!("(1,{})", coordinates.len());
    write_tab_lookup_columns(
        writer,
        version,
        level,
        &[("COORD", coordinates, Some(shape.as_str()))],
    );
}

fn write_tab_lookup_columns(
    writer: &mut FitsWriter<Cursor<Vec<u8>>>,
    version: i64,
    level: i64,
    columns: &[(&str, &[f64], Option<&str>)],
) {
    let mut header = lookup_header(columns);
    header
        .set_internal(keywords::EXTNAME, "WCS-TABLE")
        .set_internal(keywords::EXTVER, version)
        .set_internal(keywords::EXTLEVEL, level);
    writer
        .write_raw_hdu(&header, &lookup_bytes(columns))
        .unwrap();
}

#[test]
fn read_wcs_resolves_the_exact_tabular_extension() {
    let mut primary = cards::structural(None, 8, &[1]);
    primary
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 1.0)
        .set_internal("CDELT1", 1.0)
        .set_internal("PS1_0", "WCS-TABLE")
        .set_internal("PV1_1", 2)
        .set_internal("PV1_2", 3)
        .set_internal("PS1_1", "COORD")
        .set_internal("PV1_3", 1);
    let mut writer = memory::memory_writer();
    writer.write_raw_hdu(&primary, &[0]).unwrap();
    // A decoy BINTABLE whose EXTNAME does not match. Its EXTVER is not an integer, so
    // resolving the reference must never interpret it — an unrelated corrupt version
    // card cannot fail a lookup that does not concern it.
    let mut decoy = table_header(0, 0, &[]);
    decoy
        .set_internal(keywords::EXTNAME, "OTHER")
        .set_internal(keywords::EXTVER, "not-an-integer");
    writer.write_raw_hdu(&decoy, &[]).unwrap();
    write_tab_lookup(&mut writer, 2, 1, &[100.0, 200.0, 400.0]);
    write_tab_lookup(&mut writer, 2, 3, &[10.0, 20.0, 40.0]);

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_matches!(
        to_world(&Wcs::from_header(&reader.hdus[0].header, None).unwrap(), &[2.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0]
    );
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
    assert_eq!(to_pixel(&wcs, &[30.0]).unwrap(), [2.5]);

    // An extension the file does not hold is the lookup's own error.
    let alone = memory::written(|w| w.write_raw_hdu(&primary, &[0]));
    assert_matches!(
        alone.slice().read_wcs(0, None),
        Err(FitsError::HduNotFound { name, version: Some(2), level: Some(3) })
            if name == "WCS-TABLE"
    );

    // A tiled-compressed table serves its arrays as the plain one does.
    #[cfg(feature = "compression")]
    {
        let lookup = Builder::with_columns(
            1,
            [
                WriteColumn::fixed("COORD", ColumnData::F64(vec![10.0, 20.0, 40.0]), 3)
                    .with_tdim([1, 3]),
            ],
        )
        .unwrap();
        let mut names = Header::new();
        names
            .set_internal(keywords::EXTNAME, "WCS-TABLE")
            .set_internal(keywords::EXTVER, 2)
            .set_internal(keywords::EXTLEVEL, 3);
        let compressed = memory::written(|w| {
            w.write_raw_hdu(&primary, &[0])?;
            w.write_compressed_table(
                &lookup,
                &CompressionConfig::new(Compression::GZIP),
                Some(&names),
            )
        });
        let mut reader = compressed.slice();
        let wcs = reader.read_wcs(0, None).unwrap();
        assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
    }
}

#[test]
fn read_wcs_resolves_shared_arrays_and_extensions_once_per_reference_group() {
    let mut primary = cards::structural(None, 8, &[]);
    primary.set_internal(keywords::WCSAXES, 5);
    for (axis, column, version, table_axis) in [
        (1, "FIRST", 1, 1),
        (2, "SECOND", 1, 1),
        (3, "COUPLED", 1, 1),
        (4, "COUPLED", 1, 2),
        (5, "DISTINCT", 2, 1),
    ] {
        primary.set_tab_axis(axis, column, table_axis);
        primary.set_internal(format!("PV{axis}_1").as_str(), version);
    }
    let coupled = [30.0, 40.0, 31.0, 40.0, 30.0, 42.0, 31.0, 42.0];
    let mut writer = memory::memory_writer();
    writer.write_raw_hdu(&primary, &[]).unwrap();
    write_tab_lookup_columns(
        &mut writer,
        1,
        1,
        &[
            ("FIRST", &[10.0, 20.0], Some("(1,2)")),
            ("SECOND", &[100.0, 200.0], Some("(1,2)")),
            ("COUPLED", &coupled, Some("(2,2,2)")),
        ],
    );
    write_tab_lookup_columns(
        &mut writer,
        2,
        1,
        &[("DISTINCT", &[1000.0, 2000.0], Some("(1,2)"))],
    );

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(
        to_world(&wcs, &[1.0, 1.0, 1.0, 1.0, 1.0]).unwrap(),
        [10.0, 100.0, 30.0, 40.0, 1000.0]
    );
}

#[test]
fn read_wcs_fetches_only_the_referenced_first_row_heap_cells() {
    let mut primary = cards::structural(None, 8, &[1]);
    primary
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 10.0)
        .set_internal("CDELT1", 10.0)
        .set_internal("PS1_0", "WCS-TABLE")
        .set_internal("PS1_1", "COORD")
        .set_internal("PS1_2", "INDEX")
        .set_internal("PV1_3", 1);
    let row_count = 64;
    let coordinates = (0..row_count).map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]));
    let indices = (0..row_count).map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]));
    let unused = (0..row_count).map(|_| ColumnData::U8(vec![7; 1024]));
    let table = Builder::with_columns(
        row_count,
        vec![
            WriteColumn::vla(
                "COORD",
                Ragged::from_rows(coordinates).unwrap(),
                DescriptorWidth::P,
            )
            .with_tdim(vec![1, 3]),
            WriteColumn::vla(
                "INDEX",
                Ragged::from_rows(indices).unwrap(),
                DescriptorWidth::P,
            ),
            WriteColumn::vla(
                "UNUSED",
                Ragged::from_rows(unused).unwrap(),
                DescriptorWidth::P,
            ),
        ],
    )
    .unwrap();
    let mut lookup_header = Header::new();
    lookup_header.set_internal(keywords::EXTNAME, "WCS-TABLE");
    let mut writer = memory::memory_writer();
    writer.write_raw_hdu(&primary, &[0]).unwrap();
    writer.write_table(&table, Some(&lookup_header)).unwrap();

    let bytes = writer.into_inner().into_inner();
    let mut counted = CountedReader::open(&bytes);
    let wcs = counted.reader.read_wcs(0, None).unwrap();
    assert_eq!(counted.fetched(), 24 + 6 * size_of::<f64>());
    assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
}
