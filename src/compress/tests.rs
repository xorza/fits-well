use crate::bitpix::Bitpix;
use crate::compress;
use crate::compress::convert::{be_to_i64_into, i32_to_be_into, i64_to_be, i64_to_be_into};
use crate::compress::tile_geometry::TileGeometry;
use crate::compress::tile_geometry::TileScratch;
use crate::compress::*;
use crate::data::image_data::ImageData;
use crate::endian::write_pq_descriptor;
use crate::error::Ranked;
use crate::keyword::key;
use crate::reader::{FitsReader, StreamReader};
use crate::table_impl::BinTable;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::tform_kind::TformKind;
use std::fs::File;
use std::io::Cursor;

fn open(name: &str) -> StreamReader<File> {
    FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap()
}

/// The fixtures encode value(x, y) = x*7 − y*5 over a 24×16 i16 image.
fn expect_pixel(flat: usize) -> i16 {
    let (x, y) = (flat % 24, flat / 24);
    (x as i16) * 7 - (y as i16) * 5
}

fn check_decoded(name: &str) {
    let mut f = open(name);
    let img = f.read_image(1).unwrap();
    assert_eq!(img.shape, vec![24, 16]);
    match img.decode() {
        ImageData::I16(v) => {
            assert_eq!(v.len(), 24 * 16);
            for (i, &got) in v.iter().enumerate() {
                assert_eq!(got, expect_pixel(i), "pixel {i} of {name}");
            }
        }
        other => panic!("expected I16, got {other:?}"),
    }
}

#[test]
fn decompresses_gzip_1_tiled_image() {
    check_decoded("comp_gzip_i16.fits");
}

#[test]
fn typed_compression_configuration_rejects_impossible_values() {
    assert!(Gzip::new(9).is_ok());
    assert!(Gzip::shuffled(0).is_ok());
    assert!(matches!(
        Gzip::new(10),
        Err(FitsError::InvalidValue { card }) if card == "gzip compression level 10 is outside 0..=9"
    ));
    assert!(matches!(
        Hcompress::lossy(0.0),
        Err(FitsError::InvalidValue { card }) if card == "lossy HCOMPRESS scale 0 must be positive"
    ));
    assert!(matches!(
        Hcompress::lossy(f64::NAN),
        Err(FitsError::InvalidValue { .. })
    ));
    assert!(matches!(
        CompressionOptions::default().with_quantization(f64::NAN, DitherMethod::None),
        Err(FitsError::InvalidValue { .. })
    ));
    assert_eq!(Compression::GZIP.name(), "GZIP_1");
    assert_eq!(Compression::GZIP_SHUFFLED.name(), "GZIP_2");
}

#[test]
fn compression_parameter_indices_use_the_standard_canonical_form() {
    assert_eq!(compress::parameter_index("ZNAME1"), Some(1));
    assert_eq!(compress::parameter_index("ZNAME999"), Some(999));
    for keyword in ["ZNAME", "ZNAME0", "ZNAME01", "ZNAME1000", "ZNAMEA"] {
        assert_eq!(compress::parameter_index(keyword), None, "{keyword}");
    }
}

#[test]
fn decompresses_rice_1_tiled_image() {
    check_decoded("comp_rice_i16.fits");
}

#[test]
fn decompresses_hcompress_1_tiled_image() {
    // Lossless HCOMPRESS (SCALE=0), single 24×16 tile.
    check_decoded("comp_hcomp_i16.fits");
}

/// Decode an i32 image and compare pixel-exact against astropy's reconstruction
/// stored as a plain-image reference.
fn check_i32_against_ref(compressed: &str, reference: &str) {
    let got = match open(compressed).read_image(1).unwrap().decode() {
        ImageData::I32(v) => v,
        other => panic!("expected I32, got {other:?}"),
    };
    let want = match open(reference).read_image(0).unwrap().decode() {
        ImageData::I32(v) => v,
        other => panic!("expected I32 reference, got {other:?}"),
    };
    assert_eq!(got, want, "{compressed} must match astropy {reference}");
}

#[test]
fn decompresses_hcompress_lossy() {
    // Lossy HCOMPRESS (SCALE=4, SMOOTH=0): exercises undigitize (×scale).
    check_i32_against_ref("comp_hcomp_lossy.fits", "comp_ref_hcomp_lossy.fits");
}

#[test]
fn decompresses_hcompress_smoothed() {
    // SMOOTH=1: the SMOOTH ZVAL triggers inverse-transform smoothing, which must
    // reproduce astropy's smoothed reconstruction bit-for-bit.
    check_i32_against_ref("comp_hcomp_smooth.fits", "comp_ref_hcomp_smooth.fits");
}

#[test]
fn decompresses_subtractive_dither_2() {
    // SUBTRACTIVE_DITHER_2 float: must match astropy's dithered reconstruction.
    check_float("comp_dither2_f32.fits", "comp_ref_dither2_f32.fits");
}

