//! Images written and read back: every type, scaling and the unsigned conventions.

use crate::fits_writer::tests::*;

#[test]
fn writes_a_multi_hdu_image_file() {
    let primary = Image::new(vec![2, 2], Data::U8(vec![1, 2, 3, 4]), Scaling::IDENTITY).unwrap();
    let ext = Image::new(vec![3], Data::I16(vec![10, 20, 30]), Scaling::IDENTITY).unwrap();
    let mut w = memory::memory_writer();
    w.write_image(primary.view(), None).unwrap();
    let header_ptr = w.header_scratch.as_ptr();
    let header_capacity = w.header_scratch.capacity();
    w.write_image(ext.view(), None).unwrap(); // second image ⇒ IMAGE extension
    assert_eq!(w.header_scratch.as_ptr(), header_ptr);
    assert_eq!(w.header_scratch.capacity(), header_capacity);
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus().len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[1].kind, HduKind::Image);
    assert_eq!(
        memory::read_whole(&mut r, 0).samples,
        Data::U8(vec![1, 2, 3, 4])
    );
    assert_eq!(
        memory::read_whole(&mut r, 1).samples,
        Data::I16(vec![10, 20, 30])
    );
}

/// A scaling that is not finite names the first keyword at fault, in a binary column,
/// fixed or variable-length, an ASCII column, integer or float, an image, and a
/// tiled-compressed image.
#[test]
fn nonfinite_scaling_is_refused_before_output() {
    for [scale, zero] in NONFINITE {
        let at_fault = |[scale_keyword, zero_keyword]: [&'static str; 2]| {
            if scale.is_finite() {
                zero_keyword
            } else {
                scale_keyword
            }
        };
        for column in fixed_and_vla(ColumnData::I32(vec![0])) {
            assert_table_column_refused(
                column.with_scaling(scale, zero),
                at_fault(["TSCAL1", "TZERO1"]),
            );
        }
        let format = AsciiFloatFormat::F {
            width: 8,
            decimals: 1,
        };
        for column in [
            AsciiWriteColumn::integer("A", vec![Some(1)], 8),
            AsciiWriteColumn::float("A", vec![Some(1.0)], format),
        ] {
            let columns = [column.with_scaling(scale, zero)];
            assert_matches!(
                memory::refused_before_output(
                    |w| w.write_ascii_table(&memory::ascii_table(1, &columns), None)
                ),
                FitsError::InvalidKeywordValue { keyword, .. }
                    if keyword == at_fault(["TSCAL1", "TZERO1"])
            );
        }
        // A literal: `Image::new` refuses this image, and the write's own check is
        // under test.
        let image = Image {
            shape: vec![1],
            samples: Data::U8(vec![0]),
            scaling: samples::linear(scale, zero),
        };
        assert_matches!(
            memory::refused_before_output(|w| w.write_image(image.view(), None)),
            FitsError::InvalidKeywordValue { keyword, .. }
                if keyword == at_fault(["BSCALE", "BZERO"])
        );
        #[cfg(feature = "compression")]
        assert_matches!(
            memory::refused_before_output(|w| w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::GZIP),
                None
            )),
            FitsError::InvalidKeywordValue { keyword, .. }
                if keyword == at_fault(["BSCALE", "BZERO"])
        );
    }
}

#[test]
fn image_round_trips_through_write_image_and_a_whole_read() {
    let image = Image::new(
        vec![2, 3],
        Data::I16(vec![1, -2, 3, -4, 5, -6]),
        Scaling::IDENTITY,
    )
    .unwrap();
    let bytes = memory::written(|w| w.write_image(image.view(), None));
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE); // one header block + one data block

    let mut r = bytes.stream();
    assert_eq!(r.hdus().len(), 1);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    let back = memory::read_whole(&mut r, 0);
    assert_eq!(back.shape, vec![2, 3]);
    assert_eq!(back.samples, Data::I16(vec![1, -2, 3, -4, 5, -6]));
}

