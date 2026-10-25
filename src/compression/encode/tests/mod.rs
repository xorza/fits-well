use crate::bintable::Table;
use crate::bintable::descriptor::DescriptorWidth;
use crate::compression::encode::{FloatImage, QuantizedFloatImage, hcompress_default_rows};
use crate::compression::half_step::assert_within_half_step;
use crate::compression::internals::{decompress_image, fixture_plane, odd_plane};
use crate::compression::plane::FloatSamples;
use crate::compression::quantization::{Dither, DitherSeed, Levels, QuantizationStep};
use crate::compression::*;
use crate::fits_error::CompressionFeature;
use crate::fits_writer::FitsWriter;
use crate::header_model::value::Value;
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::internals::physical;
use crate::internals::cards;
use crate::internals::fixture::Fixture;
use crate::internals::memory;
use crate::internals::rng::SplitMix;
use crate::internals::samples;
use crate::internals::samples::BITPIXES;
use crate::internals::whole_column::WholeColumn;
use crate::reader::FitsReader;
use crate::reader::source::Source;
use std::assert_matches;
use std::io::Cursor;
use std::iter;

mod float;
mod fpack;
mod hcompress_write;

/// The image codecs, each once.
const CODECS: [Compression; 6] = [
    Compression::GZIP,
    Compression::GZIP_SHUFFLED,
    Compression::Rice,
    Compression::Hcompress(Hcompress {
        scale: HcompressScale::Lossless,
    }),
    Compression::Plio,
    Compression::None,
];

/// What an image holds: a ramp of both signs, or each type's extremes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Content {
    Ramp,
    Extremes,
}

impl Content {
    /// `count` samples, at most 256, which a `u8` ramp holds.
    fn samples(self, bitpix: Bitpix, count: usize) -> Data {
        fn cycle<T: Copy>(values: &[T], count: usize) -> Vec<T> {
            values.iter().copied().cycle().take(count).collect()
        }
        match self {
            Self::Ramp => samples::ramp(bitpix, count),
            Self::Extremes => match bitpix {
                Bitpix::U8 => Data::U8(cycle(&[0, u8::MAX, 1, u8::MAX - 1], count)),
                Bitpix::I16 => Data::I16(cycle(&[i16::MIN, i16::MAX, 0, -1, i16::MIN + 1], count)),
                Bitpix::I32 => Data::I32(cycle(&[i32::MIN, i32::MAX, 0, -1, i32::MIN + 1], count)),
                // Neighbours whose differences pass 2⁶³, which Rice wraps, and values past
                // 2⁵³, which a float would round.
                Bitpix::I64 => Data::I64(cycle(
                    &[
                        i64::MIN,
                        i64::MAX,
                        0,
                        -1,
                        9_007_199_254_740_993,
                        -9_007_199_254_740_993,
                    ],
                    count,
                )),
                Bitpix::F32 => Data::F32(cycle(
                    &[
                        f32::MIN,
                        f32::MAX,
                        -0.0,
                        f32::INFINITY,
                        f32::MIN_POSITIVE / 2.0,
                        f32::from_bits(0x7FC0_1234),
                    ],
                    count,
                )),
                Bitpix::F64 => Data::F64(cycle(
                    &[
                        f64::MIN,
                        f64::MAX,
                        -0.0,
                        f64::NEG_INFINITY,
                        f64::MIN_POSITIVE / 2.0,
                        f64::from_bits(0x7FF8_0000_0000_1234),
                    ],
                    count,
                )),
            },
        }
    }
}

/// The refusal a lossless write of `content` of `bitpix` in `tiles`, empty for the default,
/// meets with
/// `compression`, if any: a float takes the gzip codecs alone, as cfitsio's
/// `imcomp_init_table` takes it; PLIO a value in `0..2²⁴`; HCOMPRESS a tile with exactly
/// two extents above 1, and values whose 64-bit transform does not overflow.
fn lossless_refusal(
    compression: Compression,
    bitpix: Bitpix,
    content: Content,
    tiles: &[usize],
) -> Option<FitsError> {
    let codec = compression.name();
    let negative = bitpix != Bitpix::U8;
    // HCOMPRESS's default tiles are cfitsio's bands of whole rows.
    let tile_plane = tiles.is_empty() || tiles.iter().filter(|&&extent| extent > 1).count() == 2;
    match compression {
        _ if bitpix.is_float() && !matches!(compression, Compression::Gzip(_)) => {
            Some(FitsError::UnsupportedCompression {
                feature: CompressionFeature::LosslessFloatCodec { codec },
            })
        }
        Compression::Plio if negative => {
            Some(FitsError::PlioValueOutOfRange { index: 0, value: 0 })
        }
        Compression::Hcompress(_) if !tile_plane => Some(FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions,
        }),
        Compression::Hcompress(_) if bitpix == Bitpix::I64 && content == Content::Extremes => {
            Some(FitsError::UnsupportedCompression {
                feature: CompressionFeature::HcompressRange,
            })
        }
        _ => None,
    }
}

