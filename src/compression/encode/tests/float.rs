//! Float images written compressed: quantized and lossless, their scaling, dither, nulls
//! and container layout.

use crate::compression::encode::tests::*;

#[test]
fn float_compression_preserves_scaling_across_quantized_and_fallback_tiles() {
    let mut f32_samples: Vec<f32> = float_field().into_iter().take(24).collect();
    f32_samples.extend(iter::repeat_n(42.25, 24));
    let f64_samples = f32_samples.iter().map(|&value| f64::from(value)).collect();

    for samples in [Data::F32(f32_samples), Data::F64(f64_samples)] {
        let bitpix = samples.bitpix();
        let expected_raw: Vec<f64> = match &samples {
            Data::F32(values) => values.iter().map(|&value| f64::from(value)).collect(),
            Data::F64(values) => values.clone(),
            _ => unreachable!("float cases only"),
        };
        let image = Image {
            shape: vec![24, 2],
            samples,
            scaling: samples::linear(2.5, -10.0),
        };
        for compression in [
            Compression::Rice,
            Compression::GZIP,
            Compression::GZIP_SHUFFLED,
            Compression::None,
        ] {
            let cmptype = compression.name();
            let file = memory::written(|w| {
                w.write_compressed_image(
                    image.view(),
                    &CompressionConfig::new(compression).with_tiles([24, 1]),
                    None,
                )
            });
            let mut r = file.stream();
            let header = &r.hdus()[1].header;
            match compression {
                Compression::Rice => {
                    cards::assert_cards(
                        header,
                        &[("ZNAME1", "BLOCKSIZE".into()), ("ZNAME2", "BYTEPIX".into())],
                    );
                }
                _ => {
                    assert_eq!(header.get_text("ZNAME1").unwrap(), None);
                }
            }
            assert_eq!(
                header.get_real(keywords::BSCALE).unwrap(),
                Some(2.5),
                "{bitpix:?} {cmptype}"
            );
            assert_eq!(
                header.get_real(keywords::BZERO).unwrap(),
                Some(-10.0),
                "{bitpix:?} {cmptype}"
            );

            let table = r.read_container_table(1).unwrap();
            let compressed = table.column("COMPRESSED_DATA").unwrap().vla().unwrap();
            let fallback = table.column("GZIP_COMPRESSED_DATA").unwrap().vla().unwrap();
            assert_ne!(
                compressed.range(0).len(),
                0,
                "quantized tile for {bitpix:?} {cmptype}"
            );
            assert_eq!(
                fallback.range(0).len(),
                0,
                "quantized tile for {bitpix:?} {cmptype}"
            );
            assert_eq!(
                compressed.range(1).len(),
                0,
                "fallback tile for {bitpix:?} {cmptype}"
            );
            assert_ne!(
                fallback.range(1).len(),
                0,
                "fallback tile for {bitpix:?} {cmptype}"
            );

            // Tile 0 is quantized at its ZSCALE, so each sample comes back within
            // half a step of it, plus the roundings of the dither arithmetic and,
            // for f32, of storing the result — at most an ulp of the sample.
            let zscale = table.column("ZSCALE").unwrap().physical().unwrap()[0];
            let rounding = match bitpix {
                Bitpix::F32 => f64::from(f32::EPSILON),
                _ => 4.0 * f64::EPSILON,
            };
            let back = memory::read_whole(&mut r, 1);
            assert_eq!(back.scaling, image.scaling, "{bitpix:?} {cmptype}");
            let physical = physical::<f64>(&back);
            let actual_raw: Vec<f64> = match back.samples {
                Data::F32(values) => values.into_iter().map(f64::from).collect(),
                Data::F64(values) => values,
                other => panic!("{cmptype}: expected {bitpix:?}, got {other:?}"),
            };
            let mut quantized_changed = false;
            for (index, (&expected, &actual)) in expected_raw.iter().zip(&actual_raw).enumerate() {
                let expected_physical = -10.0 + 2.5 * expected;
                if index < 24 {
                    assert_within_half_step(
                        expected,
                        actual,
                        zscale,
                        bitpix,
                        &format!("{bitpix:?} {cmptype} raw pixel {index}"),
                    );
                    let raw_bound = 0.5 * zscale + rounding * expected.abs();
                    // BSCALE scales the raw error; the scaling itself rounds once more.
                    let physical_error = (physical[index] - expected_physical).abs();
                    assert!(
                        physical_error
                            <= 2.5 * raw_bound + 2.0 * f64::EPSILON * expected_physical.abs(),
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
fn dither_option_sets_zquantiz_and_round_trips() {
    let image = Image::new(vec![24, 16], Data::F32(float_field()), Scaling::IDENTITY).unwrap();
    // Each `Dither` writes its `ZQUANTIZ` keyword and decodes within half a
    // quantization step. The three decoded planes differ pairwise, so the option is
    // honored. A dither writes the seed of the samples' bytes as `ZDITHER0`; no dither
    // writes none.
    let original = float_field();
    let seed = quantize::sample_seed(FloatSamples::F32(&original));
    let mut planes = Vec::new();
    for (dither, zquantiz, zdither0) in [
        (Dither::None, "NO_DITHER", None),
        (
            Dither::Subtractive1(DitherSeed::FROM_SAMPLES),
            "SUBTRACTIVE_DITHER_1",
            Some(seed),
        ),
        (
            Dither::Subtractive2(DitherSeed::FROM_SAMPLES),
            "SUBTRACTIVE_DITHER_2",
            Some(seed),
        ),
    ] {
        let config = CompressionConfig::new(Compression::Rice)
            .with_tiles([24, 16])
            .with_quantization(Quantization::relative(4.0, dither).unwrap());
        let file = memory::written(|w| w.write_compressed_image(image.view(), &config, None));
        let mut r = file.stream();
        let header = &r.hdus()[1].header;
        assert_eq!(
            header.get_text(keywords::ZQUANTIZ).unwrap(),
            Some(zquantiz),
            "{dither:?} must write {zquantiz}"
        );
        assert_eq!(
            header.get_integer(keywords::ZDITHER0).unwrap(),
            zdither0,
            "{dither:?}"
        );
        planes.push(assert_within_half_a_step(
            &mut r,
            &original,
            original.len(),
            &format!("{dither:?}"),
        ));
    }
    assert_ne!(planes[0], planes[1]);
    assert_ne!(planes[0], planes[2]);
    assert_ne!(planes[1], planes[2]);
}

/// `Quantization::NONE` stores the floats themselves: a NaN's payload, a signaling NaN,
/// ±0.0 and subnormals come back bit for bit, with `ZQUANTIZ = 'NONE'` and no scale
/// columns or seed.
#[test]
fn lossless_float_compression_round_trips_bit_for_bit() {
    let f32s: Vec<f32> = (0..24 * 16)
        .map(|i| match i % 7 {
            0 => f32::from_bits(0x7FC0_0001),
            1 => -0.0,
            2 => f32::from_bits(1),
            3 => f32::MAX,
            4 => f32::from_bits(0x7F80_0001),
            _ => i as f32 * 0.37 - 50.0,
        })
        .collect();
    let f64s: Vec<f64> = f32s
        .iter()
        .map(|&v| {
            if v.to_bits() == 0x7F80_0001 {
                f64::from_bits(0x7FF0_0000_0000_0001)
            } else if v.is_nan() {
                f64::from_bits(0x7FF8_0000_0000_0001)
            } else {
                f64::from(v) * 1.000_000_1
            }
        })
        .collect();
    for compression in [Compression::GZIP, Compression::GZIP_SHUFFLED] {
        let config = CompressionConfig::new(compression).with_quantization(Quantization::NONE);
        for image in [
            Image::new(vec![24, 16], f32s.clone(), Scaling::IDENTITY).unwrap(),
            Image::new(vec![24, 16], f64s.clone(), Scaling::IDENTITY).unwrap(),
        ] {
            let file = memory::written(|w| w.write_compressed_image(image.view(), &config, None));
            let mut r = file.stream();
            let header = &r.hdus()[1].header;
            assert_eq!(header.get_text(keywords::ZQUANTIZ).unwrap(), Some("NONE"));
            assert_eq!(header.get_integer(keywords::ZDITHER0).unwrap(), None);
            assert_eq!(header.get_integer(keywords::TFIELDS).unwrap(), Some(1));
            samples::assert_same_bits(
                &memory::read_whole(&mut r, 1).samples,
                &image.samples,
                &format!("{compression:?}"),
            );
        }
    }
    let image = Image::new(vec![24, 16], f32s, Scaling::IDENTITY).unwrap();
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice).with_quantization(Quantization::NONE),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::LosslessFloatCodec { codec: "RICE_1" }
        }
    );
}

/// The float container widens both descriptors to `Q` past the 32-bit range, as the
/// integer one does; the layout is taken at `Q` here, as no test writes a 2 GiB heap.
#[test]
fn float_container_lays_out_q_descriptors() {
    let image = Image::new(vec![24, 16], float_field(), Scaling::IDENTITY).unwrap();
    let Data::F32(samples) = &image.samples else {
        unreachable!("an f32 image")
    };
    let config = CompressionConfig::new(Compression::Rice);
    let encode = || {
        let float = FloatImage {
            image: image.view(),
            samples: FloatSamples::F32(samples),
        };
        let levels = Levels {
            step: QuantizationStep::Relative(4.0),
            dither: Dither::default(),
        };
        QuantizedFloatImage::encode(float, &config, levels).unwrap()
    };
    assert_eq!(encode().addressing().unwrap(), DescriptorWidth::P);
    let read = |width: DescriptorWidth| {
        let unit = encode().lay_out(width).unwrap();
        let header = &unit.header;
        assert_eq!(
            header
                .get_text("TFORM1")
                .unwrap()
                .map(|tform| tform.as_bytes()[1]),
            Some(width.letter() as u8)
        );
        let table = Table::from_data(header, unit.data()).unwrap();
        decompress_image(header, &table).unwrap().samples
    };
    assert_eq!(read(DescriptorWidth::Q), read(DescriptorWidth::P));
}

#[test]
fn float_write_preserves_nan_nulls() {
    let mut orig = float_field();
    orig[5 + 3 * 24] = f32::NAN;
    orig[20 + 10 * 24] = f32::NAN;
    let image = Image::new(vec![24, 16], Data::F32(orig.clone()), Scaling::IDENTITY).unwrap();
    let file = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice).with_tiles([24, 16]),
            None,
        )
    });
    let mut r = file.stream();
    assert_within_half_a_step(&mut r, &orig, orig.len(), "NaN nulls");
}
