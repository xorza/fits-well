use super::*;
use crate::header::from_card_lines as header;

fn image(samples: ImageData, scaling: Scaling) -> Image {
    Image {
        shape: vec![samples.len()],
        samples,
        scaling,
    }
}

/// Encode to a fresh buffer — the allocating shorthand over `encode_into` the
/// round-trip tests read against.
fn encoded(data: &ImageData) -> Vec<u8> {
    let mut out = Vec::new();
    data.encode_into(&mut out);
    out
}

#[test]
fn decodes_big_endian_integers_and_floats() {
    // i16: 0x0001=1, 0xFFFF=-1, 0x8000=-32768 (the unsigned-u16 min sentinel).
    assert_eq!(
        ImageData::decode(&[0x00, 0x01, 0xFF, 0xFF, 0x80, 0x00], Bitpix::I16),
        ImageData::I16(vec![1, -1, -32768])
    );
    // BITPIX=8 is raw bytes, no byte order.
    assert_eq!(
        ImageData::decode(&[1, 2, 3], Bitpix::U8),
        ImageData::U8(vec![1, 2, 3])
    );
    // i32 0x00000100 = 256.
    assert_eq!(
        ImageData::decode(&[0, 0, 1, 0], Bitpix::I32),
        ImageData::I32(vec![256])
    );
    // i64 = 5.
    assert_eq!(
        ImageData::decode(&[0, 0, 0, 0, 0, 0, 0, 5], Bitpix::I64),
        ImageData::I64(vec![5])
    );
    // f32 1.0 = 0x3F800000, f64 1.0 = 0x3FF0000000000000.
    assert_eq!(
        ImageData::decode(&[0x3F, 0x80, 0x00, 0x00], Bitpix::F32),
        ImageData::F32(vec![1.0])
    );
    assert_eq!(
        ImageData::decode(&[0x3F, 0xF0, 0, 0, 0, 0, 0, 0], Bitpix::F64),
        ImageData::F64(vec![1.0])
    );
}

#[test]
fn encode_produces_big_endian_bytes() {
    assert_eq!(
        encoded(&ImageData::I16(vec![1, -1])),
        vec![0x00, 0x01, 0xFF, 0xFF]
    );
    assert_eq!(encoded(&ImageData::U8(vec![1, 2, 3])), vec![1, 2, 3]);
}

