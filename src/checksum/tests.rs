use crate::block::padded_len;
use crate::checksum::checksum_report::ChecksumStatus;
use crate::checksum::declared::Declared;
use crate::checksum::running::Running;
use crate::checksum::*;
use crate::header_model::keywords;
use crate::internals::cards;
use crate::internals::cards::VALID;
use crate::internals::fixture::Fixture;
use crate::internals::rng::SplitMix;

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
            let field = cards::quoted_field(&header, "CHECKSUM", 16);
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
fn running_sum_of_pieces_equals_the_whole() {
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

/// The status of each keyword value, over an empty data unit, whose sum is 0. An empty or
/// all-blank value is unknown (§4.4.2.7), as cfitsio `ffvcks` reads it. A `CHECKSUM` is
/// valid when the header sum completes the data sum to negative zero, `0xFFFF_FFFF`.
#[test]
fn declared_values_give_absent_unknown_valid_and_invalid() {
    use ChecksumStatus::{Absent, Invalid, Unknown, Valid};
    for (datasum, checksum, header_sum, expected) in [
        (None, None, 0, cards::report(Absent, Absent)),
        (Some(" "), None, 0, cards::report(Unknown, Absent)),
        (Some(""), None, 0, cards::report(Unknown, Absent)),
        (Some("0"), None, 0, cards::report(Valid, Absent)),
        (Some("1"), None, 0, cards::report(Invalid, Absent)),
        (
            Some("not-a-number"),
            None,
            0,
            cards::report(Invalid, Absent),
        ),
        (None, Some(" "), 0, cards::report(Absent, Unknown)),
        (None, Some(""), 0, cards::report(Absent, Unknown)),
        (
            None,
            Some("not-a-checksum"),
            0,
            cards::report(Absent, Invalid),
        ),
        (
            None,
            Some("not-a-checksum"),
            0xFFFF_FFFF,
            cards::report(Absent, Valid),
        ),
    ] {
        let mut header = cards::structural(None, 8, &[]);
        if let Some(value) = datasum {
            header.set_internal(keywords::DATASUM, value);
        }
        if let Some(value) = checksum {
            header.set_internal(keywords::CHECKSUM, value);
        }
        let report = Declared::read(&header)
            .report(header_sum, || Ok(0))
            .unwrap();
        assert_eq!(
            report, expected,
            "DATASUM={datasum:?} CHECKSUM={checksum:?}"
        );
    }
}

/// Of the 16 pairs, only (Valid, Valid) is valid, and the 7 with an `Invalid` side have
/// one.
#[test]
fn report_is_valid_only_when_both_sums_are() {
    let statuses = [
        ChecksumStatus::Absent,
        ChecksumStatus::Unknown,
        ChecksumStatus::Valid,
        ChecksumStatus::Invalid,
    ];
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for datasum in statuses {
        for checksum in statuses {
            let report = cards::report(datasum, checksum);
            if report.is_valid() {
                valid.push((datasum, checksum));
            }
            if report.has_invalid() {
                invalid.push((datasum, checksum));
            }
        }
    }
    assert_eq!(valid, [(ChecksumStatus::Valid, ChecksumStatus::Valid)]);
    assert_eq!(
        invalid,
        [
            (ChecksumStatus::Absent, ChecksumStatus::Invalid),
            (ChecksumStatus::Unknown, ChecksumStatus::Invalid),
            (ChecksumStatus::Valid, ChecksumStatus::Invalid),
            (ChecksumStatus::Invalid, ChecksumStatus::Absent),
            (ChecksumStatus::Invalid, ChecksumStatus::Unknown),
            (ChecksumStatus::Invalid, ChecksumStatus::Valid),
            (ChecksumStatus::Invalid, ChecksumStatus::Invalid),
        ]
    );
}

/// The deferred-carry sum equals the sum that folds each word's carry at once, over random
/// bytes and seeds, all-ones words that wrap on every add, and zeros.
#[test]
fn deferred_carries_give_the_word_by_word_sum() {
    let word_by_word = |bytes: &[u8], seed: u32| {
        bytes
            .as_chunks::<4>()
            .0
            .iter()
            .fold(seed, |sum, word| combine(sum, u32::from_be_bytes(*word)))
    };
    let mut rng = SplitMix::new(0xc4ec_5e5d);
    for round in 0..200 {
        let len = 4 * (rng.next() % 2000) as usize;
        let bytes: Vec<u8> = (0..len).map(|_| rng.next() as u8).collect();
        let seed = rng.next() as u32;
        assert_eq!(
            accumulate(&bytes, seed),
            word_by_word(&bytes, seed),
            "round {round}"
        );
    }
    for (bytes, seed) in [
        (vec![0xFF; 4096], 0xFFFF_FFFF),
        (vec![0xFF; 4096], 0),
        (vec![0; 4096], 0),
        (vec![0; 4096], 0xFFFF_FFFF),
        (vec![0xFF, 0xFF, 0xFF, 0xFE], 1),
    ] {
        assert_eq!(
            accumulate(&bytes, seed),
            word_by_word(&bytes, seed),
            "{seed:#x}"
        );
    }
}
