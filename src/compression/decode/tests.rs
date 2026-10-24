use crate::bintable::Table;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::internals;
use crate::compression::decode::decode_sample::DecodeSample;
#[cfg(feature = "parallel")]
use crate::compression::decode::decode_wave_tile_count;
use crate::compression::decode::tile_decoder::Overshoot;
use crate::compression::decode::tiled_image::TiledImage;
use crate::compression::internals::{mask, odd_plane, ramp};
#[cfg(feature = "parallel")]
use crate::compression::tile_geometry::TileGeometry;
use crate::compression::*;
use crate::fits_error::CompressedDataDefect;
use crate::fits_error::CompressionFeature;
use crate::fits_error::ExpectedColumn;
use crate::fits_error::ExpectedHdu;
use crate::fits_error::SizeUnit;
use crate::fits_error::ValueKind;
use crate::hdu::HduKind;
use crate::hdu::HduRole;
use crate::hdu::image_geometry::ImageGeometry;
use crate::header_model::Header;
use crate::header_model::record_name::RecordName;
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::internals::container::{OneTile, be};
use crate::internals::fixture::Fixture;
use crate::internals::memory::read_whole;
use crate::internals::samples::{assert_same_bits, nan_blind_bits};

/// Decode the tiled image `header` describes from its container `table`, resolving
/// the kind and geometry the way the reader's scan does.
fn decompress_image(header: &Header, table: &Table) -> Result<Image> {
    let kind = HduKind::classify(header, HduRole::Extension)?;
    let image = ImageGeometry::from_header(header, kind)?;
    let samples = TiledImage::new(header, &image)?.decode_table(table)?;
    Image::new(image.shape.clone(), samples, image.scaling)
}

/// Each integer fixture decodes to the plane it was written from.
#[test]
fn decompresses_the_integer_codec_fixtures() {
    // `fpack -h -s 0 -t 17,17` (CFITSIO 4.070) of the plane `(7919·i + 13) mod 2001 − 1000`:
    // both halves odd at every level of the inverse transform, the corner case 24×16 never reaches.
    let odd = odd_plane(17 * 17);
    for (name, shape, want) in [
        (Fixture::CompGzipI16, [24, 16], Data::I16(ramp())),
        (Fixture::CompGzip2I16, [24, 16], Data::I16(ramp())),
        (Fixture::CompRiceI16, [24, 16], Data::I16(ramp())),
        // Lossless HCOMPRESS (SCALE=0), one 24×16 tile.
        (Fixture::CompHcompI16, [24, 16], Data::I16(ramp())),
        (Fixture::CompHcompOddI16, [17, 17], Data::I16(odd)),
        (Fixture::CompPlioI32, [24, 16], Data::I32(mask())),
    ] {
        let mut reader = name.stream();
        let image = read_whole(&mut reader, 1);
        assert_eq!(image.shape, shape, "{name:?}");
        assert_eq!(image.samples, want, "{name:?}");
    }
}

/// Each lossy or float fixture decodes bit for bit to astropy's or cfitsio's
/// reconstruction, stored as a plain image beside it; a NaN matches any NaN.
#[test]
fn decompresses_to_the_reference_reconstructions() {
    for (compressed, reference, nulls) in [
        // Lossy HCOMPRESS, SCALE=4: undigitize multiplies by the scale.
        (Fixture::CompHcompLossy, Fixture::CompRefHcompLossy, 0),
        // `fpack -h -s 4` of the 17×17 lossless fixture's plane against `funpack`'s
        // reconstruction (CFITSIO 4.070): the odd-on-both-axes corner under a scale.
        (Fixture::CompHcompLossyOdd, Fixture::CompRefHcompLossyOdd, 0),
        // SMOOTH=1 smooths during the inverse transform.
        (Fixture::CompHcompSmooth, Fixture::CompRefHcompSmooth, 0),
        // Smooth data stored losslessly: ZSCALE=0, the floats gzip'd in
        // GZIP_COMPRESSED_DATA.
        (Fixture::CompRicefNodither, Fixture::CompRefF32, 0),
        // Noisy data quantized per tile, RICE-packed, read as ZSCALE·int + ZZERO.
        (Fixture::CompRicefQuant, Fixture::CompRefQuantF32, 0),
        (Fixture::CompDither2F32, Fixture::CompRefDither2F32, 0),
        // SUBTRACTIVE_DITHER_1 with ZBLANK: two pixels are null.
        (Fixture::CompNanF32, Fixture::CompRefNanF32, 2),
    ] {
        let got = nan_blind_bits(&read_whole(&mut compressed.stream(), 1).samples);
        let want = nan_blind_bits(&read_whole(&mut reference.stream(), 0).samples);
        assert_eq!(got, want, "{compressed:?}");
        assert_eq!(
            got.iter().filter(|v| v.is_none()).count(),
            nulls,
            "{compressed:?}"
        );
    }
}

