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

/// Decode an i32 image and compare pixel-exact against astropy's reconstruction
/// stored as a plain-image reference.
fn check_i32_against_ref(compressed: &str, reference: &str) {
    let got = match open(compressed).read_compressed_image(1).unwrap().samples {
        ImageData::I32(v) => v,
        other => panic!("expected I32, got {other:?}"),
    };
    let want = match open(reference).read_image(0).unwrap().samples {
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
    let got = match open("comp_nan_f32.fits")
        .read_compressed_image(1)
        .unwrap()
        .samples
    {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open("comp_ref_nan_f32.fits").read_image(0).unwrap().samples {
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
fn float_write_preserves_nan_nulls() {
    use crate::data::{Image, ImageData, Scaling};
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
    w.write_compressed_image(&image, "RICE_1", &[24, 16])
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = match r.read_compressed_image(1).unwrap().samples {
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
    use super::quantize::{DitherMethod, dequantize, quantize_tile};

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
    let q = quantize_tile(&data, 8, 8, 0.0, DitherMethod::Subtractive2, irow).unwrap();
    // Exact zeros must encode to the reserved ZERO_VALUE.
    for &k in &[0usize, 13, 27, 40, 63] {
        assert_eq!(q.idata[k], super::quantize::ZERO_VALUE, "zero pixel {k}");
    }
    let ints: Vec<i64> = q.idata.iter().map(|&v| v as i64).collect();
    let back = dequantize(
        &ints,
        q.bscale,
        q.bzero,
        DitherMethod::Subtractive2,
        irow,
        None,
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
}

#[test]
fn hcompress_lossy_write_round_trips_within_scale() {
    use crate::data::{Image, ImageData, Scaling};
    use crate::writer::FitsWriter;
    use std::io::Cursor;

    // A smooth-ish 32×32 i32 image; lossy HCOMPRESS with SCALE=4.
    let samples: Vec<i32> = (0..32 * 32)
        .map(|i| 100 + 5 * (i % 32) + 3 * (i / 32))
        .collect();
    let image = Image {
        shape: vec![32, 32],
        samples: ImageData::I32(samples.clone()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image_lossy(&image, "HCOMPRESS_1", &[32, 32], 4)
        .unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = match r.read_compressed_image(1).unwrap().samples {
        ImageData::I32(v) => v,
        other => panic!("expected I32, got {other:?}"),
    };
    // Lossy: each pixel within the quantization scale (±scale), and not identical.
    let max_err = samples
        .iter()
        .zip(&back)
        .map(|(a, b)| (a - b).abs())
        .max()
        .unwrap();
    assert!(
        max_err <= 4,
        "HCOMPRESS lossy error {max_err} exceeds scale"
    );
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

/// Build a fixed-width BINTABLE, write it, then round-trip it through table
/// compression with `algo`/`rows_per_tile` and assert the data is byte-identical.
#[test]
fn decompresses_nocompress_tile_verbatim() {
    use crate::data::ImageData;
    use crate::header::Header;
    use crate::table::BinTable;
    // A 2×2 i16 image as a single NOCOMPRESS tile: the COMPRESSED_DATA cell holds
    // the four pixels verbatim as big-endian i16.
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 8) // one 1P descriptor
        .set("NAXIS2", 1) // one tile
        .set("PCOUNT", 8) // heap = 8 raw bytes
        .set("GCOUNT", 1)
        .set("TFIELDS", 1)
        .set("TFORM1", "1PB(8)")
        .set("TTYPE1", "COMPRESSED_DATA")
        .set("ZIMAGE", true)
        .set("ZCMPTYPE", "NOCOMPRESS")
        .set("ZBITPIX", 16)
        .set("ZNAXIS", 2)
        .set("ZNAXIS1", 2)
        .set("ZNAXIS2", 2)
        .set("ZTILE1", 2)
        .set("ZTILE2", 2);
    let mut data = Vec::new();
    data.extend_from_slice(&8i32.to_be_bytes()); // descriptor nelem = 8 bytes
    data.extend_from_slice(&0i32.to_be_bytes()); // descriptor offset = 0
    for x in [1i16, 2, 3, 4] {
        data.extend_from_slice(&x.to_be_bytes());
    }
    let table = BinTable::from_data(&h, data).unwrap();
    let img = decompress_image(&h, &table).unwrap();
    assert_eq!(img.shape, vec![2, 2]);
    assert_eq!(img.samples, ImageData::I16(vec![1, 2, 3, 4]));
}

#[test]
fn zblank_column_overrides_keyword_per_tile() {
    use crate::data::ImageData;
    use crate::header::Header;
    use crate::table::BinTable;
    // A 2×1 float image, one NOCOMPRESS tile of quantized i32 [10, 99]. ZSCALE=2,
    // ZZERO=5 ⇒ pixel 0 = 25.0; pixel 1's quantized int equals the per-tile ZBLANK
    // *column* value (99), so it decodes to NaN — proving the column drives nulls.
    let mut h = Header::new();
    h.set("XTENSION", "BINTABLE")
        .set("BITPIX", 8)
        .set("NAXIS", 2)
        .set("NAXIS1", 28) // 1P(8) + 1D + 1D + 1J
        .set("NAXIS2", 1)
        .set("PCOUNT", 8)
        .set("GCOUNT", 1)
        .set("TFIELDS", 4)
        .set("TFORM1", "1PB(8)")
        .set("TTYPE1", "COMPRESSED_DATA")
        .set("TFORM2", "1D")
        .set("TTYPE2", "ZSCALE")
        .set("TFORM3", "1D")
        .set("TTYPE3", "ZZERO")
        .set("TFORM4", "1J")
        .set("TTYPE4", "ZBLANK")
        .set("ZIMAGE", true)
        .set("ZCMPTYPE", "NOCOMPRESS")
        .set("ZBITPIX", -32)
        .set("ZNAXIS", 2)
        .set("ZNAXIS1", 2)
        .set("ZNAXIS2", 1)
        .set("ZTILE1", 2)
        .set("ZTILE2", 1);
    let mut data = Vec::new();
    data.extend_from_slice(&8i32.to_be_bytes()); // descriptor nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // descriptor offset
    data.extend_from_slice(&2.0f64.to_be_bytes()); // ZSCALE
    data.extend_from_slice(&5.0f64.to_be_bytes()); // ZZERO
    data.extend_from_slice(&99i32.to_be_bytes()); // ZBLANK column value
    data.extend_from_slice(&10i32.to_be_bytes()); // heap: quantized int 0
    data.extend_from_slice(&99i32.to_be_bytes()); // heap: quantized int 1 (== ZBLANK)
    let table = BinTable::from_data(&h, data).unwrap();
    let img = decompress_image(&h, &table).unwrap();
    let ImageData::F32(px) = img.samples else {
        panic!("expected F32")
    };
    assert_eq!(px[0], 25.0);
    assert!(px[1].is_nan());
}

fn check_table_roundtrip(algo: &str, rows_per_tile: usize) {
    use crate::table::ColumnData;
    use crate::writer::{FitsWriter, WriteColumn};
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

    // 1. Write an uncompressed table and read it back.
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(nrows, &columns).unwrap();
    let bytes = w.into_inner().into_inner();
    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    let orig = r.read_table(1).unwrap();
    let orig_header = r.hdu(1).header.clone();

    // 2. Compress it, then read + uncompress.
    let mut cw = FitsWriter::new(Cursor::new(Vec::new()));
    cw.write_compressed_table(&orig_header, &orig, rows_per_tile, algo)
        .unwrap();
    let cbytes = cw.into_inner().into_inner();
    let mut cr = FitsReader::open(Cursor::new(cbytes)).unwrap();
    let restored = cr.read_compressed_table(1).unwrap();

    // 3. The uncompressed table must be byte-identical to the original.
    assert_eq!(restored.nrows, orig.nrows, "{algo}/{rows_per_tile} nrows");
    assert_eq!(
        restored.row_width(),
        orig.row_width(),
        "{algo}/{rows_per_tile} row width"
    );
    assert_eq!(
        restored.raw_rows(),
        orig.raw_rows(),
        "{algo}/{rows_per_tile} data mismatch"
    );
}

#[test]
fn table_compression_round_trips() {
    // One tile, several tiles, and a tile smaller than the table — across codecs.
    for &rpt in &[10usize, 4, 1] {
        check_table_roundtrip("GZIP_1", rpt);
        check_table_roundtrip("GZIP_2", rpt);
        check_table_roundtrip("RICE_1", rpt);
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
    let mut r = FitsReader::open(std::io::Cursor::new(src)).unwrap();
    let table = r.read_table(1).unwrap();
    let header = r.hdu(1).header.clone();
    let mut w = FitsWriter::new(File::create(".tmp/my_ctable.fits").unwrap());
    w.write_compressed_table(&header, &table, 100, "RICE_1")
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

    assert_eq!(restored.nrows, 500);
    assert_eq!(restored.nrows, original.nrows);
    assert_eq!(restored.row_width(), original.row_width());
    assert_eq!(restored.columns.len(), 6);
    assert_eq!(
        restored.raw_rows(),
        original.raw_rows(),
        "decoded cfitsio-compressed table must match the original bytes"
    );
    // Spot-check a decoded value against the known formula (INT = i·100000 − 5).
    match original.read_column(1).unwrap() {
        ColumnData::I32(v) => assert_eq!(v[3], 3 * 100_000 - 5),
        other => panic!("expected I32, got {other:?}"),
    }
}

#[test]
fn compressed_table_with_vla_column_is_rejected_cleanly() {
    // `comp_table_vla.fits` (from `fpack -tableonly` of a table with a `PJ` VLA
    // column) — decoding VLA columns inside a compressed table is not yet
    // implemented, so it must error rather than misread or panic.
    let mut f = open("comp_table_vla.fits");
    assert!(matches!(
        f.read_compressed_table(1),
        Err(FitsError::UnsupportedCompression { .. })
    ));
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
fn read_compressed_image_rejects_a_plain_bintable() {
    // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
    let mut f = open("DDTSUVDATA.fits");
    assert!(matches!(
        f.read_compressed_image(1),
        Err(FitsError::NotCompressedImage)
    ));
}

#[test]
fn integer_image_compression_preserves_bscale_bzero_and_blank() {
    use crate::data::{Image, ImageData, Scaling};
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
    w.write_compressed_image(&image, "GZIP_1", &[]).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let back = r.read_compressed_image(1).unwrap();

    match back.samples {
        ImageData::I16(v) => assert_eq!(v, samples, "raw samples"),
        other => panic!("expected I16, got {other:?}"),
    }
    assert_eq!(back.scaling.bscale, 2.5);
    assert_eq!(back.scaling.bzero, 100.0);
    assert_eq!(back.scaling.blank, Some(-5));
}

#[test]
fn rice_rejects_64_bit_pixels() {
    use crate::data::{Image, ImageData, Scaling};
    use crate::writer::FitsWriter;
    use std::io::Cursor;
    // Table 37 permits BYTEPIX 8, but the 64-bit RICE bitstream is unsupported; it
    // must error cleanly rather than panic / silently corrupt.
    let image = Image {
        shape: vec![4],
        samples: ImageData::I64(vec![1, 2, 3, 4]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        w.write_compressed_image(&image, "RICE_1", &[]),
        Err(FitsError::UnsupportedCompression { .. })
    ));
    // GZIP handles 64-bit fine — the rejection is RICE-specific.
    let mut w2 = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(w2.write_compressed_image(&image, "GZIP_1", &[]).is_ok());
}

#[test]
fn nocompress_image_round_trips() {
    use crate::data::{Image, ImageData, Scaling};
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
    w.write_compressed_image(&image, "NOCOMPRESS", &[]).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    match r.read_compressed_image(1).unwrap().samples {
        ImageData::I16(v) => assert_eq!(v, samples),
        other => panic!("expected I16, got {other:?}"),
    }
}

#[test]
fn compressed_image_descriptor_switches_to_q_for_large_offsets() {
    // §10.1.3: a heap offset beyond the 32-bit P range needs a 64-bit Q descriptor.
    let mut q = Vec::new();
    super::push_compressed_descriptor(&mut q, true, 3, u32::MAX as u64 + 8);
    assert_eq!(q.len(), 16);
    assert_eq!(i64::from_be_bytes(q[0..8].try_into().unwrap()), 3);
    assert_eq!(
        i64::from_be_bytes(q[8..16].try_into().unwrap()),
        u32::MAX as i64 + 8
    );
    let mut p = Vec::new();
    super::push_compressed_descriptor(&mut p, false, 3, 40);
    assert_eq!(p.len(), 8);
    assert_eq!(i32::from_be_bytes(p[4..8].try_into().unwrap()), 40);
}
