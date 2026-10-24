use crate::block::padded_len;
use crate::checksum::running::Running;
use crate::checksum::*;
use crate::header_model::keywords;
use crate::internals::cards::VALID;
use crate::internals::fixture::Fixture;

#[test]
fn accumulate_folds_end_around_carry() {
    // Two words summing past 2^32 wrap around (ones'-complement).
    let bytes = [0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x02];
    // 0xFFFFFFFF + 0x00000002 = 0x100000001 → fold → 0x00000002.
    assert_eq!(accumulate(&bytes, 0), 0x0000_0002);
    assert_eq!(accumulate(&[0, 0, 0, 1], 0), 1);

    // Carry is folded after every word, so repeated maximal words never build
    // an overflow-prone wider accumulator.
    let mut sum = 0;
    for _ in 0..1_000_000 {
        sum = combine(sum, u32::MAX);
    }
    assert_eq!(sum, u32::MAX);

    let first = [0xFF, 0xFF, 0xFF, 0xFE, 0, 0, 0, 5];
    let second = [0x80, 0, 0, 0, 0x80, 0, 0, 1];
    let mut joined = first.to_vec();
    joined.extend_from_slice(&second);
    assert_eq!(
        combine(accumulate(&first, 0), accumulate(&second, 0)),
        accumulate(&joined, 0)
    );
}

/// Appendix J.2: the value starts at byte 12 of its card, one byte past a word
/// boundary. Summing an HDU with sixteen `'0'`s there, then writing the encoded
/// complement over them, brings the whole-HDU sum to negative zero.
#[test]
fn encoded_checksum_is_alphanumeric_and_sums_to_negative_zero() {
    for rest in [
        [0u8; 8],
        [0xFF; 8],
        [0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0],
    ] {
        let mut hdu = rest.to_vec();
        // Byte 12 of a word-aligned card is offset 11, three past a boundary.
        let field = hdu.len() + 3;
        hdu.resize(hdu.len() + 20, b' ');
        hdu[field..field + 16].copy_from_slice(b"0000000000000000");
        let encoded = encode(accumulate(&hdu, 0));
        assert!(
            encoded.iter().all(|&b| b.is_ascii_alphanumeric()),
            "non-alphanumeric output: {encoded:?}"
        );
        hdu[field..field + 16].copy_from_slice(&encoded);
        assert_eq!(accumulate(&hdu, 0), 0xFFFF_FFFF, "{rest:?}");
    }
}

/// cfitsio wrote both HDUs' `CHECKSUM` and `DATASUM` in these files, so they
/// are answers from an independent implementation: each verifies, the data's
/// sum is the `DATASUM`, and encoding the HDU's sum with the `CHECKSUM` field
/// zeroed reproduces cfitsio's string.
#[test]
fn encoding_reproduces_cfitsio_checksums() {
    for name in [Fixture::CompTableCfitsio, Fixture::CompTableVla] {
        let bytes = name.bytes();
        let mut reader = name.stream();
        let mut header_start = 0;
        for index in 0..reader.hdus().len() {
            assert_eq!(
                reader.verify_checksum(index).unwrap(),
                VALID,
                "{name:?} HDU {index}"
            );
            let hdu = &reader.hdus()[index];
            let data_start = hdu.data_offset as usize;
            let data_end = data_start + padded_len(hdu.data_bytes).unwrap() as usize;
            let data_sum = accumulate(&bytes[data_start..data_end], 0);
            // cfitsio right-justifies the number in ten characters.
            let datasum = hdu.header.get_text(keywords::DATASUM).unwrap().unwrap();
            assert_eq!(datasum.trim_start().parse::<u32>(), Ok(data_sum));

            let mut header = bytes[header_start..data_start].to_vec();
            let card = header
                .as_chunks::<80>()
                .0
                .iter()
                .position(|card| card.starts_with(b"CHECKSUM= '"))
                .unwrap();
            let field = card * 80 + 11..card * 80 + 27;
            let written: [u8; 16] = header[field.clone()].try_into().unwrap();
            header[field].copy_from_slice(b"0000000000000000");
            let sum = combine(accumulate(&header, 0), data_sum);
            assert_eq!(
                encode(sum),
                written,
                "{name:?} HDU {index}: cfitsio wrote {}",
                String::from_utf8_lossy(&written)
            );
            header_start = data_end;
        }
    }
}

/// A running sum fed in pieces of any length, words split between them, equals the sum
/// of the whole: 4 KiB of bytes cut at a seeded set of points.
#[test]
fn a_running_sum_of_pieces_equals_the_whole() {
    let bytes: Vec<u8> = (0u32..4096)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 24) as u8)
        .collect();
    let whole = accumulate(&bytes, 0);
    for step in [1, 2, 3, 5, 7, 4093] {
        let mut sum = Running::new();
        for piece in bytes.chunks(step) {
            sum.update(piece);
        }
        assert_eq!(sum.finish(), whole, "pieces of {step}");
    }
}