#[test]
fn decompresses_nocompress_tile_verbatim() {
    // A 2×2 i16 image as a single NOCOMPRESS tile: the COMPRESSED_DATA cell holds
    // the four pixels verbatim as big-endian i16.
    let tile = OneTile::new("NOCOMPRESS", 16, &[2, 2]).heap(
        "COMPRESSED_DATA",
        'B',
        &be(&[1i16, 2, 3, 4], i16::to_be_bytes),
    );
    let (h, table) = (tile.header(), tile.table());
    let img = decompress_image(&h, &table).unwrap();
    assert_eq!(img.shape, vec![2, 2]);
    assert_eq!(img.samples, Data::I16(vec![1, 2, 3, 4]));

    let mut invalid_hcompress = h;
    invalid_hcompress
        .set_internal(keywords::ZCMPTYPE, "HCOMPRESS_1")
        .set_internal(keywords::ZNAXIS, 1)
        .set_internal("ZNAXIS1", 4);
    assert!(matches!(
        decompress_image(&invalid_hcompress, &table),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        })
    ));
}

/// A tile's values must fit the image's `BITPIX` before they narrow. The 16-bit image
/// here holds 40000 and −40000, which `i16` cannot: stored losslessly (a `1PJ`
/// `UNCOMPRESSED_DATA` cell) that is corrupt data and refused, while HCOMPRESS, whose
/// lossy reconstruction may overshoot, clamps them to 32767 and −32768 — where a
/// plain `as i16` gave −25536 and 25536.
#[test]
fn decoded_values_fit_the_image_bitpix() {
    let values = [40_000i64, -40_000, 7, 8];
    let tile = OneTile::new("RICE_1", 16, &[2, 2]).heap(
        "UNCOMPRESSED_DATA",
        'J',
        &be(&[40_000i32, -40_000, 7, 8], i32::to_be_bytes),
    );
    let (uncompressed, table) = (tile.header(), tile.table());
    assert!(matches!(
        decompress_image(&uncompressed, &table),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::ValueOutOfRange,
        })
    ));

    let stream =
        TileCodec::named("HCOMPRESS_1", IntBitpix::I16).encoded(&[2, 2], IntBitpix::I16, &values);
    let tile = OneTile::new("HCOMPRESS_1", 16, &[2, 2]).heap("COMPRESSED_DATA", 'B', &stream);
    let (hcompressed, table) = (tile.header(), tile.table());
    assert_eq!(
        decompress_image(&hcompressed, &table).unwrap().samples,
        Data::I16(vec![i16::MAX, i16::MIN, 7, 8])
    );

    // A fallback cell of the same image is stored losslessly: out of range, it is
    // corrupt rather than clamped.
    let tile = OneTile::new("HCOMPRESS_1", 16, &[2, 2])
        .heap("COMPRESSED_DATA", 'B', &[])
        .heap(
            "UNCOMPRESSED_DATA",
            'J',
            &be(&[40_000i32, -40_000, 7, 8], i32::to_be_bytes),
        );
    assert!(matches!(
        decompress_image(&tile.header(), &tile.table()),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::ValueOutOfRange,
        })
    ));

    // The bounds themselves fit; one past them clamps, or is refused.
    let mut bytes = [-1, 0, 255, 256];
    <u8 as DecodeSample>::fit(&mut bytes, Overshoot::Clamp).unwrap();
    assert_eq!(bytes, [0, 0, 255, 255]);
    assert!(matches!(
        <u8 as DecodeSample>::fit(&mut [0, 256], Overshoot::Refuse),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::ValueOutOfRange
        })
    ));
    let mut words = [i64::from(i32::MIN), i64::from(i32::MAX)];
    <i32 as DecodeSample>::fit(&mut words, Overshoot::Refuse).unwrap();
    assert!(matches!(
        <i32 as DecodeSample>::fit(&mut [i64::from(i32::MAX) + 1], Overshoot::Refuse),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::ValueOutOfRange
        })
    ));
}