#[test]
fn decompresses_float_with_nan_nulls() {
    // SUBTRACTIVE_DITHER_1 with ZBLANK: null pixels decode to NaN, the rest match.
    let got = match open("comp_nan_f32.fits").read_image(1).unwrap().decode() {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open("comp_ref_nan_f32.fits")
        .read_image(0)
        .unwrap()
        .decode()
    {
        ImageData::F32(v) => v,
        other => panic!("expected F32 reference, got {other:?}"),
    };
    assert_eq!(got.len(), want.len());
    let mut nan_count = 0;
    for (i, (&g, &w)) in got.iter().zip(&want).enumerate() {
        if w.is_nan() {
            assert!(g.is_nan(), "pixel {i} should be NaN");
            nan_count += 1;
        } else {
            assert_eq!(g, w, "pixel {i}");
        }
    }
    assert_eq!(nan_count, 2, "expected 2 null pixels");
}

/// Emit compressed files written by this crate for external (astropy) validation.
/// Run with `cargo test --features compression -- --ignored emit_`.
#[test]
#[ignore]
fn emit_compressed_files_for_astropy() {
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::fs::File;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
            use crate::header::value::Value;
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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
    use crate::data::Image;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::error::FitsError;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
fn dither2_quantize_round_trips() {
    use crate::compress::DitherMethod;
    use crate::compress::quantize::{QuantizeScratch, ZERO_VALUE, dequantize_into, quantize_tile};

    // 8×8 field with genuine noise and a scattering of exact zeros.
    let mut data: Vec<f64> = (0..64)
        .map(|i| {
            let mut z = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15);
            z ^= z >> 31;
            10.0 + (z % 1000) as f64 / 100.0
        })
        .collect();
    for &k in &[0usize, 13, 27, 40, 63] {
        data[k] = 0.0;
    }
    let irow = 7;
    let mut scratch = QuantizeScratch::default();
    let q = quantize_tile(
        &data,
        8,
        8,
        0.0,
        DitherMethod::Subtractive2,
        irow,
        &mut scratch,
    )
    .unwrap();
    // Exact zeros must encode to the reserved ZERO_VALUE.
    for &k in &[0usize, 13, 27, 40, 63] {
        assert_eq!(scratch.ints[k], ZERO_VALUE, "zero pixel {k}");
    }
    let ints: Vec<i64> = scratch.ints.iter().map(|&v| v as i64).collect();
    let mut back = Vec::new();
    dequantize_into(
        &ints,
        q.bscale,
        q.bzero,
        DitherMethod::Subtractive2,
        irow,
        None,
        &mut back,
    );
    for (i, (&o, &b)) in data.iter().zip(&back).enumerate() {
        if o == 0.0 {
            assert_eq!(b, 0.0, "zero pixel {i} must decode to exactly 0.0");
        } else {
            assert!(
                (o - b).abs() <= 0.5 * q.bscale + 1e-9,
                "pixel {i}: {o} vs {b}"
            );
        }
    }

    // Nulls are skipped while retaining the finite-value order. The two third-order
    // differences are |2·4 − 1 − 16| = 9 and |2·8 − 2 − 32| = 18; the lower row
    // median is therefore 9, giving the exact qlevel=1 scale below.
    let with_nulls = [
        1.0,
        f64::NAN,
        2.0,
        f64::INFINITY,
        4.0,
        8.0,
        f64::NEG_INFINITY,
        16.0,
        32.0,
    ];
    let q = quantize_tile(
        &with_nulls,
        with_nulls.len(),
        1,
        1.0,
        DitherMethod::None,
        1,
        &mut scratch,
    )
    .unwrap();
    assert_eq!(q.bscale, 0.6052697 * 9.0);

    // Native i32 Rice input must produce the identical bitstream as the former
    // widened representation.
    let widened: Vec<i64> = scratch.ints.iter().map(|&value| value as i64).collect();
    let mut rice_scratch = rice::RiceScratch::default();
    let native = rice::rice_encode(&scratch.ints, 4, 32, &mut rice_scratch);
    let widened = rice::rice_encode(&widened, 4, 32, &mut rice_scratch);
    assert_eq!(native, widened);
}

#[test]
fn hcompress_writer_converts_noise_multiplier_to_tile_scale() {
    use crate::data::Image;
    use crate::header::value::Value;
    use crate::writer::FitsWriter;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::writer::FitsWriter;

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
fn hcompress_64_bit_transform_matches_external_golden() {
    const RAW: &[u8] = include_bytes!("../../tests/data/fits/hcomp_wide_i32.raw");
    const COMPRESSED: &[u8] = include_bytes!("../../tests/data/fits/hcomp_wide_i32.huf");

    let values: Vec<i64> = RAW
        .chunks_exact(4)
        .map(|bytes| i32::from_be_bytes(bytes.try_into().unwrap()) as i64)
        .collect();
    assert_eq!(values.len(), 100 * 100);
    assert!(
        i64::from_be_bytes(COMPRESSED[14..22].try_into().unwrap()) > i32::MAX as i64,
        "the golden must require a wide H-transform accumulator"
    );
    assert!(
        COMPRESSED[22..25].iter().any(|&planes| planes > 31),
        "the golden must require more than 31 coefficient bit planes"
    );

    let mut scratch = hcompress::HcompressScratch::default();
    let encoded = hcompress::hcompress_tile_encode(&values, &[100, 100], 0, &mut scratch).unwrap();
    assert_eq!(encoded, COMPRESSED);

    let mut decoded = Vec::new();
    hcompress::hcompress_tile_into(COMPRESSED, false, values.len(), &mut decoded, &mut scratch)
        .unwrap();
    assert_eq!(decoded, values);
}

#[test]
fn plio_write_round_trips_through_decode() {
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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
fn decompresses_gzip_2_tiled_image() {
    check_decoded("comp_gzip2_i16.fits");
}

#[test]
fn decompresses_plio_1_mask() {
    // PLIO fixture encodes value(x, y) = (x + y) % 7 as an i32 mask.
    let mut f = open("comp_plio_i32.fits");
    let img = f.read_image(1).unwrap();
    assert_eq!(img.shape, vec![24, 16]);
    match img.decode() {
        ImageData::I32(v) => {
            assert_eq!(v.len(), 24 * 16);
            for (i, &got) in v.iter().enumerate() {
                let (x, y) = (i % 24, i / 24);
                assert_eq!(got, ((x + y) % 7) as i32, "pixel {i}");
            }
        }
        other => panic!("expected I32, got {other:?}"),
    }
}

/// Compare a compressed-float decode against astropy's reconstructed reference.
fn check_float(compressed: &str, reference: &str) {
    let got = match open(compressed).read_image(1).unwrap().decode() {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open(reference).read_image(0).unwrap().decode() {
        ImageData::F32(v) => v,
        other => panic!("expected F32 reference, got {other:?}"),
    };
    assert_eq!(got.len(), 24 * 16);
    assert_eq!(got, want, "{compressed} must match astropy");
}

#[test]
fn decompresses_unquantized_float_via_gzip_fallback() {
    // Smooth data stored losslessly: ZSCALE=0, raw floats gzip'd in
    // GZIP_COMPRESSED_DATA (COMPRESSED_DATA empty).
    check_float("comp_ricef_nodither.fits", "comp_ref_f32.fits");
}

#[test]
fn decompresses_quantized_float_no_dither() {
    // Noisy data genuinely quantized: per-tile ZSCALE≠0, integers RICE-packed in
    // COMPRESSED_DATA, dequantized as ZSCALE·int + ZZERO.
    check_float("comp_ricef_quant.fits", "comp_ref_quant_f32.fits");
}

/// Build a fixed-width BINTABLE, write it, then round-trip it through table
/// compression with `algo`/`rows_per_tile` and assert the data is byte-identical.
#[test]
fn decompresses_nocompress_tile_verbatim() {
    use crate::data::image_data::ImageData;
    use crate::header::Header;
    use crate::table_impl::BinTable;
    // A 2×2 i16 image as a single NOCOMPRESS tile: the COMPRESSED_DATA cell holds
    // the four pixels verbatim as big-endian i16.
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8) // one 1P descriptor
        .set_internal("NAXIS2", 1) // one tile
        .set_internal("PCOUNT", 8) // heap = 8 raw bytes
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1PB(8)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "NOCOMPRESS")
        .set_internal("ZBITPIX", 16)
        .set_internal("ZNAXIS", 2)
        .set_internal("ZNAXIS1", 2)
        .set_internal("ZNAXIS2", 2)
        .set_internal("ZTILE1", 2)
        .set_internal("ZTILE2", 2);
    let mut data = Vec::new();
    data.extend_from_slice(&8i32.to_be_bytes()); // descriptor nelem = 8 bytes
    data.extend_from_slice(&0i32.to_be_bytes()); // descriptor offset = 0
    for x in [1i16, 2, 3, 4] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&h, data).unwrap();
    let img = decode::decompress_image(&h, &table).unwrap();
    assert_eq!(img.shape, vec![2, 2]);
    assert_eq!(img.samples, ImageData::I16(vec![1, 2, 3, 4]));

    let mut invalid_hcompress = h.clone();
    invalid_hcompress
        .set_internal("ZCMPTYPE", "HCOMPRESS_1")
        .set_internal("ZNAXIS", 1)
        .set_internal("ZNAXIS1", 4);
    assert!(matches!(
        decode::decompress_image(&invalid_hcompress, &table),
        Err(FitsError::UnsupportedCompression { name })
            if name == "HCOMPRESS_1 requires a two-dimensional image"
    ));
}

#[test]
fn compressed_integer_null_mask_restores_blank_pixels() {
    use crate::data::image_data::ImageData;
    use crate::header::Header;
    use crate::table_impl::BinTable;

    let gzip2 = gzip::gzip2_encode(
        &[0, 1],
        1,
        gzip::DEFAULT_GZIP_LEVEL,
        &mut gzip::GzipScratch::default(),
    );
    let rice = rice::rice_encode(&[0i64, 1], 1, 32, &mut rice::RiceScratch::default());
    let plio = plio::plio_encode(&[0i64, 1])
        .unwrap()
        .into_iter()
        .flat_map(i16::to_be_bytes)
        .collect();
    for (codec, mask) in [
        (
            "GZIP_1",
            gzip::gzip_encode(&[0, 1], gzip::DEFAULT_GZIP_LEVEL),
        ),
        ("GZIP_2", gzip2),
        ("RICE_1", rice),
        ("PLIO_1", plio),
        ("NOCOMPRESS", vec![0, 1]),
    ] {
        let mut h = Header::new();
        h.set_internal("XTENSION", "BINTABLE")
            .set_internal("BITPIX", 8)
            .set_internal("NAXIS", 2)
            .set_internal("NAXIS1", 16)
            .set_internal("NAXIS2", 1)
            .set_internal("PCOUNT", 4 + mask.len() as i64)
            .set_internal("GCOUNT", 1)
            .set_internal("TFIELDS", 2)
            .set_internal("TFORM1", "1PB(4)")
            .set_internal("TTYPE1", "COMPRESSED_DATA")
            .set_internal("TFORM2", format!("1PB({})", mask.len()))
            .set_internal("TTYPE2", "NULL PIXEL MASK")
            .set_internal("ZIMAGE", true)
            .set_internal("ZCMPTYPE", "NOCOMPRESS")
            .set_internal("ZMASKCMP", codec)
            .set_internal("ZBITPIX", 16)
            .set_internal("ZNAXIS", 2)
            .set_internal("ZNAXIS1", 2)
            .set_internal("ZNAXIS2", 1)
            .set_internal("ZTILE1", 2)
            .set_internal("ZTILE2", 1)
            .set_internal("BLANK", -999);
        let mut data = Vec::new();
        data.extend_from_slice(&4i32.to_be_bytes());
        data.extend_from_slice(&0i32.to_be_bytes());
        data.extend_from_slice(&(mask.len() as i32).to_be_bytes());
        data.extend_from_slice(&4i32.to_be_bytes());
        data.extend_from_slice(&10i16.to_be_bytes());
        data.extend_from_slice(&20i16.to_be_bytes());
        data.extend_from_slice(&mask);
        let table = BinTable::from_data(&h, data).unwrap();
        assert_eq!(
            decode::decompress_image(&h, &table).unwrap().samples,
            ImageData::I16(vec![10, -999]),
            "{codec}"
        );

        let mut missing_blank = h.clone();
        missing_blank.remove_all("BLANK");
        assert!(matches!(
            decode::decompress_image(&missing_blank, &table),
            Err(FitsError::MissingKeyword { name: "BLANK" })
        ));
    }
}

#[test]
fn compressed_float_null_mask_restores_nan_pixels() {
    use crate::data::image_data::ImageData;
    use crate::header::Header;
    use crate::table_impl::BinTable;

    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 32)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 10)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 4)
        .set_internal("TFORM1", "1PB(8)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("TFORM2", "1PB(2)")
        .set_internal("TTYPE2", "NULL_PIXEL_MASK")
        .set_internal("TFORM3", "1D")
        .set_internal("TTYPE3", "ZSCALE")
        .set_internal("TFORM4", "1D")
        .set_internal("TTYPE4", "ZZERO")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "NOCOMPRESS")
        .set_internal("ZMASKCMP", "NOCOMPRESS")
        .set_internal("ZBITPIX", -32)
        .set_internal("ZNAXIS", 2)
        .set_internal("ZNAXIS1", 2)
        .set_internal("ZNAXIS2", 1)
        .set_internal("ZTILE1", 2)
        .set_internal("ZTILE2", 1);
    let mut data = Vec::new();
    data.extend_from_slice(&8i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&2i32.to_be_bytes());
    data.extend_from_slice(&8i32.to_be_bytes());
    data.extend_from_slice(&1.0f64.to_be_bytes());
    data.extend_from_slice(&0.0f64.to_be_bytes());
    data.extend_from_slice(&10i32.to_be_bytes());
    data.extend_from_slice(&20i32.to_be_bytes());
    data.extend_from_slice(&[1, 0]);
    let table = BinTable::from_data(&h, data).unwrap();
    let ImageData::F32(values) = decode::decompress_image(&h, &table).unwrap().samples else {
        panic!("expected F32")
    };
    assert!(values[0].is_nan());
    assert_eq!(values[1], 20.0);
}