/// Every codec × type × content × tiling round-trips a lossless write exactly, its
/// scaling and `BLANK` included, or meets the one refusal its codec's limits name. The
/// tilings: whole rows, 2-D tiles partial on both axes, the whole image, single rows, and a
/// cube's 3-D tiles and planes.
#[test]
fn every_codec_round_trips_every_type_and_tiling_or_names_its_limit() {
    let geometries: [(&[usize], &[usize]); 6] = [
        (&[16, 14], &[]),
        (&[16, 14], &[6, 5]),
        (&[16, 14], &[16, 14]),
        (&[16, 14], &[16, 1]),
        (&[8, 8, 4], &[4, 4, 4]),
        (&[8, 8, 4], &[8, 8, 1]),
    ];
    for compression in CODECS {
        for bitpix in BITPIXES {
            for content in [Content::Ramp, Content::Extremes] {
                for (shape, tiles) in geometries {
                    let case = format!("{compression:?} {bitpix:?} {content:?} {tiles:?}");
                    let samples = content.samples(bitpix, shape.iter().product());
                    let scaling = Scaling {
                        bscale: 2.5,
                        bzero: 100.0,
                        blank: bitpix.is_integer().then_some(1),
                    };
                    let image = Image::new(shape.to_vec(), samples.clone(), scaling).unwrap();
                    let mut config = CompressionConfig::new(compression).with_tiles(tiles);
                    if bitpix.is_float() {
                        config = config.with_quantization(Quantization::NONE);
                    }
                    let write = |w: &mut memory::MemoryWriter| {
                        w.write_compressed_image(image.view(), &config, None)
                    };
                    match lossless_refusal(compression, bitpix, content, tiles) {
                        Some(FitsError::PlioValueOutOfRange { .. }) => assert_matches!(
                            memory::refused_before_output(write),
                            FitsError::PlioValueOutOfRange { .. },
                            "{case}"
                        ),
                        Some(refusal) => assert_eq!(
                            memory::refused_before_output(write).to_string(),
                            refusal.to_string(),
                            "{case}"
                        ),
                        None => {
                            let file = memory::written(write);
                            let mut reader = file.stream();
                            let back = memory::read_whole(&mut reader, 1);
                            assert_eq!(back.shape, shape, "{case}");
                            assert_eq!(back.scaling, scaling, "{case}");
                            samples::assert_same_bits(&back.samples, &samples, &case);
                        }
                    }
                }
            }
        }
    }
}

/// A `NAXIS = 0` image has no data array: each codec that takes the type writes an empty `NAXIS2 = 0` table,
/// not a phantom tile, through the integer and the float paths. An empty first axis holds
/// no pixels either.
#[test]
fn every_codec_round_trips_an_empty_image() {
    for compression in CODECS {
        for (shape, samples) in [
            (vec![], Data::I16(Vec::new())),
            (vec![], Data::I32(Vec::new())),
            (vec![], Data::F32(Vec::new())),
            (vec![0], Data::I16(Vec::new())),
        ] {
            // PLIO takes no float tile, which the default quantizes.
            if compression == Compression::Plio && samples.bitpix().is_float() {
                continue;
            }
            let image = Image::new(shape.clone(), samples.clone(), Scaling::IDENTITY).unwrap();
            let file = memory::written(|w| {
                w.write_compressed_image(image.view(), &CompressionConfig::new(compression), None)
            });
            let mut reader = file.stream();
            assert_eq!(
                reader.hdus()[1].header.get_integer("NAXIS2").unwrap(),
                Some(0)
            );
            let back = memory::read_whole(&mut reader, 1);
            assert_eq!(back.shape, shape, "{compression:?} {samples:?}");
            assert_eq!(back.samples, samples, "{compression:?}");
        }
    }
}

/// In 6×5 tiles of a 16×14 image, sample 6 opens tile 1 and sample 16 is the second row
/// of tile 0. With both out of PLIO's range, the error names sample 6, the first in image
/// order, though tile 0 comes first, and for each tiling alike.
#[test]
fn plio_names_the_first_bad_sample_in_image_order() {
    let mut samples = vec![1i32; 16 * 14];
    samples[16] = -2;
    samples[6] = -1;
    let image = Image::new(vec![16, 14], samples, Scaling::IDENTITY).unwrap();
    for tiles in [&[6, 5][..], &[], &[16, 14]] {
        assert_matches!(
            memory::refused_before_output(|w| w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Plio).with_tiles(tiles),
                None
            )),
            FitsError::PlioValueOutOfRange {
                index: 6,
                value: -1
            },
            "{tiles:?}"
        );
    }
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
    let Data::F32(plane) = memory::read_whole(reader, 1).samples else {
        panic!("{context}: expected F32");
    };
    assert_eq!(plane.len(), original.len(), "{context}");
    for (index, (&o, &b)) in original.iter().zip(&plane).enumerate() {
        if o.is_nan() {
            assert!(b.is_nan(), "{context} pixel {index}: {b} for a NaN");
            continue;
        }
        assert_within_half_step(
            f64::from(o),
            f64::from(b),
            zscale[index / tile_len],
            Bitpix::F32,
            &format!("{context} pixel {index}"),
        );
    }
    plane
}

