//! The reads of an image: whole, by section, as a view into the caller's words, and
//! their judgement of blanks.

use crate::reader::tests::*;

/// Every image path reads `BLANK` one way: a float image ignores it and reads, an
/// integer `BLANK` its type cannot hold fails the metadata, the view and the read
/// alike. 40000 is no `i16`.
#[test]
fn every_image_path_judges_blank_alike() {
    let float = memory::written(|w| {
        w.write_raw_hdu(
            &cards::from_card_lines(&[
                "SIMPLE  = T",
                "BITPIX  = -32",
                "NAXIS   = 1",
                "NAXIS1  = 1",
                "BLANK   = 7",
            ]),
            &1.5f32.to_be_bytes(),
        )
    });
    let mut r = float.slice();
    assert_eq!(r.hdus()[0].image_metadata().unwrap().scaling.blank, None);
    let mut words = Vec::new();
    assert_eq!(
        r.image(0)
            .unwrap()
            .view(Region::All, &mut words)
            .unwrap()
            .samples(),
        View::F32(&[1.5])
    );
    assert_eq!(
        r.image(0).unwrap().read(Region::All).unwrap().samples,
        Data::F32(vec![1.5])
    );

    let integer = memory::written(|w| {
        w.write_raw_hdu(
            &cards::from_card_lines(&[
                "SIMPLE  = T",
                "BITPIX  = 16",
                "NAXIS   = 1",
                "NAXIS1  = 1",
                "BLANK   = 40000",
            ]),
            &3i16.to_be_bytes(),
        )
    });
    let mut r = integer.slice();
    let blank = |result: Result<()>| matches!(result, Err(FitsError::InvalidKeywordValue { keyword, .. }) if keyword == "BLANK");
    assert!(blank(r.hdus()[0].image_metadata().map(|_| ())));
    assert!(blank(r.image(0).map(|_| ())));
}

#[test]
fn image_reads_the_primary_array_shape_and_type() {
    let mut f = Fixture::Uit.stream();
    let mut image = f.image(0).unwrap();
    assert_eq!(image.metadata().shape, [512, 512]);
    assert_eq!(image.metadata().bitpix, Bitpix::I16);
    // The sum, extremes and centre sample of the 262144 stored values; BSCALE is
    // 2.0587209E-16 and BZERO 0.
    let mut physical: Vec<f64> = Vec::new();
    image.physical_into(Region::All, &mut physical).unwrap();
    assert_eq!(physical.len(), 512 * 512);
    let Data::I16(samples) = image.read(Region::All).unwrap().samples else {
        panic!("expected I16 samples");
    };
    assert_eq!(
        samples.iter().map(|&v| i64::from(v)).sum::<i64>(),
        1_050_151
    );
    assert_eq!(samples.iter().min(), Some(&-9));
    assert_eq!(samples.iter().max(), Some(&2049));
    let centre = 256 * 512 + 256;
    assert_eq!(samples[centre], 2);
    assert_eq!(physical[centre], 2.0 * 2.058_720_9e-16);
}

#[test]
fn image_samples_match_a_manual_big_endian_decode() {
    let mut f = Fixture::Uit.stream();
    // Independently decode every pixel straight from the data bytes.
    let unit = f.read_data_unit(0).unwrap();
    let manual: Vec<i16> = unit
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_be_bytes(*c))
        .collect();
    assert_eq!(memory::read_whole(&mut f, 0).samples, Data::I16(manual));
}