#[test]
fn zblank_column_overrides_keyword_per_tile() {
    use crate::data::image_data::ImageData;
    use crate::header::Header;
    use crate::table_impl::BinTable;
    // A 2×1 float image, one NOCOMPRESS tile of quantized i32 [10, 99]. ZSCALE=2,
    // ZZERO=5 ⇒ pixel 0 = 25.0; pixel 1's quantized int equals the per-tile ZBLANK
    // *column* value (99), so it decodes to NaN — proving the column drives nulls.
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 28) // 1P(8) + 1D + 1D + 1J
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 8)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 4)
        .set_internal("TFORM1", "1PB(8)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("TFORM2", "1D")
        .set_internal("TTYPE2", "ZSCALE")
        .set_internal("TFORM3", "1D")
        .set_internal("TTYPE3", "ZZERO")
        .set_internal("TFORM4", "1J")
        .set_internal("TTYPE4", "ZBLANK")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "NOCOMPRESS")
        .set_internal("ZBITPIX", -32)
        .set_internal("ZNAXIS", 2)
        .set_internal("ZNAXIS1", 2)
        .set_internal("ZNAXIS2", 1)
        .set_internal("ZTILE1", 2)
        .set_internal("ZTILE2", 1);
    let mut data = Vec::new();
    data.extend_from_slice(&8i32.to_be_bytes()); // descriptor nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // descriptor offset
    data.extend_from_slice(&2.0f64.to_be_bytes()); // ZSCALE
    data.extend_from_slice(&5.0f64.to_be_bytes()); // ZZERO
    data.extend_from_slice(&99i32.to_be_bytes()); // ZBLANK column value
    data.extend_from_slice(&10i32.to_be_bytes()); // heap: quantized int 0
    data.extend_from_slice(&99i32.to_be_bytes()); // heap: quantized int 1 (== ZBLANK)
    let table = BinTable::from_data(&h, data).unwrap();
    let img = decode::decompress_image(&h, &table).unwrap();
    let ImageData::F32(px) = img.samples else {
        panic!("expected F32")
    };
    assert_eq!(px[0], 25.0);
    assert!(px[1].is_nan());

    for invalid in [0, 10_001] {
        let mut invalid_dither = h.clone();
        invalid_dither.set_internal("ZDITHER0", invalid);
        assert!(matches!(
            decode::decompress_image(&invalid_dither, &table),
            Err(FitsError::KeywordOutOfRange { name: "ZDITHER0" })
        ));
    }

    let mut mistyped_header = h.clone();
    mistyped_header.set_internal("ZBITPIX", "not an integer");
    assert!(matches!(
        decode::decompress_image(&mistyped_header, &table),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "ZBITPIX" && expected == "integer"
    ));

    let mut out_of_range_header = h.clone();
    out_of_range_header.set_internal("ZTILE1", 0);
    assert!(matches!(
        decode::decompress_image(&out_of_range_header, &table),
        Err(FitsError::KeywordOutOfRange { name: "ZTILEn" })
    ));
    let mut words = Vec::new();
    assert!(matches!(
        decode::decompress_image_section_into_words(
            &out_of_range_header,
            &table,
            &[0],
            &[0..2, 0..1],
            &mut words,
        ),
        Err(FitsError::KeywordOutOfRange { name: "ZTILEn" })
    ));

    for (column, name, kind) in [
        (1, "ZSCALE", TformKind::I64),
        (2, "ZZERO", TformKind::I64),
        (3, "ZBLANK", TformKind::F32),
    ] {
        let mut malformed = table.clone();
        crate::table_impl::internals::set_column_kind(&mut malformed, column, kind);
        assert!(matches!(
            decode::decompress_image(&h, &malformed),
            Err(FitsError::TypeMismatch { name: actual, .. }) if actual == name
        ));
    }
}

