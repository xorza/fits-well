use crate::error::FitsError;
use crate::header::Header;
use crate::table_impl::BinTable;
use crate::wcs::Wcs;
use crate::wcs::tabular;
use crate::wcs::tabular::TabularTransform;

fn encoded_f64(values: &[f64]) -> Vec<u8> {
    values
        .iter()
        .flat_map(|value| value.to_be_bytes())
        .collect()
}

fn lookup_table(columns: &[(&str, &[f64], Option<&str>)]) -> BinTable {
    let mut header = Header::new();
    let row_len = columns
        .iter()
        .map(|(_, values, _)| values.len() * 8)
        .sum::<usize>();
    header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", row_len as i64)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", columns.len() as i64);
    let mut bytes = Vec::with_capacity(row_len);
    for (index, (name, values, shape)) in columns.iter().enumerate() {
        let column = index + 1;
        header
            .set_internal(format!("TTYPE{column}").as_str(), *name)
            .set_internal(
                format!("TFORM{column}").as_str(),
                format!("{}D", values.len()),
            );
        if let Some(shape) = shape {
            header.set_internal(format!("TDIM{column}").as_str(), *shape);
        }
        bytes.extend(encoded_f64(values));
    }
    BinTable::from_data(&header, bytes).unwrap()
}

fn tab_header(axis_count: usize, coordinate: &str) -> Header {
    let mut header = Header::new();
    header.set_internal("NAXIS", axis_count as i64);
    for axis in 1..=axis_count {
        header
            .set_internal(format!("CTYPE{axis}").as_str(), format!("AX{axis:02}-TAB"))
            .set_internal(format!("CRPIX{axis}").as_str(), 0.0)
            .set_internal(format!("CRVAL{axis}").as_str(), 0.0)
            .set_internal(format!("CDELT{axis}").as_str(), 1.0)
            .set_internal(format!("PS{axis}_0").as_str(), "WCS-TABLE")
            .set_internal(format!("PS{axis}_1").as_str(), coordinate)
            .set_internal(format!("PV{axis}_3").as_str(), axis as i64);
    }
    header
}

fn resolved_wcs(header: &Header, table: &BinTable) -> Wcs {
    let descriptors = tabular::descriptors(
        header,
        usize::try_from(header.get_integer("NAXIS").unwrap().unwrap()).unwrap(),
        None,
    )
    .unwrap();
    let transforms = descriptors
        .into_iter()
        .map(|descriptor| TabularTransform::from_table(descriptor, table).unwrap())
        .collect();
    Wcs::from_header_with_tabular(header, None, transforms).unwrap()
}

#[test]
fn one_dimensional_tab_supports_nonmonotonic_coordinates_and_boundaries() {
    let table = lookup_table(&[("COORD", &[10.0, 20.0, 15.0, 30.0], Some("(1,4)"))]);
    let header = tab_header(1, "COORD");
    let wcs = resolved_wcs(&header, &table);

    let cases = [
        (0.5, 5.0),
        (1.0, 10.0),
        (2.5, 17.5),
        (3.5, 22.5),
        (4.5, 37.5),
    ];
    for (pixel, expected) in cases {
        assert_eq!(wcs.pixel_to_world(&[pixel]).unwrap(), [expected]);
    }
    assert_eq!(wcs.world_to_pixel(&[17.5]).unwrap(), [1.75]);
    assert_eq!(wcs.world_to_pixel(&[37.5]).unwrap(), [4.5]);
    assert!(matches!(
        wcs.pixel_to_world(&[0.49]),
        Err(FitsError::WcsCoordinateDomain {
            axis: 0,
            algorithm: "TAB"
        })
    ));
}

#[test]
fn tab_index_vectors_may_decrease_and_change_sampling() {
    let table = lookup_table(&[
        ("COORD", &[100.0, 200.0, 400.0, 800.0], Some("(1,4)")),
        ("INDEX", &[40.0, 30.0, 10.0, 0.0], None),
    ]);
    let mut header = tab_header(1, "COORD");
    header
        .set_internal("PS1_2", "INDEX")
        .set_internal("CRVAL1", 30.0);
    let wcs = resolved_wcs(&header, &table);

    assert_eq!(wcs.pixel_to_world(&[0.0]).unwrap(), [200.0]);
    assert_eq!(wcs.pixel_to_world(&[5.0]).unwrap(), [150.0]);
    assert_eq!(wcs.pixel_to_world(&[-22.5]).unwrap(), [500.0]);
    for pixel in [5.0, 0.0, -22.5] {
        let world = wcs.pixel_to_world(&[pixel]).unwrap();
        let round_trip = wcs.world_to_pixel(&world).unwrap();
        assert!(
            (round_trip[0] - pixel).abs() < 1e-12,
            "{pixel}: {round_trip:?}"
        );
    }
}

