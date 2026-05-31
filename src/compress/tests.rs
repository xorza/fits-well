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
    for cmptype in ["GZIP_1", "GZIP_2", "RICE_1"] {
        let f = File::create(format!(".tmp/wr_{}.fits", cmptype.to_lowercase())).unwrap();
        let mut w = FitsWriter::new(f);
        w.write_compressed_image(&image, cmptype, &[]).unwrap();
    }
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
    for cmptype in ["GZIP_1", "GZIP_2", "RICE_1"] {
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_compressed_image(&image, cmptype, &[]).unwrap(); // default row tiling
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        let back = r.read_compressed_image(1).unwrap();
        assert_eq!(back.shape, vec![24, 16], "{cmptype}");
        match back.samples {
            ImageData::I16(v) => assert_eq!(v, samples, "{cmptype} round-trip"),
            other => panic!("{cmptype}: expected I16, got {other:?}"),
        }
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