#[test]
fn images_lists_readable_images_including_compressed() {
    let images = |fixture: Fixture| {
        fixture
            .stream()
            .images()
            .map(|hdu| hdu.index)
            .collect::<Vec<_>>()
    };
    // A single primary array image.
    assert_eq!(images(Fixture::Uit), [0]);
    // Empty primary + a tiled-compressed image extension (classified by ZIMAGE),
    // so only HDU 1 is an image — the `NAXIS = 0` primary is skipped.
    assert_eq!(images(Fixture::CompGzipI16), [1]);
    // Random-groups primary + plain bintable: no images at all.
    assert!(images(Fixture::Ddts).is_empty());

    // `NAXIS = 0` holds no array in a primary and in an image extension alike.
    let mut bytes = cards::fits_file(
        &["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 0", "EXTEND  = T"],
        &[],
    );
    let extensions: [(&[&str], &[u8]); 2] = [
        (&["NAXIS   = 0"], &[]),
        (&["NAXIS   = 1", "NAXIS1  = 2"], &[1, 2]),
    ];
    for (naxis, data) in extensions {
        let cards = [
            &["XTENSION= 'IMAGE   '", "BITPIX  = 8"][..],
            naxis,
            &["PCOUNT  = 0", "GCOUNT  = 1"],
        ]
        .concat();
        bytes.extend(cards::fits_file(&cards, data));
    }
    let reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    let images: Vec<bool> = reader.hdus().iter().map(Hdu::is_image).collect();
    assert_eq!(images, [false, false, true]);
    assert_eq!(
        reader.images().map(|hdu| hdu.index).collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn u8_view_borrows_the_source_with_zero_copy() {
    let image = Image::new(vec![4], Data::U8(vec![10, 20, 30, 40]), Scaling::IDENTITY).unwrap();
    let buf = memory::written(|w| w.write_image(image.view(), None));

    let mut reader = buf.slice();
    let mut image = reader.image(0).unwrap();
    assert_eq!(image.metadata().shape, [4]);
    assert_eq!(image.metadata().bitpix, Bitpix::U8);
    // U8 needs no byte-swap, so the view borrows the stored bytes in the source: its
    // address lies within `buf`, and the caller's words stay untouched.
    let mut scratch = Vec::new();
    let view = image.view(Region::All, &mut scratch).unwrap();
    assert_eq!(view.shape(), [4]);
    let v = view.samples().as_u8().expect("a U8 image views as U8");
    assert_eq!(v, &[10, 20, 30, 40]);
    let base = buf.as_ptr() as usize;
    assert!(
        (base..base + buf.len()).contains(&(v.as_ptr() as usize)),
        "the u8 view must point inside the source buffer (zero-copy)"
    );
    assert!(scratch.is_empty(), "a U8 view must not touch the scratch");
}

#[test]
fn multibyte_view_swaps_into_the_callers_words() {
    let image = Image::new(vec![3], Data::I16(vec![1, -2, 300]), Scaling::IDENTITY).unwrap();
    let buf = memory::written(|w| w.write_image(image.view(), None));

    // The stored bytes are big-endian: 1 → 0x0001, -2 → 0xFFFE, 300 → 0x012C.
    let mut reader = buf.slice();
    assert_eq!(reader.read_data_unit(0).unwrap(), [0, 1, 255, 254, 1, 44]);
    // A type that needs byte-swapping views from the caller's words, not the source.
    let mut image = reader.image(0).unwrap();
    let mut words = Vec::new();
    let view = image.view(Region::All, &mut words).unwrap();
    let v = view.samples().as_i16().expect("an I16 image views as I16");
    assert_eq!(v, [1, -2, 300]);
    assert_eq!(v.as_ptr(), words.as_ptr().cast::<i16>());
}

/// A whole view equals the owned read of a plain and of a compressed image, on either
/// source: the same shape, scaling and samples, decoded into the caller's scratch.
#[test]
fn whole_view_matches_the_owned_read_on_either_source() {
    fn check<S: Source>(reader: &mut FitsReader<S>, hdu: usize, shape: [usize; 2]) {
        let owned = memory::read_whole(reader, hdu);
        let mut scratch: Vec<u64> = Vec::with_capacity(owned.samples.len().div_ceil(4));
        let scratch_ptr = scratch.as_ptr().cast::<i16>();
        let mut image = reader.image(hdu).unwrap();
        let view = image.view(Region::All, &mut scratch).unwrap();
        assert_eq!(view.shape(), shape);
        assert_eq!(view.scaling(), owned.scaling);
        let (View::I16(v), Data::I16(o)) = (view.samples(), &owned.samples) else {
            panic!("expected an I16 view and decode");
        };
        assert_eq!(v, o.as_slice());
        assert_eq!(v.as_ptr(), scratch_ptr);
    }
    let compressed = cfg!(feature = "compression").then_some((Fixture::CompGzipI16, 1, [24, 16]));
    for (fixture, hdu, shape) in iter::once((Fixture::Uit, 0, [512, 512])).chain(compressed) {
        check(&mut fixture.stream(), hdu, shape);
        check(&mut fixture.slice(), hdu, shape);
    }
}

#[test]
fn image_sections_match_hand_computed_values_for_every_bitpix() {
    // x + 5y + 20z over x in 1..4, y in 1..3, z in 1..3: 26, 27, 28, 31, … 53.
    let idxs = samples::section_indices(&[5, 4, 3], &[1..4, 1..3, 1..3]);
    assert_eq!(idxs, [26, 27, 28, 31, 32, 33, 46, 47, 48, 51, 52, 53]);
    for bitpix in BITPIXES {
        let samples = samples::ramp(bitpix, 60);
        let expected = samples::gather(&samples, &idxs);
        let image = Image::new(vec![5, 4, 3], samples, Scaling::IDENTITY).unwrap();
        let bytes = memory::written(|w| w.write_image(image.view(), None));
        let mut reader = bytes.slice();
        let mut scratch = Vec::new();
        let mut image = reader.image(0).unwrap();
        let section = image.view(&[1..4, 1..3, 1..3], &mut scratch).unwrap();
        assert_eq!(section.shape, [3, 2, 2], "{bitpix:?}");
        assert_eq!(section.scaling, Scaling::IDENTITY, "{bitpix:?}");
        samples::assert_same_bits(
            &section.samples.to_data(),
            &expected,
            &format!("{bitpix:?}"),
        );
    }
}

#[test]
fn image_sections_preserve_scaling_and_validate_empty_and_invalid_regions() {
    let image = Image::new(
        vec![4, 3],
        vec![1i16, 2, -99, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank: Some(-99),
        },
    )
    .unwrap();
    let bytes = memory::written(|w| w.write_image(image.view(), None));
    let mut reader = bytes.slice();
    let mut handle = reader.image(0).unwrap();
    let section = handle.read(&[1..4, 0..2]).unwrap();
    assert_eq!(section.shape, [3, 2]);
    assert_eq!(section.scaling, image.scaling);
    // Stored 2, -99 (BLANK), 4, 6, 7, 8: 10 + 2·2 = 14, NaN, 18, 22, 24, 26 — the owned
    // read's view and the handle's one pass from the stored bytes agree, appended
    // after what `out` already held.
    let expected = [14.0, f64::NAN, 18.0, 22.0, 24.0, 26.0];
    let mut from_view: Vec<f64> = Vec::new();
    section.view().physical_into(&mut from_view);
    let mut from_bytes = vec![-1.0];
    handle
        .physical_into(&[1..4, 0..2], &mut from_bytes)
        .unwrap();
    for physical in [&*from_view, &from_bytes[1..]] {
        assert_eq!(physical.len(), expected.len());
        for (got, want) in physical.iter().zip(expected) {
            assert!(
                got == &want || got.is_nan() && want.is_nan(),
                "{physical:?}"
            );
        }
    }
    assert_eq!(from_bytes[0], -1.0);

    let empty = handle.read(&[2..2, 1..3]).unwrap();
    assert_eq!(empty.shape, [0, 2]);
    assert!(empty.samples.is_empty());
    let wrong_rank = 0..1;
    assert_matches!(
        handle.read(slice::from_ref(&wrong_rank)),
        Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: 2,
            got: 1,
        })
    );
    assert_matches!(
        handle.read(&[0..5, 0..1]),
        Err(FitsError::ImageRegionOutOfBounds {
            axis: 0,
            start: 0,
            end: 5,
            len: 4,
        })
    );
}

