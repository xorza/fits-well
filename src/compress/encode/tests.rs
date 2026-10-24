use crate::bintable::Table;
use crate::bintable::descriptor::DescriptorWidth;
use crate::compress::decode::tiled_image::TiledImage;
use crate::compress::encode::{FloatImage, QuantizedFloatImage, hcompress_default_rows};
use crate::compress::internals::{mask, odd_plane, ramp};
use crate::compress::plane::FloatSamples;
use crate::compress::quantization::{Dither, DitherSeed, Levels, QuantizationStep};
use crate::compress::*;
use crate::error::CompressionFeature;
use crate::fits_writer::FitsWriter;
use crate::hdu::image_geometry::ImageGeometry;
use crate::hdu::{HduKind, HduRole};
use crate::header_model::value::Value;
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::internals::physical;
use crate::internals::cards::assert_cards;
use crate::internals::fixture::Fixture;
use crate::internals::memory::{read_whole, rejected_before_output, round_trip};
use crate::internals::rng::SplitMix;
use crate::internals::samples::assert_same_bits;
use crate::reader::FitsReader;
use crate::reader::source::Source;
use std::io::Cursor;
use std::iter;

#[test]
fn compression_write_round_trips_through_decode() {
    let ramp_image = Image::new(vec![24, 16], ramp(), Scaling::IDENTITY).unwrap();
    // PLIO is a mask codec: non-negative i32 values.
    let mask_image = Image::new(vec![24, 16], mask(), Scaling::IDENTITY).unwrap();
    // `BITPIX = 8` is unsigned; 37 is coprime with 256, so every byte value occurs.
    let byte_image = Image::new(
        vec![24, 16],
        (0..24 * 16usize)
            .map(|i| u8::try_from(i * 37 % 256).unwrap())
            .collect::<Vec<u8>>(),
        Scaling::IDENTITY,
    )
    .unwrap();
    for (case, image, compression, tiles) in [
        ("GZIP_1 row tiles", &ramp_image, Compression::GZIP, &[][..]),
        ("GZIP_1 2-D tiles", &ramp_image, Compression::GZIP, &[7, 5]),
        (
            "GZIP_2 row tiles",
            &ramp_image,
            Compression::GZIP_SHUFFLED,
            &[],
        ),
        ("RICE_1 row tiles", &ramp_image, Compression::Rice, &[]),
        ("RICE_1 bytes", &byte_image, Compression::Rice, &[]),
        ("GZIP_1 bytes", &byte_image, Compression::GZIP, &[]),
        (
            "HCOMPRESS_1 whole image",
            &ramp_image,
            Compression::Hcompress(Hcompress::default()),
            &[24, 16],
        ),
        ("PLIO_1 row tiles", &mask_image, Compression::Plio, &[]),
        // §10.4: tiles stored verbatim, as big-endian pixels.
        ("NOCOMPRESS row tiles", &ramp_image, Compression::None, &[]),
    ] {
        let mut r = round_trip(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(compression).with_tiles(tiles),
                None,
            )
        });
        if matches!(compression, Compression::Hcompress(_)) {
            let header = &r.hdus()[1].header;
            assert!(matches!(
                header.get("ZVAL1").unwrap(),
                Some(Value::Real(0.0))
            ));
            assert_eq!(header.get_text("ZNAME2").unwrap(), None);
        }
        let back = read_whole(&mut r, 1);
        assert_eq!(back.shape, vec![24, 16], "{case}");
        assert_eq!(back.samples, image.samples, "{case}");
    }

    let samples_3d: Vec<i16> = (0..5 * 4 * 3).map(|i| i as i16 * 3 - 50).collect();
    let image_3d = Image {
        shape: vec![5, 4, 3],
        samples: Data::I16(samples_3d.clone()),
        scaling: Scaling::IDENTITY,
    };
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image_3d.view(),
            &CompressionConfig::new(Compression::GZIP).with_tiles([3, 2, 2]),
            None,
        )
    });
    let back = read_whole(&mut r, 1);
    assert_eq!(back.shape, vec![5, 4, 3]);
    assert!(matches!(back.samples, Data::I16(v) if v == samples_3d));
}

