use crate::compress::*;

#[test]
fn typed_compression_configuration_rejects_impossible_values() {
    assert!(Gzip::new(9).is_ok());
    assert!(Gzip::shuffled(0).is_ok());
    assert!(matches!(
        Gzip::new(10),
        Err(FitsError::InvalidCompressionParameter { detail }) if detail == "gzip compression level 10 is outside 0..=9"
    ));
    assert!(matches!(
        Hcompress::lossy(0.0),
        Err(FitsError::InvalidCompressionParameter { detail }) if detail == "lossy HCOMPRESS scale 0 must be positive"
    ));
    assert!(matches!(
        Hcompress::lossy(f64::NAN),
        Err(FitsError::InvalidCompressionParameter { .. })
    ));
    assert!(matches!(
        CompressionConfig::new(Compression::Rice).with_quantization(f64::NAN, DitherMethod::None),
        Err(FitsError::InvalidCompressionParameter { .. })
    ));
    assert!(matches!(
        CompressionConfig::new(Compression::Rice).with_quantization(-1.0, DitherMethod::None),
        Err(FitsError::InvalidCompressionParameter { .. })
    ));
    assert_eq!(Compression::GZIP.name(), "GZIP_1");
    assert_eq!(Compression::GZIP_SHUFFLED.name(), "GZIP_2");
}

#[test]
fn table_tile_rows_follow_the_config_and_cfitsio_default() {
    let rice = CompressionConfig::new(Compression::Rice);
    // cfitsio's default: 10 000 000 / 40 = 250 000 rows, cut to the 1000 the table has;
    // 10 000 000 / 4000 = 2500 rows within 1 000 000.
    assert_eq!(rice.table_tile_rows(1000, 40).unwrap(), 1000);
    assert_eq!(rice.table_tile_rows(1_000_000, 4000).unwrap(), 2500);
    // Rows of no bytes have no default height: the whole table is one tile.
    assert_eq!(rice.table_tile_rows(7, 0).unwrap(), 7);
    // A set height is cut to 1..=row_count; an empty table still has a height of 1.
    let tiled = |rows: usize| rice.clone().with_tiles([rows]);
    assert_eq!(tiled(3).table_tile_rows(10, 8).unwrap(), 3);
    assert_eq!(tiled(0).table_tile_rows(10, 8).unwrap(), 1);
    assert_eq!(tiled(30).table_tile_rows(10, 8).unwrap(), 10);
    assert_eq!(tiled(3).table_tile_rows(0, 8).unwrap(), 1);

    assert!(matches!(
        rice.clone().with_tiles([3, 2]).table_tile_rows(10, 8),
        Err(FitsError::RankMismatch {
            ranked: Ranked::TileShape,
            expected: 1,
            got: 2,
        })
    ));
    assert!(matches!(
        rice.with_quantization(4.0, DitherMethod::None)
            .unwrap()
            .table_tile_rows(10, 8),
        Err(FitsError::InvalidCompressionParameter { detail })
            if detail == "a compressed table takes no float quantization"
    ));
}

#[test]
fn compression_parameter_indices_use_the_standard_canonical_form() {
    assert_eq!(parameter_index("ZNAME1"), Some(1));
    assert_eq!(parameter_index("ZNAME999"), Some(999));
    for keyword in ["ZNAME", "ZNAME0", "ZNAME01", "ZNAME1000", "ZNAMEA"] {
        assert_eq!(parameter_index(keyword), None, "{keyword}");
    }
}