#[test]
fn plain_image_section_streams_exact_strided_runs() {
    let shape = [6, 5, 4, 3];
    let samples: Vec<i16> = (0..shape.iter().product::<usize>())
        .map(|index| i16::try_from(index).unwrap() - 100)
        .collect();
    let image = Image::new(shape.to_vec(), samples.clone(), Scaling::IDENTITY).unwrap();
    let bytes = memory::written(|w| w.write_image(image.view(), None));
    let counting = Counting::new(&bytes);
    let fetches = counting.fetches();
    let mut stream = FitsReader::open(counting).unwrap();
    let data_offset = stream.hdus[0].data_offset as usize;
    fetches.borrow_mut().ranges.clear();
    let ranges = [1..3, 1..4, 0..4, 1..3];
    let section = stream.image(0).unwrap().read(&ranges).unwrap();

    let mut expected_values = Vec::new();
    let mut expected_reads = Vec::new();
    for axis3 in 1..3 {
        for axis2 in 0..4 {
            for axis1 in 1..4 {
                let element = 1 + 6 * axis1 + 30 * axis2 + 120 * axis3;
                expected_values.extend_from_slice(&samples[element..element + 2]);
                let start = data_offset + element * size_of::<i16>();
                expected_reads.push(start..start + 2 * size_of::<i16>());
            }
        }
    }
    assert_eq!(section.shape, [2, 3, 4, 2]);
    assert_eq!(section.samples, Data::I16(expected_values.clone()));
    assert_eq!(fetches.borrow().ranges, expected_reads);

    let mut slice = bytes.slice();
    assert_eq!(
        slice.image(0).unwrap().read(&ranges).unwrap().samples,
        Data::I16(expected_values)
    );
}

