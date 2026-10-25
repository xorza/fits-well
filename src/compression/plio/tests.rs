use crate::compression::plane::IntSamples;
use crate::compression::plio::{self, Words};
use crate::internals::rng::SplitMix;
use crate::internals::samples;
use std::assert_matches;

use crate::fits_error::{CompressedDataDefect, CompressionFeature, FitsError, PlioDefect};

/// The two header words hold 15 bits each, low first: 2³⁰ − 1 is the longest
/// list they can state.
#[test]
fn list_length_splits_into_two_fifteen_bit_words() {
    assert_eq!(plio::length_words(0).unwrap(), [0, 0]);
    assert_eq!(plio::length_words(32_768 + 5).unwrap(), [5, 1]);
    assert_eq!(plio::length_words((1 << 30) - 1).unwrap(), [32_767, 32_767]);
    assert_matches!(
        plio::length_words(1 << 30),
        Err(FitsError::UnsupportedCompression {
            // 2³⁰ words, one past the longest list two 15-bit words state.
            feature: CompressionFeature::PlioListLength {
                words: 1_073_741_824
            }
        })
    );
}

#[test]
fn validates_encoding_and_refuses_malformed_streams() {
    let mut out = Vec::new();

    let mut encoded = Vec::new();
    plio::Codec { offset: 0 }
        .encode(&[0, 0xFF_FFFF], &mut encoded)
        .unwrap();
    plio::Codec { offset: 0 }
        .decode(&encoded, 2, &mut out)
        .unwrap();
    assert_eq!(out, [0, 0xFF_FFFF]);
    // The offset joins the value before the range check and leaves it again.
    encoded.clear();
    plio::Codec { offset: 32768 }
        .encode(&[-32768, 32767], &mut encoded)
        .unwrap();
    plio::Codec { offset: 32768 }
        .decode(&encoded, 2, &mut out)
        .unwrap();
    assert_eq!(out, [-32768, 32767]);
    // The check names the stored sample, the offset left out: −32769 plus 32768 is −1.
    for (offset, invalid) in [(0, -1), (0, 0x100_0000), (32768, -32769)] {
        assert_matches!(
            plio::Codec { offset }.check(IntSamples::I64(&[0, invalid])),
            Err(FitsError::PlioValueOutOfRange {
                index: 1,
                value,
            }) if value == invalid
        );
    }
    plio::Codec { offset: 32768 }
        .check(IntSamples::I16(&[i16::MIN, i16::MAX]))
        .unwrap();
    // A type the stream holds whole passes unread; a type it holds in part is read.
    plio::Codec { offset: 0 }
        .check(IntSamples::U8(&[0, u8::MAX]))
        .unwrap();
    assert_matches!(
        plio::Codec { offset: 0 }.check(IntSamples::I16(&[0, 5, -1])),
        Err(FitsError::PlioValueOutOfRange {
            index: 2,
            value: -1
        })
    );
    assert_matches!(
        plio::Codec { offset: 0xFF_FF01 }.check(IntSamples::U8(&[0, 254, 255])),
        Err(FitsError::PlioValueOutOfRange {
            index: 2,
            value: 255
        })
    );
    let truncated_span = [0, 7, -100, 8, 0, 0, 0];
    assert_matches!(
        plio::Codec { offset: 0 }.decode(
            &samples::be(&truncated_span, i16::to_be_bytes),
            1,
            &mut out
        ),
        Err(FitsError::UnexpectedEof)
    );

    let missing_absolute_high_word = [0, 0, 4, 0x1001];
    assert_matches!(
        plio::Codec { offset: 0 }.decode(
            &samples::be(&missing_absolute_high_word, i16::to_be_bytes),
            1,
            &mut out
        ),
        Err(FitsError::UnexpectedEof)
    );

    let invalid_opcode = [0, 0, 4, i16::MIN];
    assert_matches!(
        plio::Codec { offset: 0 }.decode(
            &samples::be(&invalid_opcode, i16::to_be_bytes),
            1,
            &mut out
        ),
        Err(FitsError::CorruptCompressedData {
            defect: CompressedDataDefect::Plio(PlioDefect::Opcode),
        })
    );
}