#[test]
fn compressed_integer_null_mask_restores_blank_pixels() {
    for (codec, mask) in ["GZIP_1", "GZIP_2", "RICE_1", "PLIO_1", "NOCOMPRESS"].map(|codec| {
        (
            codec,
            TileCodec::named(codec, IntBitpix::U8).encoded(&[2], IntBitpix::U8, &[0, 1]),
        )
    }) {
        let tile = OneTile::new("NOCOMPRESS", 16, &[2, 1])
            .heap("COMPRESSED_DATA", 'B', &be(&[10i16, 20], i16::to_be_bytes))
            .heap("NULL PIXEL MASK", 'B', &mask);
        let (mut h, table) = (tile.header(), tile.table());
        h.set_internal(keywords::ZMASKCMP, codec)
            .set_internal(keywords::BLANK, -999);
        assert_eq!(
            decompress_image(&h, &table).unwrap().samples,
            Data::I16(vec![10, -999]),
            "{codec}"
        );

        let mut missing_blank = h.clone();
        missing_blank.remove_all(keywords::BLANK).unwrap();
        assert!(matches!(
            decompress_image(&missing_blank, &table),
            Err(FitsError::MissingKeyword {
                name: RecordName::Keyword(keywords::BLANK)
            })
        ));
    }
}

/// `fpack -g1 -q 0` and `fpack -g2 -q 0` (CFITSIO 4.070) of a 24×16 plane
/// `((7919·i + 13) mod 2001) / 7 − 100`, which is `(odd_plane + 1000) / 7 − 100`, NaN
/// where `i mod 97 = 5`: `ZQUANTIZ = 'NONE'`, no `ZSCALE`/`ZZERO` column, each tile's
/// floats gzip'd (and shuffled for `GZIP_2`).
#[test]
fn decompresses_the_lossless_float_fixtures() {
    let plane = || {
        odd_plane(24 * 16)
            .into_iter()
            .enumerate()
            .map(|(i, v)| (i % 97 != 5).then(|| f64::from(v + 1000) / 7.0 - 100.0))
    };
    let want: Vec<Option<u64>> = plane().map(|v| v.map(f64::to_bits)).collect();
    #[expect(
        clippy::cast_possible_truncation,
        reason = "the f32 fixtures hold the f64 plane rounded to f32"
    )]
    let want_f32: Vec<Option<u64>> = plane()
        .map(|v| v.map(|v| u64::from((v as f32).to_bits())))
        .collect();
    for (fixture, bitpix, want) in [
        (Fixture::CompLosslessGzip1F64, Bitpix::F64, &want),
        (Fixture::CompLosslessGzip2F64, Bitpix::F64, &want),
        (Fixture::CompLosslessGzip1F32, Bitpix::F32, &want_f32),
        (Fixture::CompLosslessGzip2F32, Bitpix::F32, &want_f32),
    ] {
        let got = read_whole(&mut fixture.stream(), 1).samples;
        assert_eq!(got.bitpix(), bitpix, "{fixture:?}");
        assert_eq!(&nan_blind_bits(&got), want, "{fixture:?}");
    }
}

