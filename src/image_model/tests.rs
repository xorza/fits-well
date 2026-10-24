use crate::error::NumericTarget;
use crate::header_model::record_name::RecordName;
use crate::header_model::{Header, keywords};
use crate::image_model::internals::physical;
use crate::image_model::sample::Sample;
use crate::image_model::sample_type::UnsignedKind;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::View;
use crate::image_model::*;
use crate::internals::cards::from_card_lines as header;

fn image(samples: Data, scaling: Scaling) -> Image {
    Image {
        shape: vec![samples.len()],
        samples,
        scaling,
    }
}

/// Encode to a fresh buffer — the allocating shorthand over `encode_into` the
/// round-trip tests read against.
fn encoded(data: &Data) -> Vec<u8> {
    let mut out = Vec::new();
    data.view().encode_into(&mut out);
    out
}

#[test]
fn shape_product_handles_empty_axes_and_rejects_overflow() {
    assert_eq!(shape_product(&[]).unwrap(), 0);
    assert_eq!(shape_product(&[usize::MAX, 0, usize::MAX]).unwrap(), 0);
    assert!(matches!(
        shape_product(&[usize::MAX, 2]),
        Err(FitsError::DataUnitOverflow)
    ));
}

#[test]
fn image_constructor_enforces_geometry_for_every_bitpix() {
    let scaling = Scaling::IDENTITY;
    let valid = [
        Data::U8(vec![1, 2]),
        Data::I16(vec![1, 2]),
        Data::I32(vec![1, 2]),
        Data::I64(vec![1, 2]),
        Data::F32(vec![1.0, 2.0]),
        Data::F64(vec![1.0, 2.0]),
    ];
    for samples in valid {
        let bitpix = samples.bitpix();
        let image = Image::new(vec![2], samples, scaling).unwrap();
        assert_eq!(image.view().metadata().shape, [2]);
        assert_eq!(image.view().metadata().bitpix, bitpix);
    }

    let short = [
        Data::U8(vec![1]),
        Data::I16(vec![1]),
        Data::I32(vec![1]),
        Data::I64(vec![1]),
        Data::F32(vec![1.0]),
        Data::F64(vec![1.0]),
    ];
    for samples in short {
        assert!(matches!(
            Image::new(vec![2], samples, scaling),
            Err(FitsError::DataSizeMismatch {
                unit: SizeUnit::Samples,
                expected: 2,
                got: 1
            })
        ));
    }
}

#[test]
fn simple_image_constructor_accepts_typed_vectors_with_identity_scaling() {
    let image = Image::new(vec![2, 2], vec![1i16, 2, 3, 4], Scaling::IDENTITY).unwrap();
    assert_eq!(image.view().metadata().shape, [2, 2]);
    assert_eq!(image.view().metadata().bitpix, Bitpix::I16);
    assert_eq!(image.view().metadata().scaling, Scaling::IDENTITY);
    assert_eq!(Scaling::default(), Scaling::IDENTITY);
    assert_eq!(image.view().samples(), View::I16(&[1, 2, 3, 4]));
}

#[test]
fn image_constructor_accepts_empty_geometry_and_rejects_nonempty_samples() {
    let scaling = Scaling::IDENTITY;
    for shape in [Vec::new(), vec![0], vec![4, 0, 3]] {
        Image::new(shape.clone(), Data::F64(Vec::new()), scaling).unwrap();
        assert!(matches!(
            Image::new(shape, Data::F64(vec![1.0]), scaling),
            Err(FitsError::DataSizeMismatch {
                unit: SizeUnit::Samples,
                expected: 0,
                got: 1
            })
        ));
    }
    assert!(matches!(
        Image::from_u16(vec![2], &[1]),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Samples,
            expected: 2,
            got: 1
        })
    ));
}

#[test]
fn image_stored_view_preserves_exact_samples_immutably() {
    let image = Image::new(
        vec![3],
        Data::I64(vec![i64::MIN, 0, i64::MAX]),
        Scaling::IDENTITY,
    )
    .unwrap();
    assert_eq!(image.view().samples(), View::I64(&[i64::MIN, 0, i64::MAX]));
}