fn check_table_roundtrip(compression: Compression, rows_per_tile: usize) {
    use crate::table_impl::column_data::ColumnData;
    use crate::writer::FitsWriter;
    use crate::writer::table::{TableBuilder, WriteColumn};
    use std::io::Cursor;

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
    use crate::writer::FitsWriter;
    use std::fs::File;

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
    let restored = open("comp_table_cfitsio.fits")
        .read_compressed_table(1)
        .unwrap();
    let original = open("comp_table_ref.fits").read_table(1).unwrap();

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
    use crate::header::Header;
    use crate::header::value::Value;

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
    use crate::header::Header;
    use crate::writer::render_header;

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
    let mut f = open("comp_table_vla.fits");
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
    let mut source = open("comp_table_vla.fits");
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
    use crate::header::Header;
    use crate::table_impl::descriptor;
    use crate::writer::FitsWriter;
    use crate::writer::table::{TableBuilder, WriteColumn};

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
    let mut f = open("DDTSUVDATA.fits");
    assert!(matches!(
        f.read_compressed_table(1),
        Err(FitsError::NotCompressedTable)
    ));
}

#[test]
fn reading_a_plain_bintable_as_an_image_is_rejected() {
    // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
    let mut f = open("DDTSUVDATA.fits");
    // Public path: `read_image` sees a non-ZIMAGE bintable and rejects it as a
    // non-image (it never reaches the decompressor).
    assert!(matches!(f.read_image(1), Err(FitsError::NotAnImage)));
    // The decompressor itself still guards its `ZIMAGE` precondition.
    let table = f.read_table(1).unwrap();
    assert!(matches!(
        decode::decompress_image(&f.hdus[1].header, &table),
        Err(FitsError::NotCompressedImage)
    ));
}