#[test]
fn encode_is_the_inverse_of_decode() {
    let cases = [
        ImageData::U8(vec![0, 1, 255]),
        ImageData::I16(vec![1, -1, -32768, 32767]),
        ImageData::I32(vec![256, -1, i32::MIN]),
        ImageData::I64(vec![5, -5, i64::MAX]),
        ImageData::F32(vec![1.0, -2.5, 0.0]),
        ImageData::F64(vec![1.0, -2.5, f64::MAX]),
    ];
    for data in cases {
        let bytes = encoded(&data);
        assert_eq!(ImageData::decode(&bytes, data.bitpix()), data);
    }
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
    let decoded = ImageData::decode(&encoded(&ImageData::F32(f32s)), Bitpix::F32);
    let ImageData::F32(out) = decoded else {
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
    let decoded = ImageData::decode(&encoded(&ImageData::F64(f64s)), Bitpix::F64);
    let ImageData::F64(out) = decoded else {
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
        ImageData::I16(vec![10, 20, -5]),
        Scaling {
            bscale: 2.0,
            bzero: 5.0,
            blank: Some(20),
        },
    );
    let phys = img.physical();
    assert_eq!(phys[0], 25.0);
    assert!(phys[1].is_nan());
    assert_eq!(phys[2], -5.0);
}

#[test]
fn unsigned_view_recovers_exact_typed_integers() {
    // Signed storage + the matching BZERO offset decodes back to the unsigned (or
    // signed-byte) values by flipping the stored sign bit.
    let u16_img = image(
        ImageData::I16(vec![-32768, 0, 32767]),
        Scaling {
            bscale: 1.0,
            bzero: 32768.0,
            blank: None,
        },
    );
    assert_eq!(
        u16_img.unsigned(),
        Some(UnsignedView::U16(vec![0, 32768, 65535]))
    );
    let u32_img = image(
        ImageData::I32(vec![i32::MIN, 0, i32::MAX]),
        Scaling {
            bscale: 1.0,
            bzero: 2_147_483_648.0,
            blank: None,
        },
    );
    assert_eq!(
        u32_img.unsigned(),
        Some(UnsignedView::U32(vec![0, 2_147_483_648, u32::MAX]))
    );
    let i8_img = image(
        ImageData::U8(vec![0, 128, 255]),
        Scaling {
            bscale: 1.0,
            bzero: -128.0,
            blank: None,
        },
    );
    assert_eq!(
        i8_img.unsigned(),
        Some(UnsignedView::I8(vec![-128, 0, 127]))
    );
}

#[test]
fn from_unsigned_constructors_invert_the_unsigned_view() {
    // from_uN builds the signed storage + BZERO offset; unsigned() recovers it.
    let u16s = vec![0u16, 1, 32768, 65535];
    let img = Image::from_u16(vec![4], &u16s);
    assert_eq!(img.samples, ImageData::I16(vec![-32768, -32767, 0, 32767]));
    assert_eq!(img.scaling.bzero, 32768.0);
    assert_eq!(img.unsigned(), Some(UnsignedView::U16(u16s)));

    let u64s = vec![0u64, 1u64 << 63, u64::MAX];
    assert_eq!(
        Image::from_u64(vec![3], &u64s).unsigned(),
        Some(UnsignedView::U64(u64s))
    );
    let i8s = vec![i8::MIN, 0, i8::MAX];
    assert_eq!(
        Image::from_i8(vec![3], &i8s).unsigned(),
        Some(UnsignedView::I8(i8s))
    );
}

#[test]
fn unsigned_u64_view_is_exact_where_physical_rounds() {
    // 2⁵³+1 is the smallest integer f64 cannot represent. The typed view recovers
    // it exactly; physical() (f64) rounds it to 2⁵³.
    let exact = 9_007_199_254_740_993u64; // 2⁵³ + 1
    let stored = (exact ^ 0x8000_0000_0000_0000) as i64;
    let img = image(
        ImageData::I64(vec![stored]),
        Scaling {
            bscale: 1.0,
            bzero: 9_223_372_036_854_775_808.0, // 2⁶³
            blank: None,
        },
    );
    assert_eq!(img.unsigned(), Some(UnsignedView::U64(vec![exact])));
    assert_eq!(img.physical()[0] as u64, exact - 1); // rounded to 2⁵³
}

#[test]
fn unsigned_returns_none_for_non_unsigned_scaling() {
    // Plain signed (BZERO=0) and a genuinely scaled image are not unsigned views.
    let signed = image(
        ImageData::I16(vec![1, 2, 3]),
        Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    );
    assert_eq!(signed.unsigned(), None);
    let scaled = image(
        ImageData::I16(vec![1, 2]),
        Scaling {
            bscale: 2.0,
            bzero: 32768.0,
            blank: None,
        },
    );
    assert_eq!(scaled.unsigned(), None); // BSCALE ≠ 1
}

#[test]
fn physical_realizes_unsigned_16_bit_via_the_bzero_offset() {
    // u16 trick: signed-16 storage with BSCALE=1, BZERO=32768.
    // -32768 -> 0, 0 -> 32768, 32767 -> 65535.
    let img = image(
        ImageData::I16(vec![-32768, 0, 32767]),
        Scaling {
            bscale: 1.0,
            bzero: 32768.0,
            blank: None,
        },
    );
    assert_eq!(img.physical(), vec![0.0, 32768.0, 65535.0]);
}

#[test]
fn float_physical_scales_and_passes_nan_through() {
    let img = image(
        ImageData::F32(vec![1.5, f32::NAN]),
        Scaling {
            bscale: 10.0,
            bzero: 1.0,
            blank: None,
        },
    );
    let phys = img.physical();
    assert_eq!(phys[0], 16.0); // 1 + 10·1.5
    assert!(phys[1].is_nan());
}

#[test]
fn image_data_reports_its_bitpix() {
    assert_eq!(ImageData::U8(vec![]).bitpix(), Bitpix::U8);
    assert_eq!(ImageData::I16(vec![]).bitpix(), Bitpix::I16);
    assert_eq!(ImageData::I32(vec![]).bitpix(), Bitpix::I32);
    assert_eq!(ImageData::I64(vec![]).bitpix(), Bitpix::I64);
    assert_eq!(ImageData::F32(vec![]).bitpix(), Bitpix::F32);
    assert_eq!(ImageData::F64(vec![]).bitpix(), Bitpix::F64);
}

#[test]
fn scaling_defaults_to_the_identity_map() {
    let s = Scaling::from_header(&header(&["SIMPLE  = T"]));
    assert_eq!(
        s,
        Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None
        }
    );
    assert!(s.is_identity());
}

#[test]
fn scaling_reads_explicit_keywords() {
    let s = Scaling::from_header(&header(&[
        "BSCALE  = 2.5",
        "BZERO   = -1000.0",
        "BLANK   = -32768",
    ]));
    assert_eq!(
        s,
        Scaling {
            bscale: 2.5,
            bzero: -1000.0,
            blank: Some(-32768)
        }
    );
    assert!(!s.is_identity());
}

#[test]
fn unsigned_16_bit_offset_is_not_an_identity_map() {
    // The unsigned-u16 trick: BSCALE=1, BZERO=32768.
    let s = Scaling::from_header(&header(&["BSCALE  = 1", "BZERO   = 32768"]));
    assert_eq!(s.bzero, 32768.0);
    assert!(!s.is_identity());
}