#[test]
fn decodes_big_endian_integers_and_floats() {
    // i16: 0x0001=1, 0xFFFF=-1, 0x8000=-32768 (the unsigned-u16 min sentinel).
    assert_eq!(
        Data::decode(&[0x00, 0x01, 0xFF, 0xFF, 0x80, 0x00], Bitpix::I16),
        Data::I16(vec![1, -1, -32768])
    );
    // BITPIX=8 is raw bytes, no byte order.
    assert_eq!(
        Data::decode(&[1, 2, 3], Bitpix::U8),
        Data::U8(vec![1, 2, 3])
    );
    // i32 0x00000100 = 256.
    assert_eq!(
        Data::decode(&[0, 0, 1, 0], Bitpix::I32),
        Data::I32(vec![256])
    );
    // i64 = 5.
    assert_eq!(
        Data::decode(&[0, 0, 0, 0, 0, 0, 0, 5], Bitpix::I64),
        Data::I64(vec![5])
    );
    // f32 1.0 = 0x3F800000, f64 1.0 = 0x3FF0000000000000.
    assert_eq!(
        Data::decode(&[0x3F, 0x80, 0x00, 0x00], Bitpix::F32),
        Data::F32(vec![1.0])
    );
    assert_eq!(
        Data::decode(&[0x3F, 0xF0, 0, 0, 0, 0, 0, 0], Bitpix::F64),
        Data::F64(vec![1.0])
    );
}

/// A caller generic over the sample type converts through `Sample`: a slice views as its
/// own variant, a view gives its samples back only as their own type, and `BITPIX`
/// names the type.
#[test]
fn samples_convert_by_type() {
    let view = View::from(&[1i16, -2][..]);
    assert_eq!(view, View::I16(&[1, -2]));
    assert_eq!(i16::of(view), Some(&[1, -2][..]));
    assert_eq!(i32::of(view), None);
    assert_eq!(<f32 as Sample>::BITPIX, Bitpix::F32);
    assert_eq!(Data::from(vec![0.5f64]), Data::F64(vec![0.5]));
    assert_eq!(f64::of_data(Data::F64(vec![0.5])), Ok(vec![0.5]));
}

/// The affine is one fused multiply-add: `BSCALE × raw` of 134217729² = 2⁵⁴ + 2²⁸ + 1
/// needs 55 bits, and `BZERO` = −(2⁵⁴ + 2²⁸) leaves exactly 1. A product rounded first
/// to its 53 bits loses the 1 and gives 0.
#[test]
fn physical_values_round_once() {
    let scaling = Scaling {
        bscale: 134_217_729.0,
        bzero: -18_014_398_777_917_440.0,
        blank: None,
    };
    let image = image(Data::I32(vec![134_217_729]), scaling);
    assert_eq!(physical::<f64>(&image), [1.0]);
    let product = 134_217_729.0f64 * 134_217_729.0;
    assert_eq!(product + scaling.bzero, 0.0);
}