/// A 9×7 ramp of `bitpix`, scaled, in GZIP tiles of 4×3: 3×3 tiles, the last row and
/// column of them partial.
#[cfg(feature = "compression")]
#[derive(Debug)]
struct TiledRamp {
    bytes: memory::Written,
    scaling: Scaling,
}

#[cfg(feature = "compression")]
impl TiledRamp {
    /// The section every test reads: columns 2..9 and rows 1..7, which cross each tile
    /// boundary.
    const SECTION: [Range<usize>; 2] = [2..9, 1..7];

    /// Sample 13, at (4, 1), which is (2, 0) in the section, is the integer types'
    /// `BLANK`, and a NaN with a payload in the floats. No other sample holds the blank.
    fn new(bitpix: Bitpix) -> Self {
        let mut samples = samples::ramp(bitpix, 9 * 7);
        let blank = match &mut samples {
            Data::U8(values) => Some(i64::from(values[13])),
            Data::I16(values) => Some(i64::from(values[13])),
            Data::I32(values) => Some(i64::from(values[13])),
            Data::I64(values) => Some(values[13]),
            Data::F32(values) => {
                values[13] = f32::from_bits(0x7FC0_1234);
                None
            }
            Data::F64(values) => {
                values[13] = f64::from_bits(0x7FF8_0000_0000_1234);
                None
            }
        };
        let scaling = Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank,
        };
        let image = Image::new(vec![9, 7], samples, scaling).unwrap();
        let bytes = memory::written(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::GZIP).with_tiles([4, 3]),
                None,
            )
        });
        Self { bytes, scaling }
    }
}

/// The metadata is the encoded image's geometry, not the container's two-axis BINTABLE,
/// whose rows are the tiles.
#[cfg(feature = "compression")]
#[test]
fn tiled_image_reports_the_encoded_geometry() {
    for bitpix in BITPIXES {
        let ramp = TiledRamp::new(bitpix);
        let reader = ramp.bytes.slice();
        let metadata = reader.hdus()[1].image_metadata().unwrap();
        assert_eq!(
            (metadata.shape, metadata.bitpix, metadata.scaling),
            (&[9, 7][..], bitpix, ramp.scaling)
        );
        assert_eq!(
            reader.hdus()[1].container_schema().unwrap().row_count(),
            3 * 3
        );
    }
}

