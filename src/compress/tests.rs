use crate::compress::quantization::{Dither, DitherSeed, Seed};
use crate::compress::*;

#[test]
fn typed_compression_configuration_rejects_impossible_values() {
    assert_eq!(
        [Gzip::new(9).unwrap(), Gzip::shuffled(0).unwrap()],
        [
            Gzip {
                form: GzipForm::Plain,
                level: 9
            },
            Gzip {
                form: GzipForm::Shuffled,
                level: 0
            }
        ]
    );
    assert!(matches!(
        Gzip::new(10),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::GzipLevel { level: 10 }
        })
    ));
    for value in [0.0, -1.0, f64::NAN, f64::INFINITY] {
        assert!(matches!(
            Hcompress::relative(value),
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::HcompressScale { scale }
            }) if scale.to_bits() == value.to_bits()
        ));
        assert!(matches!(
            Hcompress::absolute(value),
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::HcompressScale { scale }
            }) if scale.to_bits() == value.to_bits()
        ));
        assert!(matches!(
            Quantization::relative(value, Dither::None),
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::QuantizationLevel { level }
            }) if level.to_bits() == value.to_bits()
        ));
        assert!(matches!(
            Quantization::absolute(value, Dither::None),
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::QuantizationStep { step }
            }) if step.to_bits() == value.to_bits()
        ));
    }
    assert_eq!(
        [
            Hcompress::relative(1.0).unwrap(),
            Hcompress::absolute(30.0).unwrap()
        ],
        [
            Hcompress {
                scale: HcompressScale::Relative(1.0)
            },
            Hcompress {
                scale: HcompressScale::Absolute(30.0)
            }
        ]
    );
    for seed in [0, 10_001] {
        assert!(matches!(
            DitherSeed::fixed(seed),
            Err(FitsError::InvalidCompressionParameter {
                parameter: CompressionParameter::DitherSeed { seed: got }
            }) if got == seed
        ));
    }
    for seed in [1, 10_000] {
        assert_eq!(DitherSeed::fixed(seed).unwrap().source(), Seed::Fixed(seed));
    }
    assert_eq!(
        Quantization::default(),
        Quantization::relative(4.0, Dither::Subtractive1(DitherSeed::FROM_SAMPLES)).unwrap()
    );
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
    // A set height is cut to at most row_count, and an empty table still has a height
    // of 1; a height of 0 is refused, as an image's tile of 0 is.
    let tiled = |rows: usize| rice.clone().with_tiles([rows]);
    assert_eq!(tiled(3).table_tile_rows(10, 8).unwrap(), 3);
    assert!(matches!(
        tiled(0).table_tile_rows(10, 8),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::EmptyTile { axis: 0 }
        })
    ));
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
        rice.with_quantization(Quantization::relative(4.0, Dither::None).unwrap())
            .table_tile_rows(10, 8),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::TableQuantization
        })
    ));
}

#[test]
fn compression_parameter_indices_use_the_standard_canonical_form() {
    assert_eq!(parameter_index(Keyword::new("ZNAME1")), Some(1));
    assert_eq!(parameter_index(Keyword::new("ZNAME999")), Some(999));
    for keyword in ["ZNAME", "ZNAME0", "ZNAME01", "ZNAMEA", "ZVAL1"] {
        assert_eq!(parameter_index(Keyword::new(keyword)), None, "{keyword}");
    }
}
