use crate::image_model::Image;
use crate::image_model::bitpix::*;
use crate::image_model::data::Data;
use crate::image_model::internals::physical;
use crate::internals::samples;
use std::assert_matches;

#[test]
fn codes_and_sizes_match_the_standard() {
    // (BITPIX code, byte size, is_float)
    let cases = [
        (Bitpix::U8, 8, 1, false),
        (Bitpix::I16, 16, 2, false),
        (Bitpix::I32, 32, 4, false),
        (Bitpix::I64, 64, 8, false),
        (Bitpix::F32, -32, 4, true),
        (Bitpix::F64, -64, 8, true),
    ];
    for (bp, code, size, is_float) in cases {
        assert_eq!(bp.code(), code);
        assert_eq!(Bitpix::try_from(code).unwrap(), bp);
        assert_eq!(bp.elem_size(), size);
        assert_eq!(bp.is_float(), is_float);
        assert_eq!(bp.is_integer(), !is_float);
    }
}

#[test]
fn codes_outside_the_allowed_set_are_refused() {
    for bad in [0, 7, 1, -1, 24, 128, -16] {
        assert_matches!(
            Bitpix::try_from(bad),
            Err(FitsError::InvalidBitpix { code }) if code == bad
        );
    }
}

/// The reader's view path: the stored bytes of a multi-byte type swapped into a `u64`
/// scratch read as a typed view equal to the owned decode. A byte image needs no swap.
#[test]
fn view_of_swapped_words_equals_the_decode() {
    let cases = [
        Data::I16(vec![1, -1, -32768, 32767]),
        Data::I32(vec![256, -1, i32::MIN]),
        Data::I64(vec![5, -5, i64::MAX]),
        Data::F32(vec![1.0, -2.5, 0.0]),
        Data::F64(vec![1.0, -2.5, f64::MAX]),
    ];
    let mut words: Vec<u64> = Vec::new();
    for data in &cases {
        let bytes = data.encode();
        data.bitpix().swap_into_words(&bytes, &mut words);
        assert_eq!(
            data.bitpix().view_words(&words, bytes.len()),
            Data::decode(&bytes, data.bitpix()).view(),
            "{:?}",
            data.bitpix()
        );
    }
}

/// Equal byte lengths of two types reuse one scratch, which the reader's page-fault-free
/// reads rely on: 4096 bytes are 2048 `i16` then 1024 `i32`.
#[test]
fn swapped_words_of_equal_length_reuse_the_scratch() {
    let i16s = Data::I16((0..2048).collect());
    let i32s = Data::I32((0..1024).map(|i| i - 7).collect());
    let mut words: Vec<u64> = Vec::new();
    Bitpix::I16.swap_into_words(&i16s.encode(), &mut words);
    let first = words.as_ptr();
    Bitpix::I32.swap_into_words(&i32s.encode(), &mut words);
    assert_eq!(words.as_ptr(), first);
    assert_eq!(
        Bitpix::I32.view_words(&words, i32s.encode().len()),
        i32s.view()
    );
}

/// The one pass from the stored bytes and the pass over host-endian samples are two
/// implementations of one map: they give the expected values, and agree bit for bit, in
/// both planes. `BLANK = -2` maps to NaN for each integer type, never for a float one.
#[test]
fn stored_byte_pass_gives_the_physical_plane_of_the_sample_pass() {
    let scaling = Scaling {
        bscale: 2.0,
        bzero: 10.0,
        blank: Some(-2),
    };
    let cases = [
        (Data::U8(vec![1, 2]), vec![12.0, 14.0]),
        (Data::I16(vec![-2, 3]), vec![f64::NAN, 16.0]),
        (Data::I32(vec![-2, 4]), vec![f64::NAN, 18.0]),
        (Data::I64(vec![-2, 5]), vec![f64::NAN, 20.0]),
        (Data::F32(vec![1.5, -2.0]), vec![13.0, 6.0]),
        (Data::F64(vec![2.25, -3.0]), vec![14.5, 4.0]),
    ];
    for (data, expected) in cases {
        let bitpix = data.bitpix();
        let bytes = data.encode();
        let mut from_bytes: Vec<f64> = Vec::new();
        bitpix.extend_physical_be(&bytes, &scaling, &mut from_bytes);
        assert_eq!(
            samples::nan_blind_of(&from_bytes),
            samples::nan_blind_of(&expected),
            "{bitpix:?}"
        );
        let mut from_bytes_f32: Vec<f32> = Vec::new();
        bitpix.extend_physical_be(&bytes, &scaling, &mut from_bytes_f32);
        let expected_f32: Vec<f32> = expected.iter().map(|&value| value as f32).collect();
        assert_eq!(
            samples::nan_blind_of(&from_bytes_f32),
            samples::nan_blind_of(&expected_f32),
            "{bitpix:?}"
        );
        let image = Image {
            shape: vec![data.len()],
            samples: data,
            scaling,
        };
        assert_eq!(
            samples::bits_of(&physical::<f64>(&image)),
            samples::bits_of(&from_bytes),
            "{bitpix:?}"
        );
        assert_eq!(
            samples::bits_of(&physical::<f32>(&image)),
            samples::bits_of(&from_bytes_f32),
            "{bitpix:?}"
        );
    }
}

/// Cells of 1 sample, which take their own step, and of 3, 600 samples in each case, cross
/// the blocks of 256 the gathered pass scales in: the values equal those of the cells
/// scaled one at a time, bit for bit, in both planes.
#[test]
fn the_cells_pass_gives_the_cell_by_cell_physical_values() {
    let scaling = Scaling {
        bscale: 2.0,
        bzero: 10.0,
        blank: Some(-2),
    };
    let samples: Vec<i16> = (0..600).map(|i| (i % 7) as i16 - 3).collect();
    let bytes = Data::I16(samples).encode();
    for per_cell in [1, 3] {
        let cells = || bytes.chunks(per_cell * 2);
        let mut whole: Vec<f64> = Vec::new();
        Bitpix::I16.extend_physical_be_cells(cells(), &scaling, &mut whole);
        let mut each: Vec<f64> = Vec::new();
        for cell in cells() {
            Bitpix::I16.extend_physical_be(cell, &scaling, &mut each);
        }
        assert_eq!(
            samples::bits_of(&whole),
            samples::bits_of(&each),
            "{per_cell}"
        );
        let mut whole: Vec<f32> = Vec::new();
        Bitpix::I16.extend_physical_be_cells(cells(), &scaling, &mut whole);
        let mut each: Vec<f32> = Vec::new();
        for cell in cells() {
            Bitpix::I16.extend_physical_be(cell, &scaling, &mut each);
        }
        assert_eq!(
            samples::bits_of(&whole),
            samples::bits_of(&each),
            "{per_cell}"
        );
    }
}
