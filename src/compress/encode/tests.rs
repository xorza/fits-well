use crate::compress::*;
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::data::scaling::Scaling;
use crate::error::FitsError;
use crate::error::Ranked;
use crate::header::value::Value;
use crate::reader::FitsReader;
use crate::writer::FitsWriter;
use std::fs::File;
use std::io::Cursor;

/// Emit compressed files written by this crate for external (astropy) validation.
/// Run with `cargo test --features compression -- --ignored emit_`.
#[test]
#[ignore]
fn emit_compressed_files_for_astropy() {
    let samples: Vec<i16> = (0..24 * 16)
        .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
        .collect();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::I16(samples),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    for (compression, tiles) in [
        (Compression::GZIP, &[][..]),
        (Compression::GZIP_SHUFFLED, &[]),
        (Compression::Rice, &[]),
        (Compression::Hcompress(Hcompress::default()), &[24, 16]),
    ] {
        let f = File::create(format!(
            ".tmp/wr_{}.fits",
            compression.name().to_lowercase()
        ))
        .unwrap();
        let mut w = FitsWriter::new(f);
        w.write_compressed_image(&image, compression, &CompressionOptions::tiled(tiles))
            .unwrap();
    }

    // PLIO needs a non-negative mask image.
    let mask: Vec<i32> = (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect();
    let mask_image = Image {
        shape: vec![24, 16],
        samples: ImageData::I32(mask),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let f = File::create(".tmp/wr_plio_1.fits").unwrap();
    let mut w = FitsWriter::new(f);
    w.write_compressed_image(
        &mask_image,
        Compression::Plio,
        &CompressionOptions::default(),
    )
    .unwrap();

    // Quantized float (SUBTRACTIVE_DITHER_1) for astropy to reconstruct.
    let fimage = Image {
        shape: vec![24, 16],
        samples: ImageData::F32(float_field()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let f = File::create(".tmp/wr_ricef.fits").unwrap();
    let mut w = FitsWriter::new(f);
    w.write_compressed_image(
        &fimage,
        Compression::Rice,
        &CompressionOptions::tiled([24, 16]),
    )
    .unwrap();
}

#[test]
fn compression_write_round_trips_through_decode() {
    let samples: Vec<i16> = (0..24 * 16)
        .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
        .collect();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::I16(samples.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    for (case, compression, tiles) in [
        ("GZIP_1 row tiles", Compression::GZIP, &[][..]),
        ("GZIP_1 2-D tiles", Compression::GZIP, &[7, 5]),
        ("GZIP_2 row tiles", Compression::GZIP_SHUFFLED, &[]),
        ("RICE_1 row tiles", Compression::Rice, &[]),
        (
            "HCOMPRESS_1 whole image",
            Compression::Hcompress(Hcompress::default()),
            &[24, 16],
        ),
    ] {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_compressed_image(&image, compression, &CompressionOptions::tiled(tiles))
            .unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        if matches!(compression, Compression::Hcompress(_)) {
            let header = &r.hdus()[1].header;
            assert!(matches!(header.get("ZVAL1"), Some(Value::Real(0.0))));
            assert_eq!(header.get_text("ZNAME2").unwrap(), None);
        }
        let back = r.read_image(1).unwrap();
        assert_eq!(back.shape, vec![24, 16], "{case}");
        match back.decode() {
            ImageData::I16(v) => assert_eq!(v, samples, "{case} round-trip"),
            other => panic!("{case}: expected I16, got {other:?}"),
        }
    }

    let samples_3d: Vec<i16> = (0..5 * 4 * 3).map(|i| i as i16 * 3 - 50).collect();
    let image_3d = Image {
        shape: vec![5, 4, 3],
        samples: ImageData::I16(samples_3d.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(
        &image_3d,
        Compression::GZIP,
        &CompressionOptions::tiled([3, 2, 2]),
    )
    .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = r.read_image(1).unwrap();
    assert_eq!(back.shape, vec![5, 4, 3]);
    assert!(matches!(back.decode(), ImageData::I16(v) if v == samples_3d));
}

/// A 24×16 float field: a smooth ramp plus genuine high-frequency noise (a
/// splitmix64 hash, decorrelated neighbour-to-neighbour) so the 3rd-order MAD
/// estimate is realistic (≈ 1) and the tile genuinely quantizes.
fn float_field() -> Vec<f32> {
    let mix = |i: u64| {
        // splitmix64 finalizer — uncorrelated output for consecutive inputs.
        let mut z = i.wrapping_add(0x9E37_79B9_7F4A_7C15);
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    };
    (0..24 * 16)
        .map(|i| {
            let (x, y) = (i % 24, i / 24);
            let smooth = 100.0 + 3.0 * x as f32 - 2.0 * y as f32;
            let noise = (mix(i as u64) % 2000) as f32 / 1000.0 - 1.0; // ±1.0
            smooth + noise
        })
        .collect()
}

#[test]
fn float_compression_preserves_scaling_across_quantized_and_fallback_tiles() {
    let mut f32_samples: Vec<f32> = float_field().into_iter().take(24).collect();
    f32_samples.extend(std::iter::repeat_n(42.25, 24));
    let f64_samples = f32_samples.iter().map(|&value| value as f64).collect();

    for samples in [ImageData::F32(f32_samples), ImageData::F64(f64_samples)] {
        let bitpix = samples.bitpix();
        let expected_raw: Vec<f64> = match &samples {
            ImageData::F32(values) => values.iter().map(|&value| value as f64).collect(),
            ImageData::F64(values) => values.clone(),
            _ => unreachable!("float cases only"),
        };
        let image = Image {
            shape: vec![24, 2],
            samples,
            scaling: Scaling {
                bscale: 2.5,
                bzero: -10.0,
                blank: None,
            },
        };
        for compression in [
            Compression::Rice,
            Compression::GZIP,
            Compression::GZIP_SHUFFLED,
            Compression::None,
        ] {
            let cmptype = compression.name();
            let mut w = FitsWriter::new(Cursor::new(Vec::new()));
            w.write_compressed_image(&image, compression, &CompressionOptions::tiled([24, 1]))
                .unwrap();
            let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
            let header = &r.hdus()[1].header;
            match compression {
                Compression::Rice => {
                    assert_eq!(header.get_text("ZNAME1").unwrap(), Some("BLOCKSIZE"));
                    assert_eq!(header.get_text("ZNAME2").unwrap(), Some("BYTEPIX"));
                }
                _ => {
                    assert_eq!(header.get_text("ZNAME1").unwrap(), None);
                }
            }
            assert_eq!(
                header.get_real("BSCALE").unwrap(),
                Some(2.5),
                "{bitpix:?} {cmptype}"
            );
            assert_eq!(
                header.get_real("BZERO").unwrap(),
                Some(-10.0),
                "{bitpix:?} {cmptype}"
            );

            let table = r.read_table(1).unwrap();
            let compressed = table
                .column_by_name("COMPRESSED_DATA")
                .unwrap()
                .vla()
                .unwrap();
            let fallback = table
                .column_by_name("GZIP_COMPRESSED_DATA")
                .unwrap()
                .vla()
                .unwrap();
            assert_ne!(
                compressed[0].element_count(),
                0,
                "quantized tile for {bitpix:?} {cmptype}"
            );
            assert_eq!(
                fallback[0].element_count(),
                0,
                "quantized tile for {bitpix:?} {cmptype}"
            );
            assert_eq!(
                compressed[1].element_count(),
                0,
                "fallback tile for {bitpix:?} {cmptype}"
            );
            assert_ne!(
                fallback[1].element_count(),
                0,
                "fallback tile for {bitpix:?} {cmptype}"
            );

            let back = r.read_image(1).unwrap();
            assert_eq!(back.scaling, image.scaling, "{bitpix:?} {cmptype}");
            let physical = back.physical();
            let actual_raw: Vec<f64> = match back.decode() {
                ImageData::F32(values) => values.into_iter().map(|value| value as f64).collect(),
                ImageData::F64(values) => values,
                other => panic!("{cmptype}: expected {bitpix:?}, got {other:?}"),
            };
            let mut quantized_changed = false;
            for (index, (&expected, &actual)) in expected_raw.iter().zip(&actual_raw).enumerate() {
                let expected_physical = -10.0 + 2.5 * expected;
                if index < 24 {
                    let raw_error = (actual - expected).abs();
                    assert!(
                        raw_error < 0.2,
                        "{bitpix:?} {cmptype} raw pixel {index}: {raw_error}"
                    );
                    let physical_error = (physical[index] - expected_physical).abs();
                    assert!(
                        physical_error < 0.5,
                        "{bitpix:?} {cmptype} physical pixel {index}: {physical_error}"
                    );
                    quantized_changed |= actual != expected;
                } else {
                    assert_eq!(
                        actual, expected,
                        "{bitpix:?} {cmptype} fallback raw pixel {index}"
                    );
                    assert_eq!(
                        physical[index], expected_physical,
                        "{bitpix:?} {cmptype} fallback physical pixel {index}"
                    );
                }
            }
            assert!(
                quantized_changed,
                "{bitpix:?} {cmptype} did not quantize the noisy tile"
            );
        }
    }
}

#[test]
fn hcompress_writer_enforces_standard_image_constraints() {
    let line = Image::new(vec![4], vec![1i16, 2, 3, 4]).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_image(
            &line,
            Compression::Hcompress(Hcompress::default()),
            &CompressionOptions::default(),
        ),
        Err(FitsError::UnsupportedCompression { name })
            if name == "HCOMPRESS_1 requires a two-dimensional image"
    ));

    let float = Image::new(vec![4, 1], vec![1.0f32, 2.0, 3.0, 4.0]).unwrap();
    assert!(matches!(
        writer.write_compressed_image(
            &float,
            Compression::Hcompress(Hcompress::lossy(2.0).unwrap()),
            &CompressionOptions::default(),
        ),
        Err(FitsError::UnsupportedCompression { name })
            if name == "HCOMPRESS_1 for float images (write)"
    ));

    let extreme = Image::new(vec![2, 2], vec![i64::MAX; 4]).unwrap();
    assert!(matches!(
        writer.write_compressed_image(
            &extreme,
            Compression::Hcompress(Hcompress::default()),
            &CompressionOptions::tiled([2, 2]),
        ),
        Err(FitsError::UnsupportedCompression { name })
            if name == "HCOMPRESS_1 tile exceeds the signed 64-bit stream range"
    ));

    let undefined = Image::new_scaled(
        vec![8, 8],
        (0..64)
            .map(|index| {
                let x = index % 8;
                x * x
            })
            .collect::<Vec<i32>>(),
        Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: Some(0),
        },
    )
    .unwrap();
    assert!(matches!(
        writer.write_compressed_image(
            &undefined,
            Compression::Hcompress(Hcompress::lossy(2.0).unwrap()),
            &CompressionOptions::tiled([8, 8]),
        ),
        Err(FitsError::UnsupportedCompression { name })
            if name == "lossy HCOMPRESS_1 with undefined pixels requires a null mask"
    ));
}

#[test]
fn dither_option_sets_zquantiz_and_round_trips() {
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::F32(float_field()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    // Each `DitherMethod` writes its `ZQUANTIZ` keyword and round-trips; the option is
    // honored rather than always emitting the hardcoded SUBTRACTIVE_DITHER_1.
    for (dither, zquantiz) in [
        (DitherMethod::None, "NO_DITHER"),
        (DitherMethod::Subtractive1, "SUBTRACTIVE_DITHER_1"),
        (DitherMethod::Subtractive2, "SUBTRACTIVE_DITHER_2"),
    ] {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        let options = CompressionOptions::tiled([24, 16])
            .with_quantization(0.0, dither)
            .unwrap();
        w.write_compressed_image(&image, Compression::Rice, &options)
            .unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        assert_eq!(
            r.hdus()[1].header.get_text("ZQUANTIZ").unwrap(),
            Some(zquantiz),
            "{dither:?} must write {zquantiz}"
        );
        match r.read_image(1).unwrap().decode() {
            ImageData::F32(v) => assert_eq!(v.len(), 24 * 16, "{dither:?}"),
            other => panic!("{dither:?}: expected F32, got {other:?}"),
        }
    }
}

#[test]
fn tile_shape_with_wrong_rank_is_rejected() {
    // A 2-D image with a 3-axis tile shape: a wrong-rank request errors rather than
    // silently row-tiling (an empty shape is still the row-tiling default).
    let image = Image {
        shape: vec![4, 3],
        samples: ImageData::I16((0..12).map(|i| i as i16).collect()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let err = w.write_compressed_image(
        &image,
        Compression::Rice,
        &CompressionOptions::tiled([2, 2, 2]),
    );
    assert!(
        matches!(
            err,
            Err(FitsError::RankMismatch {
                ranked: Ranked::TileShape,
                expected: 2,
                got: 3,
            })
        ),
        "got {err:?}"
    );
}

#[test]
fn float_write_preserves_nan_nulls() {
    let mut orig = float_field();
    orig[5 + 3 * 24] = f32::NAN;
    orig[20 + 10 * 24] = f32::NAN;
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::F32(orig.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(
        &image,
        Compression::Rice,
        &CompressionOptions::tiled([24, 16]),
    )
    .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = match r.read_image(1).unwrap().decode() {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    for (i, (&o, &b)) in orig.iter().zip(&back).enumerate() {
        if o.is_nan() {
            assert!(b.is_nan(), "null pixel {i} must round-trip to NaN");
        } else {
            assert!((o - b).abs() < 0.2, "pixel {i}: {o} vs {b}");
        }
    }
}

#[test]
fn hcompress_writer_converts_noise_multiplier_to_tile_scale() {
    // For f(x,y)=x²+3y, every third-order row difference is exactly 8, so the
    // noise estimate is 0.6052697×8. A SCALE multiplier of 2 therefore produces
    // the absolute HCOMPRESS stream scale round(9.6843152)=10.
    let samples: Vec<i32> = (0..8 * 8)
        .map(|index| {
            let x = index % 8;
            let y = index / 8;
            x * x + 3 * y
        })
        .collect();
    let image = Image::new(vec![8, 8], samples).unwrap();
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(
        &image,
        Compression::Hcompress(Hcompress::lossy(2.0).unwrap()),
        &CompressionOptions::tiled([8, 8]),
    )
    .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    assert!(matches!(
        r.hdus()[1].header.get("ZVAL1"),
        Some(Value::Real(2.0))
    ));
    let table = r.read_table(1).unwrap();
    let stream = table
        .column_by_name("COMPRESSED_DATA")
        .unwrap()
        .vla_column()
        .unwrap()
        .cell(0)
        .unwrap()
        .bytes;
    assert_eq!(&stream[..2], &[0xDD, 0x99]);
    assert_eq!(i32::from_be_bytes(stream[10..14].try_into().unwrap()), 10);
}

#[test]
fn hcompress_lossless_write_round_trips_exactly() {
    let samples: Vec<i32> = (0..16 * 16)
        .map(|index| {
            let x = index % 16;
            let y = index / 16;
            x * x - 3 * y
        })
        .collect();
    let image = Image::new(vec![16, 16], samples.clone()).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(
            &image,
            Compression::Hcompress(Hcompress::default()),
            &CompressionOptions::tiled([16, 16]),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    match reader.read_image(1).unwrap().decode() {
        ImageData::I32(actual) => assert_eq!(actual, samples),
        other => panic!("expected I32, got {other:?}"),
    };

    let wide = vec![i32::MAX, i32::MAX, i32::MAX, i32::MAX];
    let image = Image::new(vec![2, 2], wide.clone()).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(
            &image,
            Compression::Hcompress(Hcompress::default()),
            &CompressionOptions::tiled([2, 2]),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(reader.read_image(1).unwrap().decode(), ImageData::I32(wide));

    let wide = vec![
        1i64 << 40,
        -(1i64 << 40),
        (1i64 << 40) + 1,
        -(1i64 << 40) + 2,
    ];
    let image = Image::new(vec![2, 2], wide.clone()).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(
            &image,
            Compression::Hcompress(Hcompress::default()),
            &CompressionOptions::tiled([2, 2]),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(reader.read_image(1).unwrap().decode(), ImageData::I64(wide));
}

#[test]
fn plio_write_round_trips_through_decode() {
    // PLIO is a mask codec: non-negative i32 values. value(x, y) = (x + y) % 7,
    // with a few longer runs to exercise multi-word counts.
    let samples: Vec<i32> = (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::I32(samples.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(&image, Compression::Plio, &CompressionOptions::default())
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    match r.read_image(1).unwrap().decode() {
        ImageData::I32(v) => assert_eq!(v, samples, "PLIO_1 round-trip"),
        other => panic!("PLIO_1: expected I32, got {other:?}"),
    }
}

#[test]
fn integer_image_compression_preserves_bscale_bzero_and_blank() {
    // §10.2: the compressed tiles store *raw* stored integers, so BSCALE/BZERO and
    // the BLANK sentinel must survive in the rebuilt header (was dropped before).
    let samples: Vec<i16> = (0..24 * 16).map(|i| (i % 50) as i16 - 5).collect();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::I16(samples.clone()),
        scaling: Scaling {
            bscale: 2.5,
            bzero: 100.0,
            blank: Some(-5),
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(&image, Compression::GZIP, &CompressionOptions::default())
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = r.read_image(1).unwrap();
    assert_eq!(back.scaling.bscale, 2.5);
    assert_eq!(back.scaling.bzero, 100.0);
    assert_eq!(back.scaling.blank, Some(-5));

    match back.decode() {
        ImageData::I16(v) => assert_eq!(v, samples, "raw samples"),
        other => panic!("expected I16, got {other:?}"),
    }
}

#[test]
fn rice_64_bit_pixels_round_trip_extreme_differences() {
    let samples = vec![
        i64::MIN,
        i64::MAX,
        0,
        -1,
        1,
        9_007_199_254_740_993,
        -9_007_199_254_740_993,
        i64::MIN + 1,
        i64::MAX - 1,
    ];
    let image = Image {
        shape: vec![samples.len()],
        samples: ImageData::I64(samples.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(&image, Compression::Rice, &CompressionOptions::default())
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let decoded = r.read_image(1).unwrap().decode();
    assert_eq!(decoded, ImageData::I64(samples));
}

#[test]
fn nocompress_image_round_trips() {
    // §10.4: tiles stored verbatim (uncompressed big-endian pixels) round-trip.
    let samples: Vec<i16> = (0..24 * 16)
        .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
        .collect();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::I16(samples.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(&image, Compression::None, &CompressionOptions::default())
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    match r.read_image(1).unwrap().decode() {
        ImageData::I16(v) => assert_eq!(v, samples),
        other => panic!("expected I16, got {other:?}"),
    }
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_full_decode_crosses_the_bounded_wave_boundary() {
    let samples: Vec<u8> = (0usize..1024 * 4097)
        .map(|index| (index.wrapping_mul(37) & 0xff) as u8)
        .collect();
    let image = Image::new(vec![1024, 4097], samples.clone()).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(&image, Compression::None, &CompressionOptions::default())
        .unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        reader.read_image(1).unwrap().decode(),
        ImageData::U8(samples)
    );
}

#[test]
fn empty_naxis0_image_round_trips() {
    // A `NAXIS = 0` image has no data array. The encoder must emit an empty
    // `NAXIS2 = 0` ZIMAGE table (not panic on a fabricated phantom tile), and the
    // decoder must restore the empty image. Exercise both the integer and float
    // encoder paths.
    let cases = [
        ImageData::I16(Vec::new()),
        ImageData::I32(Vec::new()),
        ImageData::F32(Vec::new()),
    ];
    for samples in cases {
        let image = Image {
            shape: Vec::new(),
            samples: samples.clone(),
            scaling: Scaling {
                bscale: 1.0,
                bzero: 0.0,
                blank: None,
            },
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_compressed_image(&image, Compression::GZIP, &CompressionOptions::default())
            .unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        let back = r.read_image(1).unwrap();
        assert!(back.shape.is_empty(), "shape for {samples:?}");
        // Same empty variant back out, no phantom pixel.
        match (back.decode(), &samples) {
            (ImageData::I16(v), ImageData::I16(_)) => assert!(v.is_empty()),
            (ImageData::I32(v), ImageData::I32(_)) => assert!(v.is_empty()),
            (ImageData::F32(v), ImageData::F32(_)) => assert!(v.is_empty()),
            (other, _) => panic!("variant mismatch: {other:?}"),
        }
    }
}

#[test]
fn empty_first_axis_image_round_trips() {
    let image = Image {
        shape: vec![0],
        samples: ImageData::I16(Vec::new()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(&image, Compression::GZIP, &CompressionOptions::default())
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = r.read_image(1).unwrap();
    assert_eq!(back.shape, [0]);
    assert!(matches!(back.decode(), ImageData::I16(v) if v.is_empty()));
}