#[test]
fn image_data_views_preserve_ranges_for_every_sample_type() {
    let u8_values = Data::U8(vec![1, 2, 3, 4]);
    assert_eq!(u8_values.view().slice(1..3), View::U8(&[2, 3]));
    // The typed accessors give the samples only as their own type, as `View`'s do.
    assert_eq!(u8_values.as_u8(), Some(&[1, 2, 3, 4][..]));
    assert_eq!(u8_values.as_i16(), None);
    assert_eq!(Data::F64(vec![0.5]).as_f64(), Some(&[0.5][..]));
    assert_eq!(Data::F64(vec![0.5]).as_f32(), None);

    let i16_values = Data::I16(vec![-2, -1, 0, 1]);
    assert_eq!(i16_values.view().slice(1..3), View::I16(&[-1, 0]));

    let i32_values = Data::I32(vec![-20, -10, 0, 10]);
    assert_eq!(i32_values.view().slice(1..3), View::I32(&[-10, 0]));

    let i64_values = Data::I64(vec![i64::MIN, -1, 0, i64::MAX]);
    assert_eq!(i64_values.view().slice(1..3), View::I64(&[-1, 0]));

    let f32_values = Data::F32(vec![-2.5, -0.0, 0.0, 3.5]);
    assert_eq!(f32_values.view().slice(1..3), View::F32(&[-0.0, 0.0]));

    let f64_values = Data::F64(vec![-2.5, -0.0, 0.0, 3.5]);
    assert_eq!(f64_values.view().slice(1..3), View::F64(&[-0.0, 0.0]));

    // Each typed accessor answers for its own variant alone, and the owned data converts
    // back to its vector, or comes back whole for any other type.
    let typed = |view: View<'_>| {
        [
            view.as_u8().is_some(),
            view.as_i16().is_some(),
            view.as_i32().is_some(),
            view.as_i64().is_some(),
            view.as_f32().is_some(),
            view.as_f64().is_some(),
        ]
    };
    let all = [
        &u8_values,
        &i16_values,
        &i32_values,
        &i64_values,
        &f32_values,
        &f64_values,
    ];
    for (variant, data) in all.into_iter().enumerate() {
        let expected: [bool; 6] = std::array::from_fn(|index| index == variant);
        assert_eq!(typed(data.view()), expected, "{:?}", data.bitpix());
    }
    assert_eq!(i16_values.view().as_i16(), Some(&[-2, -1, 0, 1][..]));
    assert_eq!(
        Vec::<i16>::try_from(i16_values.clone()),
        Ok(vec![-2, -1, 0, 1])
    );
    assert_eq!(Vec::<f32>::try_from(i16_values.clone()), Err(i16_values));
}

#[test]
fn a_borrowed_image_checks_its_sample_count_and_scaling() {
    let samples = [1i16, 2, 3, 4, 5, 6];
    let view = Borrowed::new(&[3, 2], View::I16(&samples), Scaling::IDENTITY).unwrap();
    assert_eq!(view.metadata().bitpix, Bitpix::I16);
    assert_eq!(view.to_image().samples, Data::I16(samples.to_vec()));
    assert!(matches!(
        Borrowed::new(&[4, 2], View::I16(&samples), Scaling::IDENTITY),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Samples,
            expected: 8,
            got: 6
        })
    ));
    // A float image carries no BLANK, which only an integer sample can equal.
    let floats = [1.0f32; 6];
    let blank = Scaling {
        blank: Some(0),
        ..Scaling::IDENTITY
    };
    assert!(matches!(
        Borrowed::new(&[3, 2], View::F32(&floats), blank),
        Err(FitsError::KeywordOutOfRange {
            keyword: keywords::BLANK
        })
    ));
    assert_eq!(
        Borrowed::new(&[3, 2], View::I16(&samples), blank)
            .unwrap()
            .scaling(),
        blank
    );
}

#[test]
fn encode_produces_big_endian_bytes() {
    assert_eq!(
        encoded(&Data::I16(vec![1, -1])),
        vec![0x00, 0x01, 0xFF, 0xFF]
    );
    assert_eq!(encoded(&Data::U8(vec![1, 2, 3])), vec![1, 2, 3]);
}

#[test]
fn encode_is_the_inverse_of_decode() {
    let cases = [
        Data::U8(vec![0, 1, 255]),
        Data::I16(vec![1, -1, -32768, 32767]),
        Data::I32(vec![256, -1, i32::MIN]),
        Data::I64(vec![5, -5, i64::MAX]),
        Data::F32(vec![1.0, -2.5, 0.0]),
        Data::F64(vec![1.0, -2.5, f64::MAX]),
    ];
    for data in cases {
        let bytes = encoded(&data);
        assert_eq!(Data::decode(&bytes, data.bitpix()), data);
    }
}