/// The encoder before runs were passed whole: a port of cfitsio's `pl_p2li` that
/// steps one pixel at a time.
fn reference_encode(offset: i64, values: &[i64]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut ll = Words::new(&mut out);
    for word in [0, 7, -100, 0, 0, 0, 0] {
        ll.push(word);
    }
    let npix = values.len();
    if npix > 0 {
        let pix = |i1: usize| values[i1 - 1] + offset;

        let xs = 1usize;
        let xe = xs + npix - 1;
        let mut pv = pix(xs);
        let mut x1 = xs as i64;
        let mut iz = xs as i64;
        let mut hi: i64 = 1;
        let mut nv: i64 = 0;

        for ip in xs..=xe {
            let mut flush = true;
            if ip < xe {
                nv = pix(ip + 1);
                if nv == pv {
                    flush = false; // extend the current run
                } else if pv == 0 {
                    pv = nv;
                    x1 = (ip + 1) as i64;
                    flush = false;
                }
            } else if pv == 0 {
                x1 = xe as i64 + 1;
            }

            if flush {
                let mut np = ip as i64 - x1 + 1; // high-pixel count in this segment
                let mut nz = x1 - iz; // leading zero count
                let mut done = false;

                if pv > 0 {
                    let dv = pv - hi; // change in the "high" value since last segment
                    if dv != 0 {
                        hi = pv;
                        if dv.abs() > 4095 {
                            // Two-word absolute set: low 12 bits (opcode 1) then high bits.
                            ll.push(((pv & 4095) + 4096) as i16);
                            ll.push((pv / 4096) as i16);
                        } else {
                            // One-word relative adjust: opcode 2 (up) or 3 (down).
                            ll.push(if dv < 0 {
                                (-dv + 12288) as i16
                            } else {
                                (dv + 8192) as i16
                            });
                            // A lone high pixel with no leading zeros folds into the
                            // adjust word as opcode 6/7 (single high pixel).
                            if np == 1 && nz == 0 {
                                ll.update_last(|last| last | 16384);
                                done = true;
                            }
                        }
                    }
                }

                if !done && nz > 0 {
                    while nz > 0 {
                        ll.push(nz.min(4095) as i16);
                        nz -= 4095;
                    }
                    // A lone high pixel after the zeros folds into the last zero word
                    // as opcode 5 (zero run whose final pixel is set high).
                    if np == 1 && pv > 0 {
                        ll.update_last(|last| (i64::from(last) + 20481) as i16);
                        done = true;
                    }
                }

                if !done {
                    while np > 0 {
                        ll.push((np.min(4095) + 16384) as i16); // opcode 4: high run
                        np -= 4095;
                    }
                }

                x1 = (ip + 1) as i64;
                iz = x1;
                pv = nv;
            }
        }
    }
    let [low, high] = plio::length_words(ll.len()).unwrap();
    ll.set(3, low);
    ll.set(4, high);
    out
}

/// The encoder writes the reference's list word for word, for random masks of runs of
/// every length: short ones, runs about a 16-value chunk long, runs past the 4095 one
/// word holds, and steps past 4095 that take the two-word set.
#[test]
fn encode_matches_the_reference_encode() {
    let mut rng = SplitMix::new(0x0091_10e5);
    for round in 0..300 {
        let offset = if round % 3 == 0 { 32768 } else { 0 };
        let mut values = Vec::new();
        let target = (rng.next() % 12_000) as usize;
        while values.len() < target {
            let len = match rng.next() % 4 {
                0 => 1 + rng.next() % 4,
                1 => 12 + rng.next() % 24,
                2 => 4090 + rng.next() % 12,
                _ => 1 + rng.next() % 300,
            } as usize;
            let value = match rng.next() % 4 {
                0 | 1 => 0,
                2 => (rng.next() % 8) as i64,
                _ => (rng.next() % 0x100_0000) as i64,
            } - offset;
            values.extend(std::iter::repeat_n(value, len));
        }
        let mut encoded = Vec::new();
        plio::Codec { offset }
            .encode(&values, &mut encoded)
            .unwrap();
        assert_eq!(encoded, reference_encode(offset, &values), "round {round}");
    }
}
