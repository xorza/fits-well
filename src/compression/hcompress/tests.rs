use crate::compression::HcompressScale;
use crate::compression::hcompress;
use crate::compression::hcompress::Smoothing;
use crate::compression::hcompress::stream_header;
use crate::compression::plane::IntBitpix;
use crate::compression::tile_codec::Tile;
use crate::fits_error::{CompressedDataDefect, FitsError, HcompressDefect};
use crate::internals::fixture::Fixture;
use std::assert_matches;

/// The lossless codec, read back with no smoothing.
const LOSSLESS: hcompress::Codec = hcompress::Codec {
    scale: HcompressScale::Lossless,
    smoothing: Smoothing::Off,
    plane: hcompress::Codec::PLANE_2D,
};

/// A tile of `shape`, whose samples' width an HCOMPRESS stream does not read.
fn tile(shape: &[usize]) -> Tile<'_> {
    Tile {
        shape,
        width: IntBitpix::I64,
        blank: None,
    }
}

/// `values` of `tdims` as a lossless `HCOMPRESS_1` stream.
fn lossless(values: &[i64], tdims: &[usize], scratch: &mut hcompress::HcompressScratch) -> Vec<u8> {
    LOSSLESS
        .encode(values, tile(tdims), scratch)
        .unwrap()
        .to_vec()
}

#[test]
fn transform_of_64_bit_values_matches_external_golden() {
    const RAW: &[u8] = Fixture::HcompWideI32Raw.bytes();
    const COMPRESSED: &[u8] = Fixture::HcompWideI32Huf.bytes();

    let values: Vec<i64> = RAW
        .as_chunks::<4>()
        .0
        .iter()
        .map(|bytes| i64::from(i32::from_be_bytes(*bytes)))
        .collect();
    assert_eq!(values.len(), 100 * 100);
    assert!(
        i64::from_be_bytes(COMPRESSED[stream_header::SUM].try_into().unwrap())
            > i64::from(i32::MAX),
        "the golden must require a wide H-transform accumulator"
    );
    assert!(
        COMPRESSED[22..25].iter().any(|&planes| planes > 31),
        "the golden must require more than 31 coefficient bit planes"
    );

    let mut scratch = hcompress::HcompressScratch::default();
    let encoded = lossless(&values, &[100, 100], &mut scratch);
    assert_eq!(encoded, COMPRESSED);

    let decoded = LOSSLESS
        .decode(COMPRESSED, tile(&[100, 100]), &mut scratch)
        .unwrap()
        .clone();
    assert_eq!(decoded, values);
}

/// A tile of ny = 2 by nx = 3 decodes losslessly as itself. The decoder reads nx and ny
/// from the stream and checks them against the tile shape it is handed, so another shape,
/// the transposed one of the same count among them, is refused, not transformed blindly.
#[test]
fn tile_decode_refuses_another_shape() {
    let vals: Vec<i64> = vec![10, 20, 30, 40, 50, 60];
    let mut scratch = hcompress::HcompressScratch::default();
    let bytes = lossless(&vals, &[2, 3], &mut scratch);
    let out = LOSSLESS
        .decode(&bytes, tile(&[2, 3]), &mut scratch)
        .unwrap()
        .clone();
    assert_eq!(out, vals);
    for tdims in [[3, 2], [7, 1], [5, 1], [6, 1]] {
        assert_matches!(
            LOSSLESS.decode(&bytes, tile(&tdims), &mut scratch),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::Hcompress(HcompressDefect::TileDimensions),
            }),
            "{tdims:?}"
        );
    }
}

/// A 1×1 tile skips its empty quadrants, and each strict prefix of its stream ends inside
/// it.
#[test]
fn one_pixel_tile_decodes_and_each_prefix_ends_early() {
    let mut scratch = hcompress::HcompressScratch::default();
    let one = lossless(&[-7], &[1, 1], &mut scratch);
    let out = LOSSLESS
        .decode(&one, tile(&[1, 1]), &mut scratch)
        .unwrap()
        .clone();
    assert_eq!(out, [-7]);
    for end in 0..one.len() {
        assert_matches!(
            LOSSLESS.decode(&one[..end], tile(&[1, 1]), &mut scratch),
            Err(FitsError::UnexpectedEof),
            "strict HCOMPRESS prefix of length {end}"
        );
    }
}

/// A defective stream header is refused: a negative dimension, a scale that doubles the
/// sum past 64 bits, which no encoder writes, and a bit-plane count past 63.
#[test]
fn defective_stream_header_fields_are_refused() {
    let mut scratch = hcompress::HcompressScratch::default();
    let one = lossless(&[-7], &[1, 1], &mut scratch);
    let mut negative_ny = one.clone();
    negative_ny[stream_header::NY].copy_from_slice(&(-1i32).to_be_bytes());
    assert_matches!(
        LOSSLESS.decode(&negative_ny, tile(&[1, 1]), &mut scratch),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::Hcompress(HcompressDefect::NegativeDimension),
        })
    );
    let mut overflowing = one.clone();
    overflowing[stream_header::SCALE].copy_from_slice(&2i32.to_be_bytes());
    overflowing[stream_header::SUM].copy_from_slice(&i64::MAX.to_be_bytes());
    assert_matches!(
        LOSSLESS.decode(&overflowing, tile(&[1, 1]), &mut scratch),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::Hcompress(HcompressDefect::CoefficientOverflow),
        })
    );
    let mut invalid_planes = one;
    invalid_planes[stream_header::PLANES] = 64;
    assert_matches!(
        LOSSLESS.decode(&invalid_planes, tile(&[1, 1]), &mut scratch),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::Hcompress(HcompressDefect::BitPlaneCount),
        })
    );
}