#[test]
fn swap_into_words_then_view_matches_decode_and_reuses_one_alloc() {
    // The reader's view path: swap big-endian bytes into a u64-backed scratch, then
    // reinterpret as a typed View. Each type's view must equal the owned decode,
    // and the cross-BITPIX scratch must reuse one allocation across types.
    let cases = [
        Data::I16(vec![1, -1, -32768, 32767]),
        Data::I32(vec![256, -1, i32::MIN]),
        Data::I64(vec![5, -5, i64::MAX]),
        Data::F32(vec![1.0, -2.5, 0.0]),
        Data::F64(vec![1.0, -2.5, f64::MAX]),
    ];
    let mut words: Vec<u64> = Vec::new();
    for data in &cases {
        let be = encoded(data);
        data.bitpix().swap_into_words(&be, &mut words);
        let view = data.bitpix().view_words(&words, be.len());
        // The view's samples equal the owned decode (compare via a fresh Data).
        let owned = Data::decode(&be, data.bitpix());
        let view_owned = match view {
            View::I16(v) => Data::I16(v.to_vec()),
            View::I32(v) => Data::I32(v.to_vec()),
            View::I64(v) => Data::I64(v.to_vec()),
            View::F32(v) => Data::F32(v.to_vec()),
            View::F64(v) => Data::F64(v.to_vec()),
            View::U8(v) => Data::U8(v.to_vec()),
        };
        assert_eq!(
            view_owned,
            owned,
            "view of {:?} must equal decode",
            data.bitpix()
        );
    }

    // Cross-BITPIX reuse: two equal-byte-length decodes of *different* types must
    // reuse the same u64 backing (no realloc) — the page-fault-free property the
    // reader's scratch relies on. 4096 bytes = 2048 i16 then 1024 i32.
    let i16s = Data::I16((0..2048).collect());
    let i32s = Data::I32((0..1024).map(|i| i - 7).collect());
    let mut w: Vec<u64> = Vec::new();
    Bitpix::I16.swap_into_words(&encoded(&i16s), &mut w);
    let ptr1 = w.as_ptr();
    Bitpix::I32.swap_into_words(&encoded(&i32s), &mut w);
    let ptr2 = w.as_ptr();
    assert_eq!(
        ptr1, ptr2,
        "equal-length cross-type swap must reuse the scratch"
    );
    // ...and the i32 view is still correct after reusing the i16 scratch.
    let view = Bitpix::I32.view_words(&w, encoded(&i32s).len());
    assert_eq!(
        view,
        View::I32(&(0..1024).map(|i| i - 7).collect::<Vec<i32>>())
    );
}

#[test]
fn float_inf_and_nan_payloads_round_trip_bit_for_bit() {
    // §3.4 / Appendix E mandate preserving ±Inf and signaling/quiet NaN payloads
    // without canonicalizing. PartialEq can't compare NaN, so check raw bits.
    let f32_bits: [u32; 5] = [
        0x7F80_0000, // +Inf
        0xFF80_0000, // -Inf
        0x7FC0_0000, // quiet NaN
        0x7F80_0001, // signaling NaN, payload 1
        0x7FAB_CDEF, // NaN with a payload
    ];
    let f32s: Vec<f32> = f32_bits.iter().map(|&b| f32::from_bits(b)).collect();
    let decoded = Data::decode(&encoded(&Data::F32(f32s)), Bitpix::F32);
    let Data::F32(out) = decoded else {
        panic!("expected F32")
    };
    for (i, (&b, o)) in f32_bits.iter().zip(&out).enumerate() {
        assert_eq!(o.to_bits(), b, "f32 pattern {i}");
    }

    let f64_bits: [u64; 5] = [
        0x7FF0_0000_0000_0000, // +Inf
        0xFFF0_0000_0000_0000, // -Inf
        0x7FF8_0000_0000_0000, // quiet NaN
        0x7FF0_0000_0000_0001, // signaling NaN, payload 1
        0x7FF0_0000_DEAD_BEEF, // NaN with a payload
    ];
    let f64s: Vec<f64> = f64_bits.iter().map(|&b| f64::from_bits(b)).collect();
    let decoded = Data::decode(&encoded(&Data::F64(f64s)), Bitpix::F64);
    let Data::F64(out) = decoded else {
        panic!("expected F64")
    };
    for (i, (&b, o)) in f64_bits.iter().zip(&out).enumerate() {
        assert_eq!(o.to_bits(), b, "f64 pattern {i}");
    }
}