/// A 24×16 float field: a smooth ramp plus genuine high-frequency noise (a
/// splitmix64 hash, decorrelated neighbour-to-neighbour) so the 3rd-order MAD
/// estimate is realistic (≈ 1) and the tile genuinely quantizes.
fn float_field() -> Vec<f32> {
    let mix = |i: u64| SplitMix::new(i).next();
    (0..24 * 16)
        .map(|i| {
            let (x, y) = (i % 24, i / 24);
            let smooth = 100.0 + 3.0 * x as f32 - 2.0 * y as f32;
            let noise = (mix(u64::try_from(i).unwrap()) % 2000) as f32 / 1000.0 - 1.0; // ±1.0
            smooth + noise
        })
        .collect()
}

/// The f32 plane of HDU 1 of `reader`, asserted to give back each sample of `original`
/// within half the quantization step of its tile of `tile_len` samples, plus an f32 ulp
/// of the sample for the dither arithmetic and the stored result. A NaN gives a NaN.
#[track_caller]
fn assert_within_half_a_step<S: Source>(
    reader: &mut FitsReader<S>,
    original: &[f32],
    tile_len: usize,
    context: &str,
) -> Vec<f32> {
    let zscale = reader
        .read_container_table(1)
        .unwrap()
        .column("ZSCALE")
        .unwrap()
        .physical()
        .unwrap();
    let Data::F32(plane) = read_whole(reader, 1).samples else {
        panic!("{context}: expected F32");
    };
    assert_eq!(plane.len(), original.len(), "{context}");
    for (index, (&o, &b)) in original.iter().zip(&plane).enumerate() {
        if o.is_nan() {
            assert!(b.is_nan(), "{context} pixel {index}: {b} for a NaN");
            continue;
        }
        let error = (f64::from(o) - f64::from(b)).abs();
        let bound = 0.5 * zscale[index / tile_len] + f64::from(f32::EPSILON) * f64::from(o).abs();
        assert!(error <= bound, "{context} pixel {index}: {o} vs {b}");
    }
    plane
}

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
            let mut r = round_trip(|w| {
                w.write_compressed_image(
                    image.view(),
                    &CompressionConfig::new(compression).with_tiles([24, 1]),
                    None,
                )
            });
            let header = &r.hdus()[1].header;
            match compression {
                Compression::Rice => {
                    assert_cards(
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
            let back = read_whole(&mut r, 1);
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
                    let raw_error = (actual - expected).abs();
                    let raw_bound = 0.5 * zscale + rounding * expected.abs();
                    assert!(
                        raw_error <= raw_bound,
                        "{bitpix:?} {cmptype} raw pixel {index}: {raw_error}"
                    );
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

/// An integer image stores its values as they are and refuses a quantization, as a
/// table does; a tile of 0 along any axis is refused, not read as 1.
#[test]
fn an_integer_image_refuses_quantization_and_an_empty_tile() {
    let image = Image::new(vec![4, 2], vec![1i16; 8], Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    let rice = CompressionConfig::new(Compression::Rice);
    assert!(matches!(
        writer.write_compressed_image(
            image.view(),
            &rice
                .clone()
                .with_quantization(Quantization::relative(4.0, Dither::None).unwrap()),
            None
        ),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::IntegerQuantization
        })
    ));
    assert!(matches!(
        writer.write_compressed_image(image.view(), &rice.with_tiles([4, 0]), None),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::EmptyTile { axis: 1 }
        })
    ));
}

