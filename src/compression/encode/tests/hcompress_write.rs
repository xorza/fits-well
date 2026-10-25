//! `HCOMPRESS_1` writes: its image constraints, tiles, scales and round trips.

use crate::compression::encode::tests::*;

#[test]
fn writer_enforces_standard_image_constraints() {
    let line = Image::new(vec![4], vec![1i16, 2, 3, 4], Scaling::IDENTITY).unwrap();
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            line.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        }
    );

    let float = Image::new(vec![8, 8], float_field()[..64].to_vec(), Scaling::IDENTITY).unwrap();
    let dither2 =
        Quantization::relative(4.0, Dither::Subtractive2(DitherSeed::FROM_SAMPLES)).unwrap();
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            float.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_quantization(dither2),
            None
        )),
        FitsError::InvalidCompressionParameter {
            parameter: CompressionParameter::HcompressDither2
        }
    );
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            float.view(),
            &CompressionConfig::new(Compression::Plio),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::QuantizedFloatCodec { codec: "PLIO_1" }
        }
    );

    let extreme = Image::new(vec![4, 4], vec![i64::MAX; 16], Scaling::IDENTITY).unwrap();
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            extreme.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_tiles([4, 4]),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressRange
        }
    );

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
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            undefined.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::relative(2.0).unwrap()))
                .with_tiles([8, 8]),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::LossyHcompressNulls
        }
    );
}

#[test]
fn writer_converts_noise_multiplier_to_tile_scale() {
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
    let file = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::relative(2.0).unwrap()))
                .with_tiles([8, 8]),
            None,
        )
    });
    let mut r = file.stream();
    assert_matches!(
        r.hdus()[1].header.get("ZVAL1").unwrap(),
        Some(Value::Real(2.0))
    );
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
    assert_eq!(
        i32::from_be_bytes(stream[hcompress::stream_header::SCALE].try_into().unwrap()),
        10
    );
}

/// cfitsio's default `HCOMPRESS_1` tile height, pinned against `fpack -h -s 0`
/// (CFITSIO 4.070): short images are one tile, and each row reaches the next
/// candidate because every earlier one leaves a 1–3 row remainder (33 = 2·16 + 1,
/// 49 = 2·24 + 1, 241 = 8·30 + 1, 1681 = 60·28 + 1, …).
#[test]
fn default_tiles_follow_cfitsio() {
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
    let file = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
            None,
        )
    });
    let r = file.stream();
    let header = &r.hdus()[1].header;
    cards::assert_cards(header, &[("ZTILE1", 64.into()), ("ZTILE2", 24.into())]);
}

/// A tiling with a side under 4 pixels is refused, never adjusted: an explicit thin
/// tile, a remainder the explicit tile leaves, and an image too thin for any tile.
#[test]
fn tiles_under_four_pixels_are_refused() {
    let refused = |shape: Vec<usize>, tiles: &[usize]| {
        let image = Image::new(
            shape.clone(),
            vec![0i16; shape.iter().product()],
            Scaling::IDENTITY,
        )
        .unwrap();
        memory::refused_before_output(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                    .with_tiles(tiles.to_vec()),
                None,
            )
        })
    };
    // A tiling with one extent above 1 has no plane: `[17, 1]` is refused for its
    // dimensions, before its sides are measured.
    assert_matches!(
        refused(vec![17, 1], &[17, 1]),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        }
    );
    for (shape, tiles, axis, length) in [
        (vec![64, 33], &[64, 16][..], 1, 1),
        (vec![64, 3], &[][..], 1, 3),
        (vec![3, 64], &[][..], 0, 3),
    ] {
        let error = refused(shape.clone(), tiles);
        assert_matches!(error, FitsError::HcompressTileTooSmall { axis: a, length: l } if a == axis && l == length,
            "{shape:?} in {tiles:?} tiles: {error:?}",
        );
    }
}

/// Lossless tiles whose halves turn odd on both axes at once, the corner the inverse
/// transform once misplaced, round-trip exactly beside ones that never do (64×64,
/// 24×16); 6000×40 takes the default tiling, two 16-row tiles and an 8-row one.
/// HCOMPRESS takes a tile of any rank with two extents above 1, its plane in those two
/// axes: a cube in `[17, 9, 1]` tiles (axes 1 and 2), one in `[17, 1, 6]` tiles (axes 1
/// and 3), and the default tiling of a cube, `[17, 9, 1]` with all 9 rows, all
/// round-trip losslessly. A cube tile with three extents above 1 has no plane.
#[test]
fn cube_tiles_with_two_extents_above_one_round_trip() {
    let shape = [17usize, 9, 6];
    let samples = odd_plane(shape.iter().product());
    let image = Image::new(shape.to_vec(), samples.clone(), Scaling::IDENTITY).unwrap();
    let config = CompressionConfig::new(Compression::Hcompress(Hcompress::default()));
    for tiles in [&[17, 9, 1][..], &[17, 1, 6], &[]] {
        let file = memory::written(|w| {
            w.write_compressed_image(image.view(), &config.clone().with_tiles(tiles), None)
        });
        let mut r = file.stream();
        assert_eq!(
            memory::read_whole(&mut r, 1).samples,
            Data::I16(samples.clone()),
            "{tiles:?}"
        );
    }
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &config.clone().with_tiles([17, 9, 6]),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        }
    );
}

#[test]
fn tiles_odd_on_both_axes_round_trip() {
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
        let file = memory::written(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::Hcompress(Hcompress::default())),
                None,
            )
        });
        let mut r = file.stream();
        assert_eq!(
            memory::read_whole(&mut r, 1).samples,
            Data::I16(samples),
            "{shape:?}"
        );
        if shape == [6000, 40] {
            cards::assert_cards(
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

/// Lossless HCOMPRESS gives back i64 values whose sums pass 2³¹ in its 64-bit transform,
/// and writes `SCALE = 0` as `ZVAL1`, with `SMOOTH` left unnamed.
#[test]
fn lossless_keeps_values_whose_sums_pass_2_31() {
    let wide = [
        1i64 << 40,
        -(1i64 << 40),
        (1i64 << 40) + 1,
        -(1i64 << 40) + 2,
    ]
    .repeat(4);
    let image = Image::new([4, 4], wide.clone(), Scaling::IDENTITY).unwrap();
    let file = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_tiles([4, 4]),
            None,
        )
    });
    let mut r = file.stream();
    let header = &r.hdus()[1].header;
    assert_matches!(header.get("ZVAL1").unwrap(), Some(Value::Real(0.0)));
    assert_eq!(header.get_text("ZNAME2").unwrap(), None);
    assert_eq!(memory::read_whole(&mut r, 1).samples, Data::I64(wide));
}