#[test]
fn physical_applies_scaling_and_maps_blank_to_nan() {
    // 10 -> 5 + 2·10 = 25 ; 20 == BLANK -> NaN ; -5 -> 5 + 2·-5 = -5
    let img = image(
        Data::I16(vec![10, 20, -5]),
        Scaling {
            bscale: 2.0,
            bzero: 5.0,
            blank: Some(20),
        },
    );
    let phys = physical::<f64>(&img);
    assert_eq!(phys[0], 25.0);
    assert!(phys[1].is_nan());
    assert_eq!(phys[2], -5.0);
    let f32s = physical::<f32>(&img);
    assert_eq!(f32s[0], 25.0_f32);
    assert!(f32s[1].is_nan());
    assert_eq!(f32s[2], -5.0_f32);

    // The f32 plane is the f64 plane narrowed element-wise, even where f32 must
    // round: BSCALE = 0.1 is not representable, so the scaling accrues error that
    // the shared f64 evaluation then narrows identically in both methods.
    let rounded = image(
        Data::I32(vec![0, 7, 123_456_789]),
        Scaling {
            bscale: 0.1,
            bzero: 0.0,
            blank: None,
        },
    );
    let via_f64: Vec<f32> = physical::<f64>(&rounded)
        .into_iter()
        .map(|v| v as f32)
        .collect();
    assert_eq!(physical::<f32>(&rounded), via_f64);
}

/// Signed storage with the matching `BZERO` offset is the unsigned (or signed-byte)
/// integer with its sign bit flipped: the typed view recovers it exactly from an
/// image and from raw big-endian bytes, and the physical plane is its value.
#[test]
fn unsigned_view_recovers_exact_typed_integers() {
    let cases = [
        (
            Data::U8(vec![0, 128, 255]),
            I8_OFFSET,
            UnsignedData::I8(vec![-128, 0, 127]),
        ),
        (
            Data::I16(vec![i16::MIN, 0, i16::MAX]),
            U16_OFFSET,
            UnsignedData::U16(vec![0, 32_768, u16::MAX]),
        ),
        (
            Data::I32(vec![i32::MIN, 0, i32::MAX]),
            U32_OFFSET,
            UnsignedData::U32(vec![0, 2_147_483_648, u32::MAX]),
        ),
        (
            Data::I64(vec![i64::MIN, 0, i64::MAX]),
            U64_OFFSET,
            UnsignedData::U64(vec![0, 9_223_372_036_854_775_808, u64::MAX]),
        ),
    ];
    for (samples, bzero, expected) in cases {
        let scaling = Scaling {
            bscale: 1.0,
            bzero,
            blank: None,
        };
        let image = image(samples, scaling);
        assert_eq!(image.view().unsigned(), Some(expected.clone()));
        // Each value is exact in f64 except u64::MAX, which both sides round to 2⁶⁴.
        let values: Vec<f64> = match expected {
            UnsignedData::I8(v) => v.into_iter().map(f64::from).collect(),
            UnsignedData::U16(v) => v.into_iter().map(f64::from).collect(),
            UnsignedData::U32(v) => v.into_iter().map(f64::from).collect(),
            UnsignedData::U64(v) => v.into_iter().map(|v| v as f64).collect(),
        };
        assert_eq!(physical::<f64>(&image), values);
    }
}

