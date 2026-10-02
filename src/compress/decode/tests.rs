use crate::bintable::BinTable;
use crate::bintable::internals::table_header;
use crate::bintable::tform_kind::TformKind;
#[cfg(feature = "parallel")]
use crate::compress::decode::decode_wave_tile_count;
use crate::compress::decode::tiled_image::TiledImage;
use crate::compress::plane::IntBitpix;
#[cfg(feature = "parallel")]
use crate::compress::tile_geometry::TileGeometry;
use crate::compress::*;
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::error::FitsError;
use crate::error::Result;
use crate::hdu::HduKind;
use crate::hdu::HduRole;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::reader::internals::open_fixture;

/// Decode the tiled image `header` describes from its container `table`, resolving
/// the kind and geometry the way the reader's scan does.
fn decompress_image(header: &Header, table: &BinTable) -> Result<Image> {
    let kind = HduKind::classify(header, HduRole::Extension)?;
    let image = ImageGeometry::from_header(header, kind)?;
    let samples = TiledImage::new(header, &image)?.decode(header, table.view())?;
    Image::new_scaled(image.shape.clone(), samples, image.scaling)
}

/// The fixtures encode value(x, y) = x*7 − y*5 over a 24×16 i16 image.
fn expect_pixel(flat: usize) -> i16 {
    let (x, y) = (flat % 24, flat / 24);
    (x as i16) * 7 - (y as i16) * 5
}