/// An owned read and a view of the section give the samples of the whole image, bit for
/// bit, and an empty section no sample.
#[cfg(feature = "compression")]
#[test]
fn section_across_tiles_reads_as_the_whole_image() {
    for bitpix in BITPIXES {
        let ramp = TiledRamp::new(bitpix);
        let mut reader = ramp.bytes.slice();
        let whole = memory::read_whole(&mut reader, 1).samples;
        let expected = samples::gather(
            &whole,
            &samples::section_indices(&[9, 7], &TiledRamp::SECTION),
        );
        let mut image = reader.image(1).unwrap();
        let owned = image.read(&TiledRamp::SECTION).unwrap();
        assert_eq!(owned.shape, [7, 6], "{bitpix:?}");
        assert_eq!(owned.scaling, ramp.scaling, "{bitpix:?}");
        samples::assert_same_bits(&owned.samples, &expected, &format!("{bitpix:?} owned"));

        let mut words = Vec::new();
        let view = image.view(&TiledRamp::SECTION, &mut words).unwrap();
        assert_eq!(view.shape, [7, 6], "{bitpix:?}");
        assert_eq!(view.scaling, ramp.scaling, "{bitpix:?}");
        samples::assert_same_bits(
            &view.samples.to_data(),
            &expected,
            &format!("{bitpix:?} view"),
        );

        let empty = image.read(&[2..2, 1..7]).unwrap();
        assert_eq!(empty.shape, [0, 6], "{bitpix:?}");
        assert!(empty.samples.is_empty(), "{bitpix:?}");
    }
}

/// The blank is the one NaN of the section's physical values. The handle's physical read
/// decodes the same tiles, so it gives the owned read's values bit for bit.
#[cfg(feature = "compression")]
#[test]
fn physical_section_read_maps_the_blank_alone_to_nan() {
    for bitpix in BITPIXES {
        let ramp = TiledRamp::new(bitpix);
        let mut reader = ramp.bytes.slice();
        let mut image = reader.image(1).unwrap();
        let mut physical: Vec<f64> = Vec::new();
        image
            .read(&TiledRamp::SECTION)
            .unwrap()
            .view()
            .physical_into(&mut physical);
        assert!(physical[2].is_nan(), "{bitpix:?}");
        assert_eq!(physical.iter().filter(|value| value.is_nan()).count(), 1);
        let mut from_handle: Vec<f64> = Vec::new();
        image
            .physical_into(&TiledRamp::SECTION, &mut from_handle)
            .unwrap();
        assert_eq!(
            samples::bits_of(&from_handle),
            samples::bits_of(&physical),
            "{bitpix:?}"
        );
    }
}

/// The whole image in `f32`, each tile converted as it decodes, lands after what the
/// buffer holds, as the owned read's values narrowed.
#[cfg(feature = "compression")]
#[test]
fn whole_physical_read_lands_after_what_the_buffer_holds() {
    for bitpix in BITPIXES {
        let ramp = TiledRamp::new(bitpix);
        let mut reader = ramp.bytes.slice();
        let whole = memory::read_whole(&mut reader, 1);
        let mut narrow: Vec<f32> = vec![7.5];
        reader
            .image(1)
            .unwrap()
            .physical_into(Region::All, &mut narrow)
            .unwrap();
        let mut expected: Vec<f32> = vec![7.5];
        whole.view().physical_into(&mut expected);
        assert_eq!(narrow.len(), 1 + 9 * 7, "{bitpix:?}");
        assert_eq!(
            samples::bits_of(&narrow),
            samples::bits_of(&expected),
            "{bitpix:?}"
        );
    }
}

/// Opening the image reads its container's main table, a row a tile. A one-tile corner
/// then fetches the heap bytes its tile's row addresses — what a one-row table selection
/// holds past its row. The whole image fetches the whole heap, which the writer packs
/// after the rows without gaps: with the rows, the data unit, once.
#[cfg(feature = "compression")]
#[test]
fn section_fetches_only_the_heap_of_its_tiles() {
    let ramp = TiledRamp::new(Bitpix::U8);
    let fetched = [[0..2, 0..2], [0..9, 0..7]].map(|ranges| {
        let mut counted = CountedReader::open(&ramp.bytes);
        let mut scratch = Vec::new();
        counted
            .reader
            .image(1)
            .unwrap()
            .view(&ranges, &mut scratch)
            .unwrap();
        counted.fetched()
    });
    let mut reader = ramp.bytes.slice();
    let first_tile = reader
        .read_container_rows(1, 0..1)
        .unwrap()
        .schema
        .extent
        .heap_end;
    let data_bytes = reader.hdus()[1].data_bytes as usize;
    let extent = reader.hdus()[1].container_schema().unwrap().extent;
    let main = extent.row_count * extent.row_len;
    assert_eq!(fetched, [main + first_tile - extent.row_len, data_bytes]);
}