#[test]
fn hcompress_writer_enforces_standard_image_constraints() {
    let line = Image::new(vec![4], vec![1i16, 2, 3, 4], Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_image(
            line.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
            None
        ),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        })
    ));

    let float = Image::new(vec![8, 8], float_field()[..64].to_vec(), Scaling::IDENTITY).unwrap();
    let dither2 =
        Quantization::relative(4.0, Dither::Subtractive2(DitherSeed::FROM_SAMPLES)).unwrap();
    assert!(matches!(
        writer.write_compressed_image(
            float.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_quantization(dither2),
            None
        ),
        Err(FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::HcompressDither2
        })
    ));
    assert!(matches!(
        writer.write_compressed_image(
            float.view(),
            &CompressionConfig::new(Compression::Plio),
            None
        ),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::QuantizedFloatCodec { codec: "PLIO_1" }
        })
    ));

    let extreme = Image::new(vec![4, 4], vec![i64::MAX; 16], Scaling::IDENTITY).unwrap();
    assert!(matches!(
        writer.write_compressed_image(
            extreme.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_tiles([4, 4]),
            None
        ),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressRange
        })
    ));

    let undefined = Image::new(
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
            undefined.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::relative(2.0).unwrap()))
                .with_tiles([8, 8]),
            None
        ),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::LossyHcompressNulls
        })
    ));
}

#[test]
fn dither_option_sets_zquantiz_and_round_trips() {
    let image = Image {
        shape: vec![24, 16],
        samples: Data::F32(float_field()),
        scaling: Scaling::IDENTITY,
    };
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
        let mut r = round_trip(|w| w.write_compressed_image(image.view(), &config, None));
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

/// `ZDITHER0` is the byte sum of all the samples: two images whose first rows are
/// equal get different seeds, the same image gives the same bytes, and a fixed seed
/// is written as given and drives the decoder's dither.
#[test]
fn the_dither_seed_comes_from_every_sample() {
    let write = |samples: Vec<f32>, dither: Dither| {
        let image = Image::new(vec![24, 16], samples, Scaling::IDENTITY).unwrap();
        let config = CompressionConfig::new(Compression::Rice)
            .with_quantization(Quantization::relative(4.0, dither).unwrap());
        let mut sink = Cursor::new(Vec::new());
        let mut writer = FitsWriter::new(&mut sink);
        writer
            .write_compressed_image(image.view(), &config, None)
            .unwrap();
        sink.into_inner()
    };
    let zdither0 = |bytes: &[u8]| {
        let reader = FitsReader::from_bytes(bytes).unwrap();
        reader.hdus()[1]
            .header
            .get_integer(keywords::ZDITHER0)
            .unwrap()
            .unwrap()
    };
    let first = float_field();
    let mut second = first.clone();
    for value in &mut second[24..] {
        *value += 1.0;
    }
    let dither = Dither::default();
    let (a, b) = (write(first.clone(), dither), write(second, dither));
    assert_ne!(zdither0(&a), zdither0(&b));
    assert_eq!(
        write(first.clone(), dither),
        a,
        "the same image gives the same file"
    );

    let fixed = write(
        first.clone(),
        Dither::Subtractive1(DitherSeed::fixed(17).unwrap()),
    );
    assert_eq!(zdither0(&fixed), 17);
    // The default tiles are rows of 24 samples.
    let mut reader = FitsReader::from_bytes(&fixed).unwrap();
    assert_within_half_a_step(&mut reader, &first, 24, "fixed seed");
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
            let mut r = round_trip(|w| w.write_compressed_image(image.view(), &config, None));
            let header = &r.hdus()[1].header;
            assert_eq!(header.get_text(keywords::ZQUANTIZ).unwrap(), Some("NONE"));
            assert_eq!(header.get_integer(keywords::ZDITHER0).unwrap(), None);
            assert_eq!(header.get_integer(keywords::TFIELDS).unwrap(), Some(1));
            assert_same_bits(
                &read_whole(&mut r, 1).samples,
                &image.samples,
                &format!("{compression:?}"),
            );
        }
    }
    let image = Image::new(vec![24, 16], f32s, Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice).with_quantization(Quantization::NONE),
            None
        ),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::LosslessFloatCodec { codec: "RICE_1" }
        })
    ));
}