#[test]
fn integer_image_compression_preserves_bscale_bzero_and_blank() {
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;
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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;
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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;
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
fn parallel_decode_wave_budget_counts_per_tile_vectors() {
    use crate::compress::decode::decode_wave_tile_count;
    use crate::compress::tile_geometry::TileGeometry;

    let geometry = TileGeometry::new(&[1, 4_194_304], &[1, 1]);
    let retained_bytes = std::mem::size_of::<Vec<u8>>() + 1;
    assert_eq!(
        decode_wave_tile_count::<u8>(&geometry),
        4 * 1024 * 1024 / retained_bytes
    );
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_full_decode_crosses_the_bounded_wave_boundary() {
    use crate::data::Image;
    use crate::writer::FitsWriter;

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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;
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
    use crate::data::Image;
    use crate::data::image_data::ImageData;
    use crate::data::scaling::Scaling;
    use crate::writer::FitsWriter;
    use std::io::Cursor;

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

#[test]
fn compressed_image_rejects_short_tiles() {
    use crate::error::FitsError;
    use crate::header::Header;
    use crate::table_impl::BinTable;

    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 1)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1PB(1)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "NOCOMPRESS")
        .set_internal("ZBITPIX", 16)
        .set_internal("ZNAXIS", 1)
        .set_internal("ZNAXIS1", 2)
        .set_internal("ZTILE1", 2);
    let mut data = Vec::new();
    data.extend_from_slice(&1i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.push(0);
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decode::decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            expected: 4,
            got: 1
        })
    ));

    h.set_internal("NAXIS1", 16)
        .set_internal("PCOUNT", 2)
        .set_internal("TFIELDS", 2)
        .set_internal("TFORM1", "1PB(0)")
        .set_internal("ZCMPTYPE", "GZIP_1")
        .set_internal("TFORM2", "1PI(1)")
        .set_internal("TTYPE2", "UNCOMPRESSED_DATA");
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&1i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&1i16.to_be_bytes());
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decode::decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));
}