fn check_decoded(name: &str) {
    let mut f = open_fixture(name);
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

/// A one-tile compressed image's container: `columns` as (`TTYPEn`, `TFORMn`) over a
/// row of `naxis1` bytes and a heap of `heap` bytes, encoding a `zbitpix` image of
/// `shape` in tiles of `tile` (no `ZTILEn` when it is empty).
fn compressed_image_header(
    cmptype: &str,
    zbitpix: i64,
    shape: &[i64],
    tile: &[i64],
    columns: &[(&str, &str)],
    naxis1: usize,
    heap: i64,
) -> Header {
    let tforms: Vec<&str> = columns.iter().map(|&(_, tform)| tform).collect();
    let mut h = table_header(naxis1, 1, &tforms);
    h.set_internal("PCOUNT", heap);
    for (index, &(ttype, _)) in columns.iter().enumerate() {
        h.set_internal(&format!("TTYPE{}", index + 1), ttype);
    }
    h.set_internal("ZIMAGE", true)
        .set_internal("ZCMPTYPE", cmptype)
        .set_internal("ZBITPIX", zbitpix)
        .set_internal("ZNAXIS", shape.len() as i64);
    for (index, &length) in shape.iter().enumerate() {
        h.set_internal(&format!("ZNAXIS{}", index + 1), length);
    }
    for (index, &length) in tile.iter().enumerate() {
        h.set_internal(&format!("ZTILE{}", index + 1), length);
    }
    h
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
    let got = match open_fixture(compressed).read_image(1).unwrap().decode() {
        ImageData::I32(v) => v,
        other => panic!("expected I32, got {other:?}"),
    };
    let want = match open_fixture(reference).read_image(0).unwrap().decode() {
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
    let got = match open_fixture("comp_nan_f32.fits")
        .read_image(1)
        .unwrap()
        .decode()
    {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open_fixture("comp_ref_nan_f32.fits")
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

#[test]
fn decompresses_gzip_2_tiled_image() {
    check_decoded("comp_gzip2_i16.fits");
}

#[test]
fn decompresses_plio_1_mask() {
    // PLIO fixture encodes value(x, y) = (x + y) % 7 as an i32 mask.
    let mut f = open_fixture("comp_plio_i32.fits");
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
    let got = match open_fixture(compressed).read_image(1).unwrap().decode() {
        ImageData::F32(v) => v,
        other => panic!("expected F32, got {other:?}"),
    };
    let want = match open_fixture(reference).read_image(0).unwrap().decode() {
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
fn decompresses_nocompress_tile_verbatim() {
    // A 2×2 i16 image as a single NOCOMPRESS tile: the COMPRESSED_DATA cell holds
    // the four pixels verbatim as big-endian i16.
    let h = compressed_image_header(
        "NOCOMPRESS",
        16,
        &[2, 2],
        &[2, 2],
        &[("COMPRESSED_DATA", "1PB(8)")],
        8, // one 1P descriptor
        8, // heap = 8 raw bytes
    );
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

    let mut invalid_hcompress = h.clone();
    invalid_hcompress
        .set_internal("ZCMPTYPE", "HCOMPRESS_1")
        .set_internal("ZNAXIS", 1)
        .set_internal("ZNAXIS1", 4);
    assert!(matches!(
        decompress_image(&invalid_hcompress, &table),
        Err(FitsError::UnsupportedCompression { name })
            if name == "HCOMPRESS_1 requires a two-dimensional image"
    ));
}

#[test]
fn compressed_integer_null_mask_restores_blank_pixels() {
    let gzip2 = gzip::gzip2_encode(
        &[0, 1],
        1,
        gzip::DEFAULT_GZIP_LEVEL,
        &mut gzip::GzipScratch::default(),
    );
    let rice = rice::rice_encode(
        &[0i64, 1],
        IntBitpix::U8,
        32,
        &mut rice::RiceScratch::default(),
    );
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
        let mut h = compressed_image_header(
            "NOCOMPRESS",
            16,
            &[2, 1],
            &[2, 1],
            &[
                ("COMPRESSED_DATA", "1PB(4)"),
                ("NULL PIXEL MASK", format!("1PB({})", mask.len()).as_str()),
            ],
            16,
            4 + mask.len() as i64,
        );
        h.set_internal("ZMASKCMP", codec)
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
            decompress_image(&h, &table).unwrap().samples,
            ImageData::I16(vec![10, -999]),
            "{codec}"
        );

        let mut missing_blank = h.clone();
        missing_blank.remove_all("BLANK");
        assert!(matches!(
            decompress_image(&missing_blank, &table),
            Err(FitsError::MissingKeyword { name: "BLANK" })
        ));
    }
}

#[test]
fn compressed_float_null_mask_restores_nan_pixels() {
    let mut h = compressed_image_header(
        "NOCOMPRESS",
        -32,
        &[2, 1],
        &[2, 1],
        &[
            ("COMPRESSED_DATA", "1PB(8)"),
            ("NULL_PIXEL_MASK", "1PB(2)"),
            ("ZSCALE", "1D"),
            ("ZZERO", "1D"),
        ],
        32,
        10,
    );
    h.set_internal("ZMASKCMP", "NOCOMPRESS");
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
    let ImageData::F32(values) = decompress_image(&h, &table).unwrap().samples else {
        panic!("expected F32")
    };
    assert!(values[0].is_nan());
    assert_eq!(values[1], 20.0);
}

#[test]
fn zblank_column_overrides_keyword_per_tile() {
    // A 2×1 float image, one NOCOMPRESS tile of quantized i32 [10, 99]. ZSCALE=2,
    // ZZERO=5 ⇒ pixel 0 = 25.0; pixel 1's quantized int equals the per-tile ZBLANK
    // *column* value (99), so it decodes to NaN — proving the column drives nulls.
    let h = compressed_image_header(
        "NOCOMPRESS",
        -32,
        &[2, 1],
        &[2, 1],
        &[
            ("COMPRESSED_DATA", "1PB(8)"),
            ("ZSCALE", "1D"),
            ("ZZERO", "1D"),
            ("ZBLANK", "1J"),
        ],
        28, // 1P(8) + 1D + 1D + 1J
        8,
    );
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

    for invalid in [0, 10_001] {
        let mut invalid_dither = h.clone();
        invalid_dither.set_internal("ZDITHER0", invalid);
        assert!(matches!(
            decompress_image(&invalid_dither, &table),
            Err(FitsError::KeywordOutOfRange { name: "ZDITHER0" })
        ));
    }

    let mut mistyped_header = h.clone();
    mistyped_header.set_internal("ZBITPIX", "not an integer");
    assert!(matches!(
        decompress_image(&mistyped_header, &table),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "ZBITPIX" && expected == "integer"
    ));

    let mut out_of_range_header = h.clone();
    out_of_range_header.set_internal("ZTILE1", 0);
    assert!(matches!(
        decompress_image(&out_of_range_header, &table),
        Err(FitsError::KeywordOutOfRange { name: "ZTILEn" })
    ));

    for (column, name, kind) in [
        (1, "ZSCALE", TformKind::I64),
        (2, "ZZERO", TformKind::I64),
        (3, "ZBLANK", TformKind::F32),
    ] {
        let mut malformed = table.clone();
        crate::bintable::internals::set_column_kind(&mut malformed, column, kind);
        assert!(matches!(
            decompress_image(&h, &malformed),
            Err(FitsError::TypeMismatch { name: actual, .. }) if actual == name
        ));
    }
}

#[test]
fn reading_a_plain_bintable_as_an_image_is_rejected() {
    // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
    let mut f = open_fixture("DDTSUVDATA.fits");
    // Public path: `read_image` sees a non-ZIMAGE bintable and rejects it as a
    // non-image (it never reaches the decompressor).
    assert!(matches!(f.read_image(1), Err(FitsError::NotAnImage)));
    // The decoder reads the geometry the same classification resolves.
    let table = f.read_table(1).unwrap();
    assert!(matches!(
        decompress_image(&f.hdus[1].header, &table),
        Err(FitsError::NotAnImage)
    ));
}

#[test]
fn compressed_image_rejects_short_tiles() {
    let h = compressed_image_header(
        "NOCOMPRESS",
        16,
        &[2],
        &[2],
        &[("COMPRESSED_DATA", "1PB(1)")],
        8,
        1,
    );
    let mut data = Vec::new();
    data.extend_from_slice(&1i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.push(0);
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            expected: 4,
            got: 1
        })
    ));

    let h = compressed_image_header(
        "GZIP_1",
        16,
        &[2],
        &[2],
        &[
            ("COMPRESSED_DATA", "1PB(0)"),
            ("UNCOMPRESSED_DATA", "1PI(1)"),
        ],
        16,
        2,
    );
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&1i32.to_be_bytes());
    data.extend_from_slice(&0i32.to_be_bytes());
    data.extend_from_slice(&1i16.to_be_bytes());
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            expected: 2,
            got: 1
        })
    ));
}