/// A float image with no `ZSCALE` and no `ZZERO` column is lossless whatever its
/// `ZQUANTIZ` says, as cfitsio reads it; lossless floats take only a gzip codec.
#[test]
fn a_float_image_without_scale_columns_is_lossless() {
    let values = [1.5f32, -2.25, 3.0, 100.125, 0.0, 7.75, -8.5, 42.0];
    let raw: Vec<u8> = values.iter().flat_map(|v| v.to_be_bytes()).collect();
    let heap = TileCodec::Gzip1 { level: 6 }.encoded_bytes(&raw);
    let tile = OneTile::new("GZIP_1", -32, &[4, 2]).heap("COMPRESSED_DATA", 'B', &heap);
    let (h, table) = (tile.header(), tile.table());
    let mut dithered = h.clone();
    dithered.set_internal(keywords::ZQUANTIZ, "SUBTRACTIVE_DITHER_1");
    for header in [&h, &dithered] {
        assert_eq!(
            decompress_image(header, &table).unwrap().samples,
            Data::F32(values.to_vec())
        );
    }

    let mut rice = h;
    rice.set_internal(keywords::ZCMPTYPE, "RICE_1");
    assert!(matches!(
        decompress_image(&rice, &table),
        Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::LosslessFloatCodec { codec: "RICE_1" }
        })
    ));
}

/// A u16 image's `PLIO_1` stream holds the physical values, and the reader subtracts
/// 32768 for the stored ones, as cfitsio's `imcomp_decompress_tile` does. The stream
/// is built here: cfitsio 4.7.0's `fpack -p` aborts on a u16 image.
#[test]
fn plio_of_a_u16_image_holds_its_physical_values() {
    let plio_image = |physical: &[i64], bzero: Option<i64>| {
        let stream = TileCodec::Plio(plio::Codec { offset: 0 }).encoded(
            &[physical.len()],
            IntBitpix::I16,
            physical,
        );
        let width = i64::try_from(physical.len()).unwrap();
        let tile = OneTile::new("PLIO_1", 16, &[width, 1]).heap("COMPRESSED_DATA", 'I', &stream);
        let (mut h, table) = (tile.header(), tile.table());
        if let Some(bzero) = bzero {
            h.set_internal(keywords::BZERO, bzero);
        }
        decompress_image(&h, &table).unwrap().samples
    };
    // Stored = physical − 32768.
    assert_eq!(
        plio_image(&[0, 1, 32767, 32768, 65535], Some(32768)),
        Data::I16(vec![-32768, -32767, -1, 0, 32767])
    );
    // Without BZERO = 32768 the stream holds the stored values.
    assert_eq!(plio_image(&[0, 1, 2, 3], None), Data::I16(vec![0, 1, 2, 3]));
}

#[test]
fn compressed_float_null_mask_restores_nan_pixels() {
    let tile = OneTile::new("NOCOMPRESS", -32, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &be(&[10i32, 20], i32::to_be_bytes))
        .heap("NULL_PIXEL_MASK", 'B', &[1, 0])
        .fixed("ZSCALE", "1D", &1.0f64.to_be_bytes())
        .fixed("ZZERO", "1D", &0.0f64.to_be_bytes());
    let (mut h, table) = (tile.header(), tile.table());
    h.set_internal(keywords::ZMASKCMP, "NOCOMPRESS");
    let Data::F32(values) = decompress_image(&h, &table).unwrap().samples else {
        panic!("expected F32")
    };
    assert!(values[0].is_nan());
    assert_eq!(values[1], 20.0);
}