#[test]
fn plio_validates_encoding_and_rejects_malformed_streams() {
    use crate::error::FitsError;

    let mut out = Vec::new();
    let be = |words: &[i16]| {
        words
            .iter()
            .flat_map(|word| word.to_be_bytes())
            .collect::<Vec<_>>()
    };

    let encoded = plio::plio_encode(&[0, 0xFF_FFFF]).unwrap();
    plio::plio_decode_be_into(&be(&encoded), 2, &mut out).unwrap();
    assert_eq!(out, [0, 0xFF_FFFF]);
    for invalid in [-1, 0x100_0000] {
        assert!(matches!(
            plio::plio_encode(&[0, invalid]),
            Err(FitsError::PlioValueOutOfRange {
                index: 1,
                value,
            }) if value == invalid
        ));
    }
    let truncated_span = [0, 7, -100, 8, 0, 0, 0];
    assert!(matches!(
        plio::plio_decode_be_into(&be(&truncated_span), 1, &mut out),
        Err(FitsError::UnexpectedEof)
    ));

    let missing_absolute_high_word = [0, 0, 4, 0x1001];
    assert!(matches!(
        plio::plio_decode_be_into(&be(&missing_absolute_high_word), 1, &mut out),
        Err(FitsError::UnexpectedEof)
    ));

    let invalid_opcode = [0, 0, 4, i16::MIN];
    assert!(matches!(
        plio::plio_decode_be_into(&be(&invalid_opcode), 1, &mut out),
        Err(FitsError::UnsupportedCompression { .. })
    ));
}

#[test]
fn compressed_image_descriptor_switches_to_q_for_large_offsets() {
    // §10.1.3: a heap offset beyond the 32-bit P range needs a 64-bit Q descriptor.
    let mut q = vec![0; 16];
    write_pq_descriptor(&mut q, true, 3, u32::MAX as u64 + 8).unwrap();
    assert_eq!(q.len(), 16);
    assert_eq!(i64::from_be_bytes(q[0..8].try_into().unwrap()), 3);
    assert_eq!(
        i64::from_be_bytes(q[8..16].try_into().unwrap()),
        u32::MAX as i64 + 8
    );
    let mut p = vec![0; 8];
    write_pq_descriptor(&mut p, false, 3, 40).unwrap();
    assert_eq!(p.len(), 8);
    assert_eq!(i32::from_be_bytes(p[4..8].try_into().unwrap()), 40);
}

#[test]
fn needs_wide_promotes_past_the_32_bit_descriptor_range() {
    // §10.1.3: a 32-bit `P` descriptor holds a signed 32-bit offset/count, so a heap
    // or tile element count past `i32::MAX` needs a 64-bit `Q` (the integer encoder
    // promotes; the fixed-layout float encoder errors). The threshold is shared.
    let max = i32::MAX as usize;
    assert!(!needs_wide(0, 0));
    assert!(!needs_wide(max, max)); // exactly i32::MAX still fits a 32-bit P
    assert!(needs_wide(max + 1, 0)); // heap offset past the range
    assert!(needs_wide(0, max + 1)); // a tile's element count past the range
}