#[test]
fn multidimensional_tab_interpolates_and_inverts_coupled_axes() {
    let table = lookup_table(&[(
        "COORD",
        &[100.0, 200.0, 110.0, 200.0, 100.0, 220.0, 110.0, 220.0],
        Some("(2,2,2)"),
    )]);
    let mut header = tab_header(2, "COORD");
    header
        .set_internal("PS1_0", "wcs-table")
        .set_internal("PS1_1", "coord");
    let wcs = resolved_wcs(&header, &table);

    let world = wcs.pixel_to_world(&[1.25, 1.5]).unwrap();
    assert_eq!(world, [102.5, 210.0]);
    let pixel = wcs.world_to_pixel(&world).unwrap();
    assert!((pixel[0] - 1.25).abs() < 1e-9, "{pixel:?}");
    assert!((pixel[1] - 1.5).abs() < 1e-9, "{pixel:?}");

    for boundary in [[0.5, 1.5], [1.25, 2.5], [0.5, 2.5]] {
        let world = wcs.pixel_to_world(&boundary).unwrap();
        let pixel = wcs.world_to_pixel(&world).unwrap();
        assert!(
            (pixel[0] - boundary[0]).abs() < 1e-9 && (pixel[1] - boundary[1]).abs() < 1e-9,
            "{boundary:?}: {world:?} -> {pixel:?}"
        );
    }
}

#[test]
fn tabular_time_axes_feed_the_typed_time_layer() {
    let table = lookup_table(&[("COORD", &[0.0, 2.0], Some("(1,2)"))]);
    let mut header = tab_header(1, "COORD");
    header
        .set_internal("CTYPE1", "TIME-TAB")
        .set_internal("CUNIT1", "d")
        .set_internal("MJDREF", 50_000.0)
        .set_internal("TIMESYS", "UTC");
    let wcs = resolved_wcs(&header, &table);
    let coordinate = header
        .time()
        .unwrap()
        .time_axis_mjd(&wcs, 1, &[1.5])
        .unwrap()
        .unwrap();
    assert_eq!(coordinate.mjd, 50_001.0);
}

#[test]
fn tabular_spectral_coordinates_normalize_declared_units() {
    let table = lookup_table(&[("COORD", &[500.0, 600.0], Some("(1,2)"))]);
    let mut header = tab_header(1, "COORD");
    header
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "nm");
    let wcs = resolved_wcs(&header, &table);
    let world = wcs.pixel_to_world(&[1.5]).unwrap();
    assert!((world[0] - 5.5e-7).abs() < 1e-20, "{world:?}");
    let pixel = wcs.world_to_pixel(&world).unwrap();
    assert!((pixel[0] - 1.5).abs() < 1e-12, "{pixel:?}");
}

#[test]
fn tab_rejects_missing_references_bad_shapes_and_nonmonotonic_indices() {
    let mut missing = Header::new();
    missing
        .set_internal("NAXIS", 1)
        .set_internal("CTYPE1", "AX01-TAB")
        .set_internal("PS1_1", "COORD");
    assert!(matches!(
        tabular::descriptors(&missing, 1, None),
        Err(FitsError::InvalidValue { .. })
    ));

    let bad_shape = lookup_table(&[("COORD", &[1.0, 2.0, 3.0, 4.0], Some("(2,2)"))]);
    let header = tab_header(1, "COORD");
    let descriptor = tabular::descriptors(&header, 1, None).unwrap().remove(0);
    assert!(matches!(
        TabularTransform::from_table(descriptor, &bad_shape),
        Err(FitsError::InvalidValue { .. })
    ));

    let bad_index = lookup_table(&[
        ("COORD", &[1.0, 2.0, 3.0], Some("(1,3)")),
        ("INDEX", &[1.0, 3.0, 2.0], None),
    ]);
    let mut header = tab_header(1, "COORD");
    header.set_internal("PS1_2", "INDEX");
    let descriptor = tabular::descriptors(&header, 1, None).unwrap().remove(0);
    assert!(matches!(
        TabularTransform::from_table(descriptor, &bad_index),
        Err(FitsError::InvalidValue { .. })
    ));
}