#[test]
fn zblank_column_marks_each_tiles_nulls() {
    // A 2×1 float image, one NOCOMPRESS tile of quantized i32 [10, 99]. ZSCALE=2,
    // ZZERO=5 ⇒ pixel 0 = 25.0; pixel 1's quantized int equals the per-tile ZBLANK
    // *column* value (99), so it decodes to NaN — proving the column drives nulls.
    let tile = OneTile::new("NOCOMPRESS", -32, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &be(&[10i32, 99], i32::to_be_bytes))
        .fixed("ZSCALE", "1D", &2.0f64.to_be_bytes())
        .fixed("ZZERO", "1D", &5.0f64.to_be_bytes())
        .fixed("ZBLANK", "1J", &99i32.to_be_bytes());
    let (h, table) = (tile.header(), tile.table());
    let img = decompress_image(&h, &table).unwrap();
    let Data::F32(px) = img.samples else {
        panic!("expected F32")
    };
    assert_eq!(px[0], 25.0);
    assert!(px[1].is_nan());

    // `ZDITHER0` seeds a dither, so it is checked only where one is applied.
    for invalid in [0, 10_001] {
        let mut undithered = h.clone();
        undithered.set_internal(keywords::ZDITHER0, invalid);
        assert_same_bits(
            &decompress_image(&undithered, &table).unwrap().samples,
            &decompress_image(&h, &table).unwrap().samples,
            &format!("ZDITHER0 = {invalid} without a dither"),
        );
        let mut dithered = undithered;
        dithered.set_internal(keywords::ZQUANTIZ, "SUBTRACTIVE_DITHER_1");
        assert!(matches!(
            decompress_image(&dithered, &table),
            Err(FitsError::KeywordOutOfRange {
                keyword: keywords::ZDITHER0
            })
        ));
    }

    let mut mistyped_header = h.clone();
    mistyped_header.set_internal(keywords::ZBITPIX, "not an integer");
    assert!(matches!(
        decompress_image(&mistyped_header, &table),
        Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), expected: ValueKind::Integer })
            if name == "ZBITPIX"
    ));

    let mut out_of_range_header = h.clone();
    out_of_range_header.set_internal("ZTILE1", 0);
    assert!(matches!(
        decompress_image(&out_of_range_header, &table),
        Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "ZTILE1"
    ));

    // A scale or zero column of any real or integer type is read as cfitsio reads it, as
    // doubles; a complex one is not a number, and an integer null not a float.
    for (column, name, kind) in [
        (1, "ZSCALE", ElementKind::ComplexF32),
        (2, "ZZERO", ElementKind::ComplexF32),
        (3, "ZBLANK", ElementKind::F32),
    ] {
        let mut malformed = table.clone();
        internals::set_column_kind(&mut malformed, column, kind);
        assert!(matches!(
            decompress_image(&h, &malformed),
            Err(FitsError::ColumnKindMismatch { column: got, expected })
                if got == column
                    && expected == if name == "ZBLANK" {
                        ExpectedColumn::Integer
                    } else {
                        ExpectedColumn::Numeric
                    }
        ));
    }
    let single = OneTile::new("NOCOMPRESS", -32, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &be(&[10i32, 99], i32::to_be_bytes))
        .fixed("ZSCALE", "1E", &2.0f32.to_be_bytes())
        .fixed("ZZERO", "1I", &5i16.to_be_bytes());
    assert_eq!(
        decompress_image(&single.header(), &single.table())
            .unwrap()
            .samples,
        Data::F32(vec![25.0, 203.0])
    );
}

/// `ZSCALE` and `ZZERO` resolve each on its own, as cfitsio's
/// `imcomp_get_compressed_image_par` resolves them: the keyword fixes the value for every
/// tile, else the column gives it per tile, else the default (1 and 0). The quantized
/// integers [10, 99] with a scale of 2 and a zero of 5 are 25 and 203.
#[test]
fn scale_and_zero_keywords_fix_every_tile() {
    let quantized = be(&[10i32, 99], i32::to_be_bytes);
    let decoded = |h: &Header, table: &Table| decompress_image(h, table).unwrap().samples;

    // A `ZSCALE` keyword beside a `ZZERO` column.
    let tile = OneTile::new("NOCOMPRESS", -32, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &quantized)
        .fixed("ZZERO", "1D", &5.0f64.to_be_bytes());
    let mut h = tile.header();
    h.set_internal(keywords::ZSCALE, 2.0);
    assert_eq!(decoded(&h, &tile.table()), Data::F32(vec![25.0, 203.0]));

    // The keyword wins over a column of the same parameter: 10·2 + 5, not 10·3 + 5.
    let tile = OneTile::new("NOCOMPRESS", -32, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &quantized)
        .fixed("ZSCALE", "1D", &3.0f64.to_be_bytes())
        .fixed("ZZERO", "1D", &5.0f64.to_be_bytes());
    let mut h = tile.header();
    h.set_internal(keywords::ZSCALE, 2.0);
    assert_eq!(decoded(&h, &tile.table()), Data::F32(vec![25.0, 203.0]));

    // Both as keywords: the image is quantized, not lossless. Stored with GZIP_1, the
    // quantized integers are as long as the floats, so a lossless read would have taken
    // their bits for floats; with `ZBLANK = 99` the second is a null.
    let tile = OneTile::new("GZIP_1", -32, &[2, 1]).heap(
        "COMPRESSED_DATA",
        'B',
        &TileCodec::named("GZIP_1", IntBitpix::U8).encoded_bytes(&quantized),
    );
    let mut h = tile.header();
    h.set_internal(keywords::ZSCALE, 2.0)
        .set_internal(keywords::ZZERO, 5.0);
    assert_eq!(decoded(&h, &tile.table()), Data::F32(vec![25.0, 203.0]));
    h.set_internal(keywords::ZBLANK, 99);
    assert_eq!(
        nan_blind_bits(&decoded(&h, &tile.table())),
        [Some(u64::from(25.0f32.to_bits())), None]
    );
    // With neither, the same cell is the floats themselves.
    let h = tile.header();
    assert_eq!(
        nan_blind_bits(&decoded(&h, &tile.table())),
        nan_blind_bits(&Data::F32(vec![f32::from_bits(10), f32::from_bits(99)]))
    );
}