#[test]
fn hcompress_tile_rejects_dimension_mismatch() {
    // Encode a valid 2×3 tile, then decode it claiming a different element count.
    // The decoder reads nx/ny from the stream and must cross-check them against the
    // tile size it was handed — rather than allocate/transform `nx*ny` blindly
    // (a wild-allocation / overflow / empty-buffer-panic guard, R2-4).
    let vals: Vec<i64> = vec![10, 20, 30, 40, 50, 60];
    let mut scratch = hcompress::HcompressScratch::default();
    let bytes = hcompress::hcompress_tile_encode(&vals, &[2, 3], 0, &mut scratch).unwrap();
    // The correct element count round-trips losslessly (scale 0).
    let mut out = Vec::new();
    hcompress::hcompress_tile_into(&bytes, false, 6, &mut out, &mut scratch).unwrap();
    assert_eq!(out, vals);
    // A mismatched element count is rejected, not decoded.
    assert!(hcompress::hcompress_tile_into(&bytes, false, 7, &mut out, &mut scratch).is_err());
    assert!(hcompress::hcompress_tile_into(&bytes, false, 5, &mut out, &mut scratch).is_err());

    let one = hcompress::hcompress_tile_encode(&[-7], &[1, 1], 0, &mut scratch).unwrap();
    hcompress::hcompress_tile_into(&one, false, 1, &mut out, &mut scratch).unwrap();
    assert_eq!(out, [-7], "a 1x1 tile must skip its empty quadrants");

    for end in 0..one.len() {
        assert!(
            matches!(
                hcompress::hcompress_tile_into(&one[..end], false, 1, &mut out, &mut scratch),
                Err(crate::error::FitsError::UnexpectedEof)
                    | Err(crate::error::FitsError::UnsupportedCompression { .. })
            ),
            "strict HCOMPRESS prefix of length {end} was accepted"
        );
    }

    let mut invalid_planes = one;
    invalid_planes[22] = 64;
    assert!(matches!(
        hcompress::hcompress_tile_into(&invalid_planes, false, 1, &mut out, &mut scratch),
        Err(crate::error::FitsError::UnsupportedCompression { .. })
    ));
}

#[test]
fn decompress_image_rejects_overflowing_znaxis_product() {
    use crate::error::FitsError;
    use crate::header::Header;
    use crate::table_impl::BinTable;
    // ZNAXIS1·ZNAXIS2 = 5e9·5e9 = 2.5e19 overflows usize; decode must reject the
    // header up front (before allocating the output plane), not wrap to a small
    // buffer and then scatter out of bounds (R2-2).
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1PB(0)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "GZIP_1")
        .set_internal("ZBITPIX", 16)
        .set_internal("ZNAXIS", 2)
        .set_internal("ZNAXIS1", 5_000_000_000i64)
        .set_internal("ZNAXIS2", 5_000_000_000i64);
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes()); // empty P descriptor: nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // offset
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decode::decompress_image(&h, &table),
        Err(FitsError::DataUnitOverflow)
    ));

    let image = crate::data::Image {
        shape: vec![usize::MAX, 2],
        samples: ImageData::I16(Vec::new()),
        scaling: crate::data::scaling::Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut out = Vec::new();
    assert!(matches!(
        encode::compress_image(
            &image,
            Compression::GZIP,
            &CompressionOptions::default(),
            &mut out
        ),
        Err(FitsError::DataUnitOverflow)
    ));
}

#[test]
fn uncompress_table_rejects_overflowing_row_product() {
    use crate::error::FitsError;
    use crate::header::Header;
    use crate::table_impl::BinTable;
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

#[test]
fn decompress_image_rejects_oversized_znaxis_product() {
    use crate::error::FitsError;
    use crate::header::Header;
    use crate::table_impl::BinTable;
    // ZNAXIS1 = 2^60 does NOT overflow usize (so the overflow guard passes), but
    // allocating that many bytes would abort the process. The output plane is
    // allocated fallibly (`try_reserve`), so decode must return a recoverable error.
    let mut h = Header::new();
    h.set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 8)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 1)
        .set_internal("TFORM1", "1PB(0)")
        .set_internal("TTYPE1", "COMPRESSED_DATA")
        .set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", "GZIP_1")
        .set_internal("ZBITPIX", 8)
        .set_internal("ZNAXIS", 1)
        .set_internal("ZNAXIS1", 1i64 << 60);
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes()); // empty P descriptor: nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // offset
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decode::decompress_image(&h, &table),
        Err(FitsError::DataUnitTooLarge { .. })
    ));
}

#[test]
fn gunzip_rejects_a_stream_larger_than_its_tile() {
    use crate::error::FitsError;
    // A decompression bomb: a tiny gzip stream inflating far past the tile's known
    // byte size must be rejected, not allocated unbounded.
    let big = vec![0u8; 100_000];
    let bomb = gzip::gzip_encode(&big, 9);
    assert!(bomb.len() < 1000, "an all-zero buffer compresses tiny");
    let mut decoded = Vec::new();
    // Bounded at 1 KiB (a small tile): inflating to 100 KB overruns → error.
    assert!(matches!(
        gzip::gunzip_into(&bomb, 1024, &mut decoded),
        Err(FitsError::UnsupportedCompression { .. })
    ));
    // One byte short of the true size still overruns (the +1 detection boundary).
    assert!(gzip::gunzip_into(&bomb, big.len() - 1, &mut decoded).is_err());
    // Bounded at the true size: decodes to exactly the original bytes.
    gzip::gunzip_into(&bomb, big.len(), &mut decoded).unwrap();
    assert_eq!(decoded, big);

    let short = gzip::gzip_encode(&[1, 2, 3], 1);
    assert!(matches!(
        gzip::gunzip_into(&short, 4, &mut decoded),
        Err(FitsError::DataSizeMismatch {
            expected: 4,
            got: 3
        })
    ));

    let raw = [1, 2, 3, 4, 5, 6];
    let mut shuffled = Vec::new();
    gzip::shuffle_bytes_into(&raw, 2, &mut shuffled);
    assert_eq!(shuffled, [1, 3, 5, 2, 4, 6]);
    gzip::unshuffle_bytes_into(&shuffled, 2, &mut decoded);
    assert_eq!(decoded, raw);
    gzip::shuffle_bytes_into(&raw, 1, &mut shuffled);
    assert_eq!(shuffled, raw, "width-one shuffle is an exact no-op");
}