#[test]
fn from_unsigned_constructors_invert_the_unsigned_view() {
    // from_uN builds the signed storage + BZERO offset; unsigned() recovers it.
    let u16s = vec![0u16, 1, 32768, 65535];
    let img = Image::from_u16(vec![4], &u16s).unwrap();
    assert_eq!(img.samples, Data::I16(vec![-32768, -32767, 0, 32767]));
    assert_eq!(img.scaling.bzero, 32768.0);
    assert_eq!(img.view().unsigned(), Some(UnsignedData::U16(u16s)));

    let u64s = vec![0u64, 1u64 << 63, u64::MAX];
    assert_eq!(
        Image::from_u64(vec![3], &u64s).unwrap().view().unsigned(),
        Some(UnsignedData::U64(u64s))
    );
    let i8s = vec![i8::MIN, 0, i8::MAX];
    assert_eq!(
        Image::from_i8(vec![3], &i8s).unwrap().view().unsigned(),
        Some(UnsignedData::I8(i8s))
    );
}

#[test]
fn unsigned_u64_view_is_exact_where_physical_rounds() {
    // 2⁵³+1 is the smallest integer f64 cannot represent. The typed view recovers
    // it exactly; physical() (f64) rounds it to 2⁵³.
    let exact = 9_007_199_254_740_993u64; // 2⁵³ + 1
    let stored = (exact ^ 0x8000_0000_0000_0000).cast_signed();
    let img = image(
        Data::I64(vec![stored]),
        Scaling {
            bscale: 1.0,
            bzero: U64_OFFSET,
            blank: None,
        },
    );
    assert_eq!(img.view().unsigned(), Some(UnsignedData::U64(vec![exact])));
    assert_eq!(physical::<f64>(&img)[0], (exact - 1) as f64); // rounded to 2⁵³
}

#[test]
fn unsigned_returns_none_for_non_unsigned_scaling() {
    // Plain signed (BZERO=0) and a genuinely scaled image are not unsigned views.
    let signed = image(Data::I16(vec![1, 2, 3]), Scaling::IDENTITY);
    assert_eq!(signed.view().unsigned(), None);
    let scaled = image(
        Data::I16(vec![1, 2]),
        Scaling {
            bscale: 2.0,
            bzero: U16_OFFSET,
            blank: None,
        },
    );
    assert_eq!(scaled.view().unsigned(), None); // BSCALE ≠ 1
}

#[test]
fn float_physical_scales_and_passes_nan_through() {
    let img = image(
        Data::F32(vec![1.5, f32::NAN]),
        Scaling {
            bscale: 10.0,
            bzero: 1.0,
            blank: None,
        },
    );
    let phys = physical::<f64>(&img);
    assert_eq!(phys[0], 16.0); // 1 + 10·1.5
    assert!(phys[1].is_nan());
}

#[test]
fn the_stored_byte_pass_gives_the_physical_plane_of_the_sample_pass() {
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
    for (samples, expected) in cases {
        let bitpix = samples.bitpix();
        let bytes = encoded(&samples);
        let mut from_bytes: Vec<f64> = Vec::new();
        bitpix.extend_physical_be(&bytes, &scaling, &mut from_bytes);
        assert_eq!(from_bytes.len(), expected.len());
        for (got, want) in from_bytes.iter().zip(&expected) {
            assert!(
                got == want || got.is_nan() && want.is_nan(),
                "{:?}: got {from_bytes:?}, expected {expected:?}",
                samples.bitpix()
            );
        }
        let mut from_bytes_f32: Vec<f32> = Vec::new();
        bitpix.extend_physical_be(&bytes, &scaling, &mut from_bytes_f32);
        assert_eq!(from_bytes_f32.len(), expected.len());
        for (got, want) in from_bytes_f32.iter().zip(&expected) {
            assert!(
                *got == *want as f32 || got.is_nan() && want.is_nan(),
                "{:?}: got {from_bytes_f32:?}, expected {expected:?}",
                samples.bitpix()
            );
        }
        // The one pass from the stored bytes and the pass over host-endian samples are
        // two implementations of one map: they agree bit for bit, in both planes.
        let bits = |values: &[f64]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        let image = image(samples, scaling);
        assert_eq!(
            bits(&physical::<f64>(&image)),
            bits(&from_bytes),
            "{bitpix:?}"
        );
        let narrow: Vec<f32> = physical::<f32>(&image);
        assert_eq!(
            narrow.iter().map(|v| v.to_bits()).collect::<Vec<_>>(),
            from_bytes_f32
                .iter()
                .map(|v| v.to_bits())
                .collect::<Vec<_>>(),
            "{bitpix:?}"
        );
    }
}