/// An integer image's tiles store their nulls as `ZBLANK`: as a keyword, else as a
/// column, else `BLANK`. The image's null value is `BLANK`, else the `ZBLANK` keyword,
/// and a stored null of another value reads as the image's.
#[test]
fn an_integer_image_maps_its_stored_nulls_to_blank() {
    let tile = OneTile::new("NOCOMPRESS", 16, &[2, 1]).heap(
        "COMPRESSED_DATA",
        'B',
        &be(&[10i16, -32768], i16::to_be_bytes),
    );
    let table = tile.table();
    // `ZBLANK` alone is the image's null value; the samples are the stored ones.
    let mut h = tile.header();
    h.set_internal(keywords::ZBLANK, -32768);
    let image = decompress_image(&h, &table).unwrap();
    assert_eq!(image.scaling.blank, Some(-32768));
    assert_eq!(image.samples, Data::I16(vec![10, -32768]));
    // A `BLANK` of another value names the image's null; the stored ones take it.
    h.set_internal(keywords::BLANK, -999);
    let image = decompress_image(&h, &table).unwrap();
    assert_eq!(image.scaling.blank, Some(-999));
    assert_eq!(image.samples, Data::I16(vec![10, -999]));

    // A `ZBLANK` column gives each tile's stored null.
    let tile = OneTile::new("NOCOMPRESS", 16, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &be(&[10i16, 20], i16::to_be_bytes))
        .fixed("ZBLANK", "1I", &20i16.to_be_bytes());
    let mut h = tile.header();
    assert!(matches!(
        decompress_image(&h, &tile.table()),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::BLANK)
        })
    ));
    h.set_internal(keywords::BLANK, -999);
    assert_eq!(
        decompress_image(&h, &tile.table()).unwrap().samples,
        Data::I16(vec![10, -999])
    );
}

/// A tile takes the first non-empty of its three cells, and reads a fallback only when
/// the cells before it are empty: a broken `GZIP_COMPRESSED_DATA` descriptor fails no
/// tile whose `COMPRESSED_DATA` holds it.
#[test]
fn a_fallback_cell_is_read_only_when_needed() {
    let tile = OneTile::new("NOCOMPRESS", 16, &[2, 1])
        .heap("COMPRESSED_DATA", 'B', &be(&[1i16, 2], i16::to_be_bytes))
        .heap("GZIP_COMPRESSED_DATA", 'B', &[0]);
    let h = tile.header();
    let mut data = tile.data();
    // The second descriptor's offset, past the heap.
    data[12..16].copy_from_slice(&1000i32.to_be_bytes());
    let table = Table::from_data(&h, data).unwrap();
    assert_eq!(
        decompress_image(&h, &table).unwrap().samples,
        Data::I16(vec![1, 2])
    );
}

/// A float tile stored as is keeps every bit, a signaling NaN's included: it decodes in
/// the image's stored type. Its raw bytes must be a whole tile.
#[test]
fn an_uncompressed_float_tile_keeps_its_bits() {
    let values = [f32::from_bits(0x7F80_0001), -0.0, f32::from_bits(1)];
    let raw = be(&values, f32::to_be_bytes);
    for cell in ['E', 'B'] {
        let tile = OneTile::new("GZIP_1", -32, &[3, 1])
            .heap("COMPRESSED_DATA", 'B', &[])
            .heap("UNCOMPRESSED_DATA", cell, &raw);
        assert_same_bits(
            &decompress_image(&tile.header(), &tile.table())
                .unwrap()
                .samples,
            &Data::F32(values.to_vec()),
            &format!("1P{cell}"),
        );
    }
    let mut long = raw;
    long.push(0);
    let tile = OneTile::new("GZIP_1", -32, &[3, 1])
        .heap("COMPRESSED_DATA", 'B', &[])
        .heap("UNCOMPRESSED_DATA", 'B', &long);
    assert!(matches!(
        decompress_image(&tile.header(), &tile.table()),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 12,
            got: 13
        })
    ));
}

