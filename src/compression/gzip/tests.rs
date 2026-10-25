use std::assert_matches;
use std::io::Write;

use flate2::{Compression, Crc, Decompress, GzBuilder};

use crate::compression::gzip;

use crate::fits_error::{CompressedDataDefect, FitsError, SizeUnit};

#[test]
fn gunzip_refuses_a_stream_larger_than_its_tile() {
    // A decompression bomb: a tiny gzip stream inflating far past the tile's known
    // byte size must be rejected, not allocated unbounded.
    let big = vec![0u8; 100_000];
    let mut scratch = gzip::GzipScratch::default();
    let bomb = gzip::Codec {
        level: 9,
        shuffle: 1,
    }
    .encode(&big, &mut scratch)
    .to_vec();
    assert!(bomb.len() < 1000, "an all-zero buffer compresses tiny");
    let mut decoded = Vec::new();
    let mut inflate = Decompress::new(false);
    // Bounded at 1 KiB (a small tile): inflating to 100 KB overruns → error.
    assert_matches!(
        gzip::gunzip_into(&bomb, 1024, &mut decoded, &mut inflate),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::GzipOverrun,
        })
    );
    // One byte short of the true size still overruns (the +1 detection boundary).
    assert_matches!(
        gzip::gunzip_into(&bomb, big.len() - 1, &mut decoded, &mut inflate),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::GzipOverrun
        })
    );
    // Bounded at the true size: decodes to exactly the original bytes.
    gzip::gunzip_into(&bomb, big.len(), &mut decoded, &mut inflate).unwrap();
    assert_eq!(decoded, big);

    assert_matches!(
        gzip::gunzip_into(b"not a gzip stream", 4, &mut decoded, &mut inflate),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::GzipHeader,
        })
    );

    let short = gzip::Codec {
        level: 1,
        shuffle: 1,
    }
    .encode(&[1, 2, 3], &mut scratch)
    .to_vec();
    assert_matches!(
        gzip::gunzip_into(&short, 4, &mut decoded, &mut inflate),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 4,
            got: 3
        })
    );

    let raw = [1, 2, 3, 4, 5, 6];
    let mut shuffled = Vec::new();
    gzip::shuffle_bytes_into(&raw, 2, &mut shuffled);
    assert_eq!(shuffled, [1, 3, 5, 2, 4, 6]);
    gzip::unshuffle_bytes_into(&shuffled, 2, &mut decoded);
    assert_eq!(decoded, raw);
    gzip::shuffle_bytes_into(&raw, 1, &mut shuffled);
    assert_eq!(shuffled, raw, "width-one shuffle is an exact no-op");
}

/// A member of `raw` whose header names a file, a comment and an extra field.
fn member_with_fields(raw: &[u8]) -> Vec<u8> {
    let mut encoder = GzBuilder::new()
        .filename("tile")
        .comment("a comment")
        .extra(vec![1, 2, 3])
        .write(Vec::new(), Compression::fast());
    encoder.write_all(raw).unwrap();
    encoder.finish().unwrap()
}

/// The header's every optional field is skipped, and a header CRC, which flate2 never
/// writes, is checked: its low 16 bits of the CRC-32 of the header before it.
#[test]
fn gunzip_reads_past_every_header_field_and_checks_the_header_crc() {
    let raw: Vec<u8> = (0..200u8).collect();
    let mut out = Vec::new();
    let mut inflate = Decompress::new(false);
    let fields = member_with_fields(&raw);
    gzip::gunzip_into(&fields, raw.len(), &mut out, &mut inflate).unwrap();
    assert_eq!(out, raw);

    // A plain 10-byte header, its FHCRC flag set and the CRC inserted after it.
    let plain = gzip::Codec {
        level: 1,
        shuffle: 1,
    }
    .encode(&raw, &mut gzip::GzipScratch::default())
    .to_vec();
    let mut header = plain[..10].to_vec();
    header[3] |= 0x02;
    let mut crc = Crc::new();
    crc.update(&header);
    let good = crc.sum() as u16;
    let with_crc = |crc: u16| [header.as_slice(), &crc.to_le_bytes(), &plain[10..]].concat();
    gzip::gunzip_into(&with_crc(good), raw.len(), &mut out, &mut inflate).unwrap();
    assert_eq!(out, raw);
    for member in [with_crc(good ^ 1), {
        let mut reserved = plain.clone();
        reserved[3] |= 0x20;
        reserved
    }] {
        assert_matches!(
            gzip::gunzip_into(&member, raw.len(), &mut out, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipHeader
            })
        );
    }
}

/// The trailer's CRC-32 and size are each checked, and a member cut short anywhere
/// past its header is truncated.
#[test]
fn gunzip_checks_the_trailer_and_refuses_a_cut_member() {
    let raw: Vec<u8> = (0..200u8).rev().collect();
    let member = gzip::Codec {
        level: 1,
        shuffle: 1,
    }
    .encode(&raw, &mut gzip::GzipScratch::default())
    .to_vec();
    let mut out = Vec::new();
    let mut inflate = Decompress::new(false);
    let trailer = member.len() - 8;
    for field in [trailer, trailer + 4] {
        let mut broken = member.clone();
        broken[field] ^= 0x01;
        assert_matches!(
            gzip::gunzip_into(&broken, raw.len(), &mut out, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipChecksum
            })
        );
    }
    for cut in [trailer + 7, trailer, 12] {
        assert_matches!(
            gzip::gunzip_into(&member[..cut], raw.len(), &mut out, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipTruncated
            })
        );
    }
    gzip::gunzip_into(&member, raw.len(), &mut out, &mut inflate).unwrap();
    assert_eq!(out, raw, "the state resets after each failure");
}

/// The kept deflate state gives the member flate2's `GzEncoder` writes, byte for byte:
/// at each level, for an empty input, a compressible one, and one of random bytes
/// whose deflate runs past the reserved bound; the scratch serves every case in
/// turn, a level change among them.
#[test]
fn encode_matches_gz_encoder_with_one_scratch() {
    let mut state = 0x9e37_79b9_u32;
    let random: Vec<u8> = (0..100_000)
        .map(|_| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            state.to_le_bytes()[0]
        })
        .collect();
    let ramp: Vec<u8> = (0..50_000u32).map(|i| (i / 64) as u8).collect();
    let mut scratch = gzip::GzipScratch::default();
    for level in [1, 6, 9, 1] {
        for raw in [&[][..], &ramp, &random] {
            let mut expected = Vec::new();
            let mut encoder = GzBuilder::new().write(&mut expected, Compression::new(level));
            encoder.write_all(raw).unwrap();
            encoder.finish().unwrap();
            let codec = gzip::Codec { level, shuffle: 1 };
            assert_eq!(
                codec.encode(raw, &mut scratch),
                expected,
                "level {level}, {} bytes",
                raw.len()
            );
        }
    }
}