/// fits-well quantizes as fpack does: `fpack -q17 4` and `fpack -q17 -0.01` (CFITSIO
/// 4.070, seed 17, `RICE_1`, row tiles) of the 64×16 f32 plane
/// `1000 + ((2654435761·i) mod 2³²) / 2³² · 100` give the same `ZSCALE` and `ZZERO`
/// for each tile and decode to the same floats as fits-well's write of that plane with
/// the same quantization. cfitsio takes the level as a C `float`, so the absolute step
/// is `0.01f32`. The plane has no run of three equal values, where the noise2 medians
/// would differ (see `quantize::noise5`).
#[test]
fn float_quantization_matches_fpack() {
    fn columns<S: Source>(r: &mut FitsReader<S>) -> [Vec<f64>; 2] {
        let table = r.read_container_table(1).unwrap();
        ["ZSCALE", "ZZERO"].map(|name| table.column(name).unwrap().physical().unwrap())
    }
    let plane: Vec<f32> = (0u64..64 * 16)
        .map(|i| {
            (1000.0 + ((i * 2_654_435_761) % (1 << 32)) as f64 / 4_294_967_296.0 * 100.0) as f32
        })
        .collect();
    let image = Image::new(vec![64, 16], plane, Scaling::IDENTITY).unwrap();
    let dither = Dither::Subtractive1(DitherSeed::fixed(17).unwrap());
    let hcompress = Compression::Hcompress(Hcompress::relative(2.5).unwrap());
    for (fixture, compression, quantization) in [
        (
            Fixture::CompQuantRelSeed17F32,
            Compression::Rice,
            Quantization::relative(4.0, dither),
        ),
        (
            Fixture::CompQuantAbsSeed17F32,
            Compression::Rice,
            Quantization::absolute(f64::from(0.01f32), dither),
        ),
        // `fpack -q17 4 -h -s 2.5`: HCOMPRESS of the quantized integers.
        (
            Fixture::CompHcompQuantSeed17F32,
            hcompress,
            Quantization::relative(4.0, dither),
        ),
    ] {
        let config = CompressionConfig::new(compression).with_quantization(quantization.unwrap());
        let mut ours = round_trip(|w| w.write_compressed_image(image.view(), &config, None));
        let mut fpack = fixture.stream();
        assert_eq!(columns(&mut ours), columns(&mut fpack), "{fixture:?}");
        assert_eq!(
            read_whole(&mut ours, 1).samples,
            read_whole(&mut fpack, 1).samples,
            "{fixture:?}"
        );
    }
}

/// fits-well's HCOMPRESS scale is fpack's: `fpack -h -s 2.5` and `fpack -h -s -30`
/// (CFITSIO 4.070) of the 64×32 i16 plane `500 + ((40503·i) mod 65536) mod 200` decode to
/// the same values as fits-well's write with `Hcompress::relative(2.5)` and
/// `Hcompress::absolute(30.0)`, and record the same `ZVAL1`.
#[test]
fn hcompress_scales_match_fpack() {
    let plane: Vec<i16> = (0..64 * 32)
        .map(|i| 500 + i16::try_from((i * 40503) % 65536 % 200).unwrap())
        .collect();
    let image = Image::new(vec![64, 32], plane, Scaling::IDENTITY).unwrap();
    for (fixture, hcompress, zval1) in [
        (Fixture::CompHcompRelI16, Hcompress::relative(2.5), 2.5),
        (Fixture::CompHcompAbsI16, Hcompress::absolute(30.0), -30.0),
    ] {
        let config = CompressionConfig::new(Compression::Hcompress(hcompress.unwrap()));
        let mut ours = round_trip(|w| w.write_compressed_image(image.view(), &config, None));
        assert_eq!(
            ours.hdus()[1].header.get_real("ZVAL1").unwrap(),
            Some(zval1),
            "{fixture:?}"
        );
        let mut fpack = fixture.stream();
        assert_eq!(
            read_whole(&mut ours, 1).samples,
            read_whole(&mut fpack, 1).samples,
            "{fixture:?}"
        );
    }
}

/// The float container widens both descriptors to `Q` past the 32-bit range, as the
/// integer one does; the layout is taken at `Q` here, as no test writes a 2 GiB heap.
#[test]
fn the_float_container_lays_out_q_descriptors() {
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
        let kind = HduKind::classify(header, HduRole::Extension).unwrap();
        let geometry = ImageGeometry::from_header(header, kind).unwrap();
        TiledImage::new(header, &geometry)
            .unwrap()
            .decode(table.view())
            .unwrap()
    };
    assert_eq!(read(DescriptorWidth::Q), read(DescriptorWidth::P));
}

