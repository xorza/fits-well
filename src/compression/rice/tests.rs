use crate::compression::plane::IntBitpix;
use crate::compression::rice::{self, BitReader};
use crate::fits_error::FitsError;
use crate::internals::rng::SplitMix;
use std::assert_matches;

/// The word-wide writer emits exactly the stream of a writer that sets one bit
/// at a time, over field widths from 0 to 64, zero runs past a word, and a
/// stream that ends mid-byte.
#[test]
fn bit_output_matches_a_bit_at_a_time_writer() {
    let mut draws = SplitMix::new(1);
    let mut next = || draws.next();
    let mut fields = Vec::new();
    for n in 0..=64 {
        fields.push((next(), n, 0));
    }
    fields.push((0, 0, 200));
    fields.push((next(), 3, 57));
    fields.push((next(), 5, 1));

    let mut expected = Vec::new();
    let mut used = 0usize;
    let mut set = |bit: bool| {
        if used.is_multiple_of(8) {
            expected.push(0u8);
        }
        if bit {
            *expected.last_mut().unwrap() |= 0x80 >> (used % 8);
        }
        used += 1;
    };
    let mut out = Vec::new();
    let mut bo = rice::BitOutput::new(&mut out);
    for &(bits, n, zeros) in &fields {
        for _ in 0..zeros {
            set(false);
        }
        for shift in (0..n).rev() {
            set((bits >> shift) & 1 == 1);
        }
        bo.output_zeros(zeros);
        bo.output_nbits(bits, n);
    }
    bo.finish();
    assert_eq!(out, expected);
}

#[test]
fn bit_reader_reads_msb_first() {
    let mut br = BitReader::new(&[0b1011_0010, 0b1111_0000]);
    assert_eq!(br.read(1).unwrap(), 1);
    assert_eq!(br.read(3).unwrap(), 0b011);
    assert_eq!(br.read(4).unwrap(), 0b0010);
    assert_eq!(br.read(4).unwrap(), 0b1111);
}

#[test]
fn read_zeros_counts_runs_across_bytes_and_leftover_bits() {
    // MSB-first. 0x00 0x80 = 0000_0000 1000_0000: an 8-bit zero run spanning the
    // first byte, terminated by the leading 1 of the second (exercises the
    // cross-byte refill mid-run). Consumes 9 bits, leaving 7 unterminated zeros.
    let mut br = BitReader::new(&[0x00, 0x80]);
    assert_eq!(br.read_zeros().unwrap(), 8);
    assert_matches!(br.read_zeros(), Err(FitsError::UnexpectedEof));

    // 0x01 = 0000_0001: 7 zeros then a 1, entirely within one byte.
    let mut br = BitReader::new(&[0x01]);
    assert_eq!(br.read_zeros().unwrap(), 7);

    // Leftover bits before a run: read(4) leaves 4 valid bits, then read_zeros
    // works from them. 0x08 = 0000_1000 → high nibble 0, then the '1' is next
    // (run 0); 0x40 = 0100_0000 → 3 trailing zeros of byte0 + 1 zero → run 4.
    let mut br = BitReader::new(&[0x08, 0x40]);
    assert_eq!(br.read(4).unwrap(), 0);
    assert_eq!(br.read_zeros().unwrap(), 0);
    assert_eq!(br.read_zeros().unwrap(), 4);
}

#[test]
fn bytepix_8_round_trips_unaligned_multiblock_extremes() {
    let values: Vec<i64> = (0..70)
        .map(|index| match index % 7 {
            0 => i64::MIN,
            1 => i64::MAX,
            2 => 9_007_199_254_740_993 + index,
            3 => -9_007_199_254_740_993 - index,
            4 => index,
            5 => -index,
            _ => 0,
        })
        .collect();
    let mut scratch = rice::RiceScratch::default();
    let encoded = rice::Codec {
        blocksize: 32,
        bytepix: IntBitpix::I64,
    }
    .encode(&values, &mut scratch)
    .to_vec();
    let mut decoded = Vec::new();
    rice::Codec {
        blocksize: 32,
        bytepix: IntBitpix::I64,
    }
    .decode(&encoded, values.len(), &mut decoded)
    .unwrap();
    assert_eq!(decoded, values);
}

#[test]
fn odd_final_block_uses_the_canonical_integer_statistic() {
    let mut scratch = rice::RiceScratch::default();
    let encoded = rice::Codec {
        blocksize: 32,
        bytepix: IntBitpix::I32,
    }
    .encode(&[0, 0, 4], &mut scratch)
    .to_vec();
    // CFITSIO's reference encoder selects split 1 and emits these exact bytes.
    assert_eq!(encoded, [0, 0, 0, 0, 0x15, 0x04]);

    let mut decoded = Vec::new();
    rice::Codec {
        blocksize: 32,
        bytepix: IntBitpix::I32,
    }
    .decode(&encoded, 3, &mut decoded)
    .unwrap();
    assert_eq!(decoded, [0, 0, 4]);
}

#[test]
fn truncated_stream_is_refused() {
    // A stream that enters a Rice zero-run (fs = 0) but ends before the
    // terminating 1-bit. byte0 is the literal first pixel; byte1 = 0b001_00000
    // gives the 3-bit fs field `001` (→ fs = 0), after which only zero bits
    // remain. The unary code has no terminating 1 and must report EOF.
    let mut out = Vec::new();
    assert_matches!(
        rice::Codec {
            blocksize: 32,
            bytepix: IntBitpix::U8
        }
        .decode(&[0x00, 0x20], 2, &mut out),
        Err(FitsError::UnexpectedEof)
    );
}