/// Each unsigned constructor writes its offset as an exact `BZERO` at unit scale, and its
/// extremes read back exactly through the unsigned view: `−2⁷` for `i8` in `BITPIX = 8`,
/// and `2¹⁵`, `2³¹`, `2⁶³` for `u16`, `u32`, `u64`. `2⁶³` is written as the integer
/// `9223372036854775808`, which a real's 17 significant digits would not spell.
#[test]
fn unsigned_images_write_their_exact_offset_and_read_back() {
    for (image, bitpix, offset, expected) in [
        (
            Image::from_i8(vec![3], &[i8::MIN, 0, i8::MAX]).unwrap(),
            8,
            Value::Real(-128.0),
            UnsignedData::I8(vec![i8::MIN, 0, i8::MAX]),
        ),
        (
            Image::from_u16(vec![3], &[0, 32768, u16::MAX]).unwrap(),
            16,
            Value::Real(32768.0),
            UnsignedData::U16(vec![0, 32768, u16::MAX]),
        ),
        (
            Image::from_u32(vec![3], &[0, 1 << 31, u32::MAX]).unwrap(),
            32,
            Value::Real(2_147_483_648.0),
            UnsignedData::U32(vec![0, 1 << 31, u32::MAX]),
        ),
        (
            Image::from_u64(vec![2], &[u64::MIN, u64::MAX]).unwrap(),
            64,
            Value::Integer(FitsInteger::from(1u64 << 63)),
            UnsignedData::U64(vec![u64::MIN, u64::MAX]),
        ),
    ] {
        let file = memory::written(|w| w.write_image(image.view(), None));
        let mut reader = file.stream();
        cards::assert_cards(
            &reader.hdus[0].header,
            &[
                ("BITPIX", bitpix.into()),
                ("BSCALE", Value::Real(1.0)),
                ("BZERO", offset),
            ],
        );
        assert_eq!(
            memory::read_whole(&mut reader, 0).view().unsigned(),
            Some(expected)
        );
    }
}

#[test]
fn written_file_reads_back_with_matching_boundaries() {
    let primary = cards::from_card_lines(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    1",
        "NAXIS1  =                   10",
    ]);
    let bytes = memory::written(|w| w.write_raw_hdu(&primary, &[0u8; 10]));

    // Header block + one padded data block.
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE);

    let f = bytes.stream();
    assert_eq!(f.hdus().len(), 1);
    assert_eq!(f.hdus[0].data_offset, BLOCK_SIZE as u64);
    assert_eq!(padded_len(f.hdus[0].data_bytes).unwrap(), BLOCK_SIZE as u64);
    assert_eq!(f.hdus[0].header.axes().unwrap(), vec![10]);

    // An extension written first follows an empty primary: one header block, then the
    // extension's header block and its one data block.
    let extension = cards::from_card_lines(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  =                    8",
        "NAXIS   =                    1",
        "NAXIS1  =                   10",
        "PCOUNT  =                    0",
        "GCOUNT  =                    1",
    ]);
    let bytes = memory::written(|w| w.write_raw_hdu(&extension, &[7u8; 10]));
    assert_eq!(bytes.len(), 3 * BLOCK_SIZE);
    let mut f = bytes.slice();
    assert_eq!(f.hdus().len(), 2);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);
    assert_eq!(f.hdus[0].data_bytes, 0);
    assert_eq!(
        f.hdus[0].header.get_logical(keywords::EXTEND).unwrap(),
        Some(true)
    );
    assert_eq!(f.hdus[1].data_offset, 2 * BLOCK_SIZE as u64);
    assert_eq!(memory::read_whole(&mut f, 1).samples, Data::U8(vec![7; 10]));
}

#[cfg(feature = "compression")]
#[test]
fn compressed_image_metadata_is_validated_before_automatic_primary() {
    // A literal: `Image::new` refuses this image, and the write's own check is
    // under test.
    let image = Image {
        shape: vec![1],
        samples: Data::F32(vec![0.0]),
        scaling: Scaling {
            blank: Some(0),
            ..Scaling::IDENTITY
        },
    };
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            None
        )),
        FitsError::InvalidKeywordValue {
            keyword: keywords::BLANK,
            ..
        }
    );

    let image = Image::new(vec![1, 1], Data::I64(vec![i64::MIN]), Scaling::IDENTITY).unwrap();
    let bytes = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice),
            None,
        )
    });
    let mut reader = bytes.slice();
    assert_eq!(
        memory::read_whole(&mut reader, 1).samples,
        Data::I64(vec![i64::MIN])
    );

    // Only a 1×1 tile carries `i64::MIN` through HCOMPRESS, as any larger one takes its
    // difference with a neighbour; a tile with no two extents above 1 has no HCOMPRESS
    // plane, so it is refused.
    assert_matches!(
        memory::refused_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_tiles([1, 1]),
            None
        )),
        FitsError::UnsupportedCompression {
            feature: CompressionFeature::HcompressDimensions
        }
    );
}