/// The writer sums and writes a compressed container's rows and then its cells in
/// order, with no copy into one buffer, so the file it writes verifies, an integer and a
/// quantized float container alike.
#[test]
fn a_container_written_in_parts_verifies() {
    use crate::internals::cards::VALID;
    use crate::internals::memory::{memory_writer, round_trip_by};
    let integer = Image::new(vec![24, 16], ramp(), Scaling::IDENTITY).unwrap();
    let float = Image::new(vec![24, 16], float_field(), Scaling::IDENTITY).unwrap();
    // One NOCOMPRESS tile past the writer's 1 MiB gathering chunk goes out on its own,
    // between the small cells the chunk gathers.
    let large = Image::new(
        vec![1024, 1100],
        (0..1024 * 1100usize)
            .map(|index| u8::try_from(index % 251).unwrap())
            .collect::<Vec<_>>(),
        Scaling::IDENTITY,
    )
    .unwrap();
    let mut reader = round_trip_by(memory_writer().with_checksums(), |w| {
        let config = CompressionConfig::new(Compression::Rice).with_tiles([24, 3]);
        w.write_compressed_image(integer.view(), &config, None)?;
        w.write_compressed_image(float.view(), &config, None)?;
        w.write_compressed_image(
            large.view(),
            &CompressionConfig::new(Compression::None).with_tiles([1024, 1100]),
            None,
        )
    });
    for hdu in [1, 2, 3] {
        assert_eq!(reader.verify_checksum(hdu).unwrap(), VALID, "HDU {hdu}");
    }
    assert_eq!(read_whole(&mut reader, 1).samples, integer.samples);
    assert_eq!(read_whole(&mut reader, 3).samples, large.samples);
}

/// A u16 image's `PLIO_1` stream holds its physical values, which PLIO takes as it
/// takes no negative value; the image reads back stored as `physical − 32768`.
#[test]
fn plio_writes_a_u16_image() {
    let physical = [0u16, 1, 32767, 32768, 65535, 7, 40000, 12345];
    let image = Image::from_u16(vec![8, 1], &physical).unwrap();
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Plio),
            None,
        )
    });
    let stored: Vec<i16> = physical
        .iter()
        .map(|&v| (i32::from(v) - 32768) as i16)
        .collect();
    assert_eq!(read_whole(&mut r, 1).samples, Data::I16(stored));
}

#[test]
fn tile_shape_with_wrong_rank_is_rejected() {
    // A 2-D image with a 3-axis tile shape: a wrong-rank request errors rather than
    // silently row-tiling (an empty shape is still the row-tiling default).
    let image = Image {
        shape: vec![4, 3],
        samples: Data::I16((0..12).map(|i| i as i16).collect()),
        scaling: Scaling::IDENTITY,
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let err = w.write_compressed_image(
        image.view(),
        &CompressionConfig::new(Compression::Rice).with_tiles([2, 2, 2]),
        None,
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
        samples: Data::F32(orig.clone()),
        scaling: Scaling::IDENTITY,
    };
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice).with_tiles([24, 16]),
            None,
        )
    });
    assert_within_half_a_step(&mut r, &orig, orig.len(), "NaN nulls");
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
    let image = Image::new(vec![8, 8], samples, Scaling::IDENTITY).unwrap();
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::relative(2.0).unwrap()))
                .with_tiles([8, 8]),
            None,
        )
    });
    assert!(matches!(
        r.hdus()[1].header.get("ZVAL1").unwrap(),
        Some(Value::Real(2.0))
    ));
    let table = r.read_container_table(1).unwrap();
    let stream = table
        .column("COMPRESSED_DATA")
        .unwrap()
        .vla_column()
        .unwrap()
        .cell(0)
        .unwrap()
        .bytes;
    assert_eq!(&stream[..2], &[0xDD, 0x99]);
    assert_eq!(i32::from_be_bytes(stream[10..14].try_into().unwrap()), 10);
}