#[test]
fn i64_be_round_trip_and_buffer_reuse() {
    // I16: narrowing (`as i16`) then big-endian packing, hand-computed. -1 → 0xFFFF,
    // 258 → 0x0102, -2 → 0xFFFE.
    let vals = [0i64, 1, -1, 258, -2];
    let want = [0, 0, 0, 1, 0xFF, 0xFF, 0x01, 0x02, 0xFF, 0xFE];
    assert_eq!(i64_to_be(&vals, Bitpix::I16), want);
    // Decode is the exact inverse (sign-extending back to i64), into a reused buffer.
    let mut d = Vec::new();
    be_to_i64_into(&want, Bitpix::I16, &mut d);
    assert_eq!(d, vals);

    // I32 packs four bytes/elem; 0x00010203 = 66051, -1 → all-ones.
    let i32_vals = [66051i64, -1];
    assert_eq!(
        i64_to_be(&i32_vals, Bitpix::I32),
        [0x00, 0x01, 0x02, 0x03, 0xFF, 0xFF, 0xFF, 0xFF]
    );
    let mut native_i32 = Vec::new();
    i32_to_be_into(&[66051, -1], &mut native_i32);
    assert_eq!(native_i32, [0x00, 0x01, 0x02, 0x03, 0xFF, 0xFF, 0xFF, 0xFF]);
    be_to_i64_into(&[0, 1, 2, 3, 0xFF, 0xFF, 0xFF, 0xFF], Bitpix::I32, &mut d);
    assert_eq!(d, i32_vals);

    // U8 and I64 ends of the range.
    assert_eq!(i64_to_be(&[255, 0], Bitpix::U8), [0xFF, 0x00]);
    be_to_i64_into(&[0xFF, 0x00], Bitpix::U8, &mut d);
    assert_eq!(d, [255, 0]);
    assert_eq!(i64_to_be(&[-1], Bitpix::I64), [0xFF; 8]);

    // `i64_to_be_into` clears + resizes its scratch: a long fill followed by a short
    // one must leave exactly the short result (no stale tail), matching the owning form.
    let mut buf = Vec::new();
    i64_to_be_into(&[7, 8, 9, 10], Bitpix::I16, &mut buf);
    i64_to_be_into(&vals, Bitpix::I16, &mut buf);
    assert_eq!(buf, want);
}

#[test]
fn tile_into_rows_match_layout() {
    // Expand the row representation (row_bases + row_len) back to the full flat-index
    // list it stands for, so the hand-computed expectations still describe pixels.
    fn flat(s: &TileScratch) -> Vec<usize> {
        let mut v = Vec::new();
        for &b in &s.row_bases {
            v.extend(b..b + s.row_len);
        }
        v
    }

    // 4×3 image (x fastest, strides [1, 4]) tiled 2×2 → 4 tiles; the top row of tiles
    // is edge-clipped to height 1. Same scratch reused across tiles (no stale rows).
    let geom = TileGeometry::new(&[4, 3], &[2, 2]);
    assert_eq!(geom.ntiles(), 4);
    assert_eq!(geom.max_tile_elements(), 4);
    let mut s = TileScratch::default();
    let expect = [
        (vec![0, 4], 2, vec![0, 1, 4, 5]), // origin (0,0), full 2×2: two rows of 2
        (vec![2, 6], 2, vec![2, 3, 6, 7]), // origin (2,0)
        (vec![8], 2, vec![8, 9]),          // origin (0,2), clipped to height 1
        (vec![10], 2, vec![10, 11]),       // origin (2,2), clipped to height 1
    ];
    for (t, (bases, row_len, pixels)) in expect.iter().enumerate() {
        geom.tile_into(t, &mut s);
        assert_eq!(&s.row_bases, bases, "tile {t} row bases");
        assert_eq!(s.row_len, *row_len, "tile {t} row len");
        assert_eq!(s.nelem(), pixels.len(), "tile {t} nelem");
        assert_eq!(&flat(&s), pixels, "tile {t} pixels");
    }

    // 4×2×2 image (strides [1, 4, 8]) tiled 2×2×2 → 2 tiles: exercises the odometer
    // carrying across the two higher axes; each tile is 4 rows of 2.
    let geom3 = TileGeometry::new(&[4, 2, 2], &[2, 2, 2]);
    assert_eq!(geom3.ntiles(), 2);
    geom3.tile_into(0, &mut s);
    assert_eq!(s.row_bases, vec![0, 4, 8, 12]);
    assert_eq!(flat(&s), vec![0, 1, 4, 5, 8, 9, 12, 13]);
    geom3.tile_into(1, &mut s);
    assert_eq!(flat(&s), vec![2, 3, 6, 7, 10, 11, 14, 15]);

    let empty = TileGeometry::new(&[usize::MAX, usize::MAX, 0], &[1, 1, 1]);
    assert_eq!(empty.ntiles(), 0);
}