#[test]
fn decompress_image_rejects_overflowing_znaxis_product() {
    // ZNAXIS1·ZNAXIS2 = 5e9·5e9 = 2.5e19 overflows usize; decode must reject the
    // header up front (before allocating the output plane), not wrap to a small
    // buffer and then scatter out of bounds (R2-2).
    let h = compressed_image_header(
        "GZIP_1",
        16,
        &[5_000_000_000i64, 5_000_000_000i64],
        &[],
        &[("COMPRESSED_DATA", "1PB(0)")],
        8,
        0,
    );
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes()); // empty P descriptor: nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // offset
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decompress_image(&h, &table),
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
fn decompress_image_rejects_oversized_znaxis_product() {
    // ZNAXIS1 = 2^60 does NOT overflow usize (so the overflow guard passes), but
    // allocating that many bytes would abort the process. The output plane is
    // allocated fallibly (`try_reserve`), so decode must return a recoverable error.
    let h = compressed_image_header(
        "GZIP_1",
        8,
        &[1i64 << 60],
        &[],
        &[("COMPRESSED_DATA", "1PB(0)")],
        8,
        0,
    );
    let mut data = Vec::new();
    data.extend_from_slice(&0i32.to_be_bytes()); // empty P descriptor: nelem
    data.extend_from_slice(&0i32.to_be_bytes()); // offset
    let table = BinTable::from_data(&h, data).unwrap();
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataUnitTooLarge { .. })
    ));
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_decode_wave_budget_counts_per_tile_vectors() {
    // A tile retains its samples and its 24-byte `Vec`: 25 bytes for one u8 and 32
    // for one i64, so the 4 MiB budget holds ⌊4194304 / 25⌋ and 4194304 / 32 tiles.
    let single = TileGeometry::new(&[1, 4_194_304], &[1, 1]);
    assert_eq!(decode_wave_tile_count::<u8>(&single), 167_772);
    assert_eq!(decode_wave_tile_count::<i64>(&single), 131_072);
    // One tile larger than the whole budget still makes a wave of one.
    let whole = TileGeometry::new(&[1, 4_194_304], &[1, 4_194_304]);
    assert_eq!(decode_wave_tile_count::<u8>(&whole), 1);
}