#[test]
fn reading_a_plain_bintable_as_an_image_is_rejected() {
    // DDTSUVDATA hdu 1 is an ordinary BINTABLE (no ZIMAGE).
    let mut f = Fixture::Ddts.stream();
    // Public path: `image` sees a non-ZIMAGE bintable and rejects it as a
    // non-image (it never reaches the decompressor).
    assert!(matches!(
        f.image(1),
        Err(FitsError::HduKindMismatch {
            expected: ExpectedHdu::Image,
            got: HduKind::BinTable
        })
    ));
    // The decoder reads the geometry the same classification resolves.
    let table = f.read_table(1).unwrap();
    assert!(matches!(
        decompress_image(&f.hdus[1].header, &table),
        Err(FitsError::HduKindMismatch {
            expected: ExpectedHdu::Image,
            got: HduKind::BinTable
        })
    ));
}

#[test]
fn compressed_image_rejects_short_tiles() {
    let tile = OneTile::new("NOCOMPRESS", 16, &[2]).heap("COMPRESSED_DATA", 'B', &[0]);
    let (h, table) = (tile.header(), tile.table());
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 4,
            got: 1
        })
    ));

    let tile = OneTile::new("GZIP_1", 16, &[2])
        .heap("COMPRESSED_DATA", 'B', &[])
        .heap("UNCOMPRESSED_DATA", 'I', &1i16.to_be_bytes());
    let (h, table) = (tile.header(), tile.table());
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Samples,
            expected: 2,
            got: 1
        })
    ));
}

#[test]
fn decompress_image_rejects_overflowing_znaxis_product() {
    // ZNAXIS1·ZNAXIS2 = 5e9·5e9 = 2.5e19 overflows usize; decode must reject the
    // header up front (before allocating the output plane), not wrap to a small
    // buffer and then scatter out of bounds.
    let tile = OneTile::new("GZIP_1", 16, &[5_000_000_000, 5_000_000_000])
        .without_tiles()
        .heap("COMPRESSED_DATA", 'B', &[]);
    let (h, table) = (tile.header(), tile.table());
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataUnitOverflow)
    ));
}

#[test]
fn decompress_image_rejects_oversized_znaxis_product() {
    // ZNAXIS1 = 2^60 does NOT overflow usize (so the overflow guard passes), but
    // allocating that many bytes would abort the process. The output plane is
    // allocated fallibly (`try_reserve`), so decode must return a recoverable error.
    let tile =
        OneTile::new("GZIP_1", 8, &[1 << 60])
            .without_tiles()
            .heap("COMPRESSED_DATA", 'B', &[]);
    let (h, table) = (tile.header(), tile.table());
    assert!(matches!(
        decompress_image(&h, &table),
        Err(FitsError::DataUnitTooLarge { .. })
    ));
}

#[cfg(feature = "parallel")]
#[test]
fn parallel_decode_wave_budget_counts_slot_bytes() {
    // A wave's one buffer holds a slot of the largest tile for each: 1 byte for one u8
    // and 8 for one i64, so the 4 MiB budget holds 4194304 and 4194304 / 8 tiles.
    let single = TileGeometry::new(&[1, 4_194_304], [1, 1]);
    assert_eq!(decode_wave_tile_count::<u8>(&single), 4_194_304);
    assert_eq!(decode_wave_tile_count::<i64>(&single), 524_288);
    // One tile larger than the whole budget still makes a wave of one.
    let whole = TileGeometry::new(&[1, 4_194_304], [1, 4_194_304]);
    assert_eq!(decode_wave_tile_count::<u8>(&whole), 1);
}