#[test]
fn sample_type_resolves_unsigned_and_signed_byte_conventions() {
    let s = |bscale: f64, bzero: f64| Scaling {
        bscale,
        bzero,
        blank: None,
    };

    // Plain signed BITPIX (no offset) keeps the stored signed type.
    assert_eq!(
        SampleType::from_scaling(Bitpix::I16, s(1.0, 0.0)),
        SampleType::I16
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::I32, s(1.0, 0.0)),
        SampleType::I32
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::I64, s(1.0, 0.0)),
        SampleType::I64
    );

    // The unsigned convention: BSCALE=1 with BZERO the sign-bit offset 2^(n-1).
    assert_eq!(
        SampleType::from_scaling(Bitpix::I16, s(1.0, U16_OFFSET)),
        SampleType::U16
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::I32, s(1.0, U32_OFFSET)),
        SampleType::U32
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::I64, s(1.0, U64_OFFSET)),
        SampleType::U64
    );

    // BITPIX=8 is unsigned by default; BZERO=-128 is the signed-byte convention.
    assert_eq!(
        SampleType::from_scaling(Bitpix::U8, s(1.0, 0.0)),
        SampleType::U8
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::U8, s(1.0, I8_OFFSET)),
        SampleType::I8
    );

    // Floats are unaffected by scaling.
    assert_eq!(
        SampleType::from_scaling(Bitpix::F32, s(10.0, 1.0)),
        SampleType::F32
    );
    assert_eq!(
        SampleType::from_scaling(Bitpix::F64, s(1.0, 0.0)),
        SampleType::F64
    );

    // A genuine BSCALE (≠ 1) at the offset BZERO is NOT the unsigned convention.
    assert_eq!(
        SampleType::from_scaling(Bitpix::I16, s(2.0, U16_OFFSET)),
        SampleType::I16
    );

    // BLANK marks nulls within a type; it must not change the classification.
    let with_blank = Scaling {
        bscale: 1.0,
        bzero: U16_OFFSET,
        blank: Some(-1),
    };
    assert_eq!(
        SampleType::from_scaling(Bitpix::I16, with_blank),
        SampleType::U16
    );

    // ...but it does disqualify the *exact* view: a BLANK sample has no integer
    // value to recover. `Scaling::unsigned_kind` is where that guard lives, and it
    // is the one resolution both the image (`BZERO`/`BLANK`) and binary-table
    // (`TZEROn`/`TNULLn`) paths go through — so the two cannot disagree.
    assert_eq!(with_blank.unsigned_kind(Bitpix::I16), None);
    assert_eq!(
        s(1.0, U16_OFFSET).unsigned_kind(Bitpix::I16),
        Some(UnsignedKind::U16)
    );
    assert_eq!(
        s(1.0, I8_OFFSET).unsigned_kind(Bitpix::U8),
        Some(UnsignedKind::I8)
    );
    // No offset to undo: a native unsigned byte, a plain signed integer, a float.
    assert_eq!(s(1.0, 0.0).unsigned_kind(Bitpix::U8), None);
    assert_eq!(s(1.0, 0.0).unsigned_kind(Bitpix::I16), None);
    assert_eq!(s(1.0, 0.0).unsigned_kind(Bitpix::F32), None);
    // The offset is only the convention at unit BSCALE.
    assert_eq!(s(2.0, U16_OFFSET).unsigned_kind(Bitpix::I16), None);
}