/// An integer image stores its values as they are: it takes `NONE` and refuses a
/// quantization level, as a table does; a tile of 0 along any axis is refused, not read
/// as 1.
#[test]
fn integer_image_takes_none_and_refuses_a_level_or_an_empty_tile() {
    let image = Image::new(vec![4, 2], vec![1i16; 8], Scaling::IDENTITY).unwrap();
    let rice = CompressionConfig::new(Compression::Rice);
    let lossless = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &rice.clone().with_quantization(Quantization::NONE),
            None,
        )
    });
    assert_eq!(
        memory::read_whole(&mut lossless.stream(), 1).samples,
        Data::I16(vec![1; 8])
    );
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &rice
                .clone()
                .with_quantization(Quantization::relative(4.0, Dither::None).unwrap()),
            None
        )),
        FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::IntegerQuantization
        }
    );
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &rice.with_tiles([4, 0]),
            None
        )),
        FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::EmptyTile { axis: 1 }
        }
    );
}

/// The writer sums and writes a compressed container's rows and then its cells in
/// order, with no copy into one buffer, so the file it writes verifies, an integer and a
/// quantized float container alike.
#[test]
fn container_written_in_parts_verifies() {
    use crate::internals::cards::VALID;
    let integer = Image::new(vec![24, 16], fixture_plane(), Scaling::IDENTITY).unwrap();
    let float = Image::new(vec![24, 16], float_field(), Scaling::IDENTITY).unwrap();
    // One NOCOMPRESS tile past the writer's 1 MiB gathering chunk goes out on its own,
    // between the small cells the chunk gathers.
    let large = Image::new(
        vec![1024, 1100],
        samples::byte_plane(1024 * 1100),
        Scaling::IDENTITY,
    )
    .unwrap();
    let file = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        let config = CompressionConfig::new(Compression::Rice).with_tiles([24, 3]);
        w.write_compressed_image(integer.view(), &config, None)?;
        w.write_compressed_image(float.view(), &config, None)?;
        w.write_compressed_image(
            large.view(),
            &CompressionConfig::new(Compression::None).with_tiles([1024, 1100]),
            None,
        )
    });
    let mut reader = file.stream();
    for hdu in [1, 2, 3] {
        assert_eq!(reader.verify_checksum(hdu).unwrap(), VALID, "HDU {hdu}");
    }
    assert_eq!(memory::read_whole(&mut reader, 1).samples, integer.samples);
    assert_eq!(memory::read_whole(&mut reader, 3).samples, large.samples);
}

/// A u16 image's `PLIO_1` stream holds its physical values, which PLIO takes as it
/// takes no negative value; the image reads back stored as `physical − 32768`.
#[test]
fn plio_writes_a_u16_image() {
    let physical = [0u16, 1, 32767, 32768, 65535, 7, 40000, 12345];
    let image = Image::from_u16(vec![8, 1], &physical).unwrap();
    let file = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Plio),
            None,
        )
    });
    let mut r = file.stream();
    let stored: Vec<i16> = physical
        .iter()
        .map(|&v| (i32::from(v) - 32768) as i16)
        .collect();
    assert_eq!(memory::read_whole(&mut r, 1).samples, Data::I16(stored));
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_full_decode_crosses_the_bounded_wave_boundary() {
    use crate::compression::decode::decode_wave_tile_count;
    use crate::compression::tile_geometry::TileGeometry;
    use crate::image_model::region::Region;

    // One column per tile makes one band of every tile, so the decode takes waves: one
    // more column than a wave holds.
    let height = 4096;
    let geometry = TileGeometry::new(&[1, height], [1, height]);
    let columns = decode_wave_tile_count::<u8>(&geometry) + 1;
    let samples = samples::byte_plane(columns * height);
    let image = Image::new(vec![columns, height], samples.clone(), Scaling::IDENTITY).unwrap();
    let bytes = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::None).with_tiles([1, height]),
            None,
        )
    });
    let mut reader = bytes.slice();
    assert_eq!(
        memory::read_whole(&mut reader, 1).samples,
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
fn compressed_image_with_an_overflowing_shape_is_refused_before_output() {
    // A literal: `Image::new` refuses this image, and the write's own check is
    // under test.
    let image = Image {
        shape: vec![usize::MAX, 2],
        samples: Data::I16(Vec::new()),
        scaling: Scaling::IDENTITY,
    };
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            None
        )),
        FitsError::DataUnitOverflow
    );
}
