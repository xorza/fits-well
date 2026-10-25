//! Parity with fpack: the quantized floats, the `HCOMPRESS_1` scales and the dither seed
//! cfitsio writes.

use crate::compression::encode::tests::*;

/// `ZDITHER0` is the byte sum of all the samples: two images whose first rows are
/// equal get different seeds, the same image gives the same bytes, and a fixed seed
/// is written as given and drives the decoder's dither.
#[test]
fn dither_seed_comes_from_every_sample() {
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
        let file = memory::written(|w| w.write_compressed_image(image.view(), &config, None));
        let mut ours = file.stream();
        let mut fpack = fixture.stream();
        assert_eq!(columns(&mut ours), columns(&mut fpack), "{fixture:?}");
        assert_eq!(
            memory::read_whole(&mut ours, 1).samples,
            memory::read_whole(&mut fpack, 1).samples,
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
        let file = memory::written(|w| w.write_compressed_image(image.view(), &config, None));
        let mut ours = file.stream();
        assert_eq!(
            ours.hdus()[1].header.get_real("ZVAL1").unwrap(),
            Some(zval1),
            "{fixture:?}"
        );
        let mut fpack = fixture.stream();
        assert_eq!(
            memory::read_whole(&mut ours, 1).samples,
            memory::read_whole(&mut fpack, 1).samples,
            "{fixture:?}"
        );
    }
}