/// cfitsio's default `HCOMPRESS_1` tile height, pinned against `fpack -h -s 0`
/// (CFITSIO 4.070): short images are one tile, and each row reaches the next
/// candidate because every earlier one leaves a 1–3 row remainder (33 = 2·16 + 1,
/// 49 = 2·24 + 1, 241 = 8·30 + 1, 1681 = 60·28 + 1, …).
#[test]
fn hcompress_default_tiles_follow_cfitsio() {
    for (rows, tile) in [
        (4, 4),
        (30, 30),
        (31, 16),
        (36, 16),
        (33, 24),
        (49, 20),
        (241, 28),
        (1681, 26),
        (21841, 22),
        (240_241, 18),
        (720_721, 17),
    ] {
        assert_eq!(hcompress_default_rows(rows), tile, "{rows} rows");
    }

    let image = Image::new(vec![64, 33], vec![0i16; 64 * 33], Scaling::IDENTITY).unwrap();
    let r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
            None,
        )
    });
    let header = &r.hdus()[1].header;
    assert_cards(header, &[("ZTILE1", 64.into()), ("ZTILE2", 24.into())]);
}

/// A tiling with a side under 4 pixels is refused, never adjusted: an explicit thin
/// tile, a remainder the explicit tile leaves, and an image too thin for any tile.
#[test]
fn hcompress_refuses_tiles_under_four_pixels() {
    let refused = |shape: Vec<usize>, tiles: &[usize]| {
        let image = Image::new(
            shape.clone(),
            vec![0i16; shape.iter().product()],
            Scaling::IDENTITY,
        )
        .unwrap();
        rejected_before_output(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                    .with_tiles(tiles.to_vec()),
                None,
            )
        })
    };
    for (shape, tiles, axis, length) in [
        (vec![17, 1], &[17, 1][..], 1, 1),
        (vec![64, 33], &[64, 16][..], 1, 1),
        (vec![64, 3], &[][..], 1, 3),
        (vec![3, 64], &[][..], 0, 3),
    ] {
        let error = refused(shape.clone(), tiles);
        assert!(
            matches!(error, FitsError::HcompressTileTooSmall { axis: a, length: l } if a == axis && l == length),
            "{shape:?} in {tiles:?} tiles: {error:?}",
        );
    }
}

/// Lossless tiles whose halves turn odd on both axes at once, the corner the inverse
/// transform once misplaced, round-trip exactly beside ones that never do (64×64,
/// 24×16); 6000×40 takes the default tiling, two 16-row tiles and an 8-row one.
#[test]
fn hcompress_round_trips_tiles_odd_on_both_axes() {
    for shape in [
        [17, 17],
        [100, 100],
        [17, 5],
        [5, 17],
        [64, 64],
        [24, 16],
        [6000, 40],
    ] {
        let samples = odd_plane(shape[0] * shape[1]);
        let image = Image::new(shape.to_vec(), samples.clone(), Scaling::IDENTITY).unwrap();
        let mut r = round_trip(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
                None,
            )
        });
        assert_eq!(
            read_whole(&mut r, 1).samples,
            Data::I16(samples),
            "{shape:?}"
        );
        if shape == [6000, 40] {
            assert_cards(
                &r.hdus()[1].header,
                &[
                    ("ZTILE1", 6000.into()),
                    ("ZTILE2", 16.into()),
                    ("NAXIS2", 3.into()),
                ],
            );
        }
    }
}

/// Lossless HCOMPRESS gives back an i32 ramp, and i32 and i64 values whose sums pass
/// 2³¹ in its 64-bit transform.
#[test]
fn hcompress_lossless_write_round_trips_exactly() {
    let ramp: Vec<i32> = (0..16 * 16)
        .map(|index| {
            let x = index % 16;
            let y = index / 16;
            x * x - 3 * y
        })
        .collect();
    let wide = [
        1i64 << 40,
        -(1i64 << 40),
        (1i64 << 40) + 1,
        -(1i64 << 40) + 2,
    ]
    .repeat(4);
    for (side, samples) in [
        (16, Data::I32(ramp)),
        (4, Data::I32(vec![i32::MAX; 16])),
        (4, Data::I64(wide)),
    ] {
        let image = Image::new([side, side], samples.clone(), Scaling::IDENTITY).unwrap();
        let mut r = round_trip(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                    .with_tiles([side, side]),
                None,
            )
        });
        assert_eq!(read_whole(&mut r, 1).samples, samples, "{side}×{side}");
    }
}