#[test]
fn sample_type_predicates_and_image_accessor() {
    assert!(SampleType::U16.is_unsigned());
    assert!(SampleType::U16.is_integer());
    assert!(!SampleType::U16.is_float());
    assert!(SampleType::I32.is_integer());
    assert!(!SampleType::I32.is_unsigned());
    assert!(SampleType::I8.is_integer());
    assert!(!SampleType::I8.is_unsigned());
    assert!(SampleType::F64.is_float());
    assert!(!SampleType::F64.is_integer());

    // Image::sample_type resolves the convention from its own scaling.
    let unsigned = image(
        Data::I16(vec![0, 1, 2]),
        Scaling {
            bscale: 1.0,
            bzero: U16_OFFSET,
            blank: None,
        },
    );
    assert_eq!(unsigned.view().sample_type(), SampleType::U16);
    assert_eq!(unsigned.view().metadata().sample_type(), SampleType::U16);
    let signed = image(Data::I16(vec![0, 1]), Scaling::IDENTITY);
    assert_eq!(signed.view().sample_type(), SampleType::I16);
}

#[test]
fn image_data_reports_its_bitpix() {
    assert_eq!(Data::U8(vec![]).bitpix(), Bitpix::U8);
    assert_eq!(Data::I16(vec![]).bitpix(), Bitpix::I16);
    assert_eq!(Data::I32(vec![]).bitpix(), Bitpix::I32);
    assert_eq!(Data::I64(vec![]).bitpix(), Bitpix::I64);
    assert_eq!(Data::F32(vec![]).bitpix(), Bitpix::F32);
    assert_eq!(Data::F64(vec![]).bitpix(), Bitpix::F64);
}

#[test]
fn scaling_defaults_to_the_identity_map() {
    let s = Scaling::from_header(&header(&["SIMPLE  = T"]), Bitpix::I16).unwrap();
    assert_eq!(s, Scaling::IDENTITY);
    assert!(s.is_identity());
}

#[test]
fn scaling_reads_explicit_keywords() {
    let explicit = header(&["BSCALE  = 2.5", "BZERO   = -1000.0", "BLANK   = -32768"]);
    let s = Scaling::from_header(&explicit, Bitpix::I16).unwrap();
    assert_eq!(
        s,
        Scaling {
            bscale: 2.5,
            bzero: -1000.0,
            blank: Some(-32768)
        }
    );
    assert!(!s.is_identity());
    // A float array ignores `BLANK`; an integer one refuses a `BLANK` its type cannot
    // hold: -32768 is no `u8`.
    assert_eq!(
        Scaling::from_header(&explicit, Bitpix::F32).unwrap().blank,
        None
    );
    assert!(matches!(
        Scaling::from_header(&explicit, Bitpix::U8),
        Err(FitsError::KeywordOutOfRange {
            keyword: keywords::BLANK
        })
    ));

    for keyword in ["BSCALE", "BZERO", "BLANK"] {
        let mut malformed = Header::new();
        malformed.set_internal(keyword, "not numeric");
        assert!(matches!(
            Scaling::from_header(&malformed, Bitpix::I32),
            Err(FitsError::ValueTypeMismatch { name: RecordName::Keyword(name), .. }) if name == keyword
        ));
    }

    let oversized_blank = header(&["BITPIX  = 64", "BLANK   = 9223372036854775808"]);
    assert!(matches!(
        Scaling::from_header(&oversized_blank, Bitpix::I64),
        Err(FitsError::IntegerOutOfRange {
            keyword: Some(keywords::BLANK),
            value,
            target: NumericTarget::I64,
        }) if value == "9223372036854775808"
    ));
}

#[test]
fn unsigned_16_bit_offset_is_not_an_identity_map() {
    // The unsigned-u16 trick: BSCALE=1, BZERO=32768.
    let s =
        Scaling::from_header(&header(&["BSCALE  = 1", "BZERO   = 32768"]), Bitpix::I16).unwrap();
    assert_eq!(s.bzero, 32768.0);
    assert!(!s.is_identity());
}
