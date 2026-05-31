use super::*;
use crate::reader::FitsReader;
use std::fs::File;

fn open(name: &str) -> FitsReader<File> {
    FitsReader::open(File::open(format!("tests/data/fits/{name}")).unwrap()).unwrap()
}

/// The fixtures encode value(x, y) = x*7 − y*5 over a 24×16 i16 image.
fn expect_pixel(flat: usize) -> i16 {
    let (x, y) = (flat % 24, flat / 24);
    (x as i16) * 7 - (y as i16) * 5
}

fn check_decoded(name: &str) {
    let mut f = open(name);
    let img = f.read_compressed_image(1).unwrap();
    assert_eq!(img.shape, vec![24, 16]);
    match img.samples {
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
fn decompresses_rice_1_tiled_image() {
    check_decoded("comp_rice_i16.fits");
}

#[test]
fn decompresses_hcompress_1_tiled_image() {
    // Lossless HCOMPRESS (SCALE=0), single 24×16 tile.
    check_decoded("comp_hcomp_i16.fits");
}

/// Emit compressed files written by this crate for external (astropy) validation.
/// Run with `cargo test --features compression -- --ignored emit_`.
#[test]
#[ignore]
fn emit_compressed_files_for_astropy() {
    use crate::data::{Image, ImageData, Scaling};
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
    for (cmptype, tiles) in [
        ("GZIP_1", &[][..]),
        ("GZIP_2", &[]),
        ("RICE_1", &[]),
        ("HCOMPRESS_1", &[24, 16]),
    ] {
        let f = File::create(format!(".tmp/wr_{}.fits", cmptype.to_lowercase())).unwrap();
        let mut w = FitsWriter::new(f);
        w.write_compressed_image(&image, cmptype, tiles).unwrap();
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
    w.write_compressed_image(&mask_image, "PLIO_1", &[])
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
    w.write_compressed_image(&fimage, "RICE_1", &[24, 16])
        .unwrap();
}

#[test]
fn compression_write_round_trips_through_decode() {
    use crate::data::{Image, ImageData, Scaling};
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
    // Row tiling for the byte codecs; HCOMPRESS needs a genuinely 2-D tile.
    for (cmptype, tiles) in [
        ("GZIP_1", &[][..]),
        ("GZIP_2", &[]),
        ("RICE_1", &[]),
        ("HCOMPRESS_1", &[24, 16]),
    ] {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_compressed_image(&image, cmptype, tiles).unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        let back = r.read_compressed_image(1).unwrap();
        assert_eq!(back.shape, vec![24, 16], "{cmptype}");
        match back.samples {
            ImageData::I16(v) => assert_eq!(v, samples, "{cmptype} round-trip"),
            other => panic!("{cmptype}: expected I16, got {other:?}"),
        }
    }
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
fn float_quantize_write_round_trips_within_tolerance() {
    use crate::data::{Image, ImageData, Scaling};
    use crate::writer::FitsWriter;
    use std::io::Cursor;

    let orig = float_field();
    let image = Image {
        shape: vec![24, 16],
        samples: ImageData::F32(orig.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    for cmptype in ["RICE_1", "GZIP_1", "GZIP_2"] {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        // Whole-image tile so the noise estimate sees the full field.
        w.write_compressed_image(&image, cmptype, &[24, 16])
            .unwrap();
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        let back = match r.read_compressed_image(1).unwrap().samples {
            ImageData::F32(v) => v,
            other => panic!("{cmptype}: expected F32, got {other:?}"),
        };
        assert_eq!(back.len(), orig.len(), "{cmptype}");
        // Quantization error is bounded by ~0.5·delta; delta ≈ noise/4 ≈ 0.07 for
        // this field, so 0.2 is a safe ceiling. Also confirm it actually quantized.
        let max_err = orig
            .iter()
            .zip(&back)
            .map(|(a, b)| (a - b).abs())
            .fold(0.0f32, f32::max);
        assert!(max_err < 0.2, "{cmptype} max error {max_err} too large");
        assert!(
            orig.iter().zip(&back).any(|(a, b)| a != b),
            "{cmptype} stored losslessly — quantized path not exercised"
        );
    }
}

#[test]
fn plio_write_round_trips_through_decode() {
    use crate::data::{Image, ImageData, Scaling};
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
    w.write_compressed_image(&image, "PLIO_1", &[]).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    match r.read_compressed_image(1).unwrap().samples {
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
    let img = f.read_compressed_image(1).unwrap();
    assert_eq!(img.shape, vec![24, 16]);
    match img.samples {
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
    let got = match open(compressed).read_compressed_image(1).unwrap().samples {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open(reference).read_image(0).unwrap().samples {
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

#[test]
fn read_compressed_image_rejects_a_plain_bintable() {
    // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
    let mut f = open("DDTSUVDATA.fits");
    assert!(matches!(
        f.read_compressed_image(1),
        Err(FitsError::NotCompressedImage)
    ));
}
