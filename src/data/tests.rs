use super::*;
use crate::block::CARD_SIZE;

fn header(lines: &[&str]) -> Header {
    let mut buf = Vec::new();
    for line in lines {
        let mut card = [b' '; CARD_SIZE];
        card[..line.len()].copy_from_slice(line.as_bytes());
        buf.extend_from_slice(&card);
    }
    let mut end = [b' '; CARD_SIZE];
    end[..3].copy_from_slice(b"END");
    buf.extend_from_slice(&end);
    Header::parse(&buf).unwrap()
}

fn image(samples: ImageData, scaling: Scaling) -> Image {
    Image {
        shape: vec![samples.len()],
        samples,
        scaling,
    }
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
        ImageData::I16(vec![1, -1]).encode(),
        vec![0x00, 0x01, 0xFF, 0xFF]
    );
    assert_eq!(ImageData::U8(vec![1, 2, 3]).encode(), vec![1, 2, 3]);
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
        let bytes = data.encode();
        assert_eq!(ImageData::decode(&bytes, data.bitpix()), data);
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