#[test]
fn integer_image_compression_preserves_bscale_bzero_and_blank() {
    // §10.2: the compressed tiles store *raw* stored integers, so BSCALE/BZERO and
    // the BLANK sentinel must survive in the rebuilt header.
    let samples: Vec<i16> = (0..24 * 16).map(|i| (i % 50) as i16 - 5).collect();
    let image = Image {
        shape: vec![24, 16],
        samples: Data::I16(samples.clone()),
        scaling: Scaling {
            bscale: 2.5,
            bzero: 100.0,
            blank: Some(-5),
        },
    };
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            None,
        )
    });
    let back = read_whole(&mut r, 1);
    assert_eq!(back.scaling.bscale, 2.5);
    assert_eq!(back.scaling.bzero, 100.0);
    assert_eq!(back.scaling.blank, Some(-5));

    match back.samples {
        Data::I16(v) => assert_eq!(v, samples, "raw samples"),
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
        samples: Data::I64(samples.clone()),
        scaling: Scaling::IDENTITY,
    };
    let mut r = round_trip(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice),
            None,
        )
    });
    let decoded = read_whole(&mut r, 1).samples;
    assert_eq!(decoded, Data::I64(samples));
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_full_decode_crosses_the_bounded_wave_boundary() {
    use crate::compress::decode::decode_wave_tile_count;
    use crate::compress::tile_geometry::TileGeometry;
    use crate::image_model::region::Region;
    use crate::internals::memory::written;

    // One column per tile makes one band of every tile, so the decode takes waves: one
    // more column than a wave holds.
    let height = 4096;
    let geometry = TileGeometry::new(&[1, height], &[1, height]);
    let columns = decode_wave_tile_count::<u8>(&geometry) + 1;
    let samples: Vec<u8> = (0usize..columns * height)
        .map(|index| (index.wrapping_mul(37) & 0xff) as u8)
        .collect();
    let image = Image::new(vec![columns, height], samples.clone(), Scaling::IDENTITY).unwrap();
    let bytes = written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::None).with_tiles([1, height]),
            None,
        )
    });
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        read_whole(&mut reader, 1).samples,
        Data::U8(samples.clone())
    );
    // A section crosses the wave boundary too.
    let section = reader
        .image(1)
        .unwrap()
        .read(Region::from(&[columns - 3..columns, 5..7]))
        .unwrap()
        .into_samples();
    let samples = &samples;
    let expected: Vec<u8> = (5..7)
        .flat_map(|y| (columns - 3..columns).map(move |x| samples[y * columns + x]))
        .collect();
    assert_eq!(section, Data::U8(expected));
}

#[test]
fn empty_images_round_trip() {
    // A `NAXIS = 0` image has no data array: the encoder emits an empty
    // `NAXIS2 = 0` ZIMAGE table, not a phantom tile, through both the integer and
    // the float paths. An empty first axis holds no pixels either.
    for (shape, samples) in [
        (vec![], Data::I16(Vec::new())),
        (vec![], Data::I32(Vec::new())),
        (vec![], Data::F32(Vec::new())),
        (vec![0], Data::I16(Vec::new())),
    ] {
        let image = Image {
            shape: shape.clone(),
            samples: samples.clone(),
            scaling: Scaling::IDENTITY,
        };
        let mut r = round_trip(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::GZIP),
                None,
            )
        });
        let back = read_whole(&mut r, 1);
        assert_eq!(back.shape, shape, "{samples:?}");
        assert_eq!(back.samples, samples);
    }
}

#[test]
fn compressed_image_with_an_overflowing_shape_is_refused_before_output() {
    let image = Image {
        shape: vec![usize::MAX, 2],
        samples: Data::I16(Vec::new()),
        scaling: Scaling::IDENTITY,
    };
    assert!(matches!(
        rejected_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            None
        )),
        FitsError::DataUnitOverflow
    ));
}