/// The reader keeps the tile index of each compressed image it opens: two images, the
/// second opened between two handles on the first, fetch each index once — the main
/// table, NAXIS1 × NAXIS2 bytes of each container — and a third handle on either fetches
/// nothing.
#[cfg(feature = "compression")]
#[test]
fn reader_keeps_the_tile_index_of_each_compressed_image() {
    let image = Image::new(
        vec![9, 7],
        Data::I16((0..63).map(|value| value * 97 - 3000).collect()),
        Scaling::IDENTITY,
    )
    .unwrap();
    let bytes = memory::written(|w| {
        for tiles in [[9, 1], [3, 7]] {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::GZIP).with_tiles(tiles),
                None,
            )?;
        }
        Ok(())
    });
    let mut counted = CountedReader::open(&bytes);
    let main = |hdu: &Hdu| {
        let axis = |name| usize::try_from(hdu.header.get_integer(name).unwrap().unwrap()).unwrap();
        axis("NAXIS1") * axis("NAXIS2")
    };
    let (first, second) = (
        main(&counted.reader.hdus()[1]),
        main(&counted.reader.hdus()[2]),
    );
    for (hdu, fetched) in [
        (1, first),
        (2, first + second),
        (1, first + second),
        (2, first + second),
    ] {
        counted.reader.image(hdu).unwrap();
        assert_eq!(counted.fetched(), fetched, "HDU {hdu}");
    }
}

/// `finish_bytes` bounds what the finish adds to the reader: for a stream, its staging
/// buffer grown past the one block the header scan left in it to the padded unit, which
/// is under the 1 MiB a finish reads at a time; nothing for a resident source, which the
/// finish borrows. `tiles` gives a compressed image's `ZTILEn`, and none of a plain image.
#[cfg(feature = "compression")]
#[test]
fn finish_bytes_counts_a_streams_staging_and_tiles_give_the_tiling() {
    let image = Image::new(
        vec![100, 100],
        Data::I16((0..10_000).map(|value| (value % 3001) as i16).collect()),
        Scaling::IDENTITY,
    )
    .unwrap();
    let bytes = memory::written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::None).with_tiles([100, 10]),
            None,
        )
    });
    let mut stream = bytes.stream();
    let padded = stream.hdus()[1].padded_data_bytes();
    assert!(padded > 2880, "a unit past the scan's one block");
    let handle = stream.image(1).unwrap();
    assert_eq!(handle.finish_bytes(), padded);
    assert_eq!(handle.tiles(), Some(&[100, 10][..]));
    let mut resident = bytes.slice();
    assert_eq!(resident.image(1).unwrap().finish_bytes(), 0);

    let plain = memory::written(|w| w.write_image(image.view(), None));
    let mut plain = plain.slice();
    assert_eq!(plain.image(0).unwrap().tiles(), None);
}

/// The heap ends with the last tile, whose gzip trailer this breaks: a section in the
/// first tile still reads, and the reads that touch the last tile fail.
#[cfg(feature = "compression")]
#[test]
fn corrupt_tile_fails_only_the_reads_that_touch_it() {
    let mut ramp = TiledRamp::new(Bitpix::I16);
    let data_end = {
        let reader = ramp.bytes.slice();
        let hdu = &reader.hdus()[1];
        usize::try_from(hdu.data_offset + hdu.data_bytes).unwrap()
    };
    ramp.bytes[data_end - 1] ^= 0xFF;
    let mut reader = ramp.bytes.slice();
    let mut image = reader.image(1).unwrap();
    assert_eq!(
        image.read(&[0..4, 0..3]).unwrap().samples,
        samples::gather(
            &samples::ramp(Bitpix::I16, 9 * 7),
            &samples::section_indices(&[9, 7], &[0..4, 0..3])
        )
    );
    for ranges in [[8..9, 6..7], [0..9, 0..7]] {
        assert_matches!(
            image.read(&ranges),
            Err(FitsError::CorruptCompressedData { .. }),
            "{ranges:?}"
        );
    }
}
