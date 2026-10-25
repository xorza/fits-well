use crate::compression::plane::IntBitpix;
use crate::compression::rice::{self, BitReader, SplitField};
use crate::fits_error::{CompressedDataDefect, FitsError, Result};
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

/// The bit reader the decode used before its refill-once reader, kept to cross-check it.
#[derive(Debug)]
struct ReferenceReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> ReferenceReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        ReferenceReader {
            bytes,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    /// Read `n` bits (MSB-first, `n ≤ 64`).
    fn read(&mut self, n: u32) -> Result<u64> {
        if n > 64 {
            return Err(CompressedDataDefect::RiceBitField { bits: n }.into());
        }
        let mut remaining = n;
        let mut value = 0u64;
        while remaining != 0 {
            if self.nbits == 0 {
                self.fill(1)?;
            }
            let take = remaining.min(self.nbits);
            self.nbits -= take;
            let mask = if take == 64 {
                u64::MAX
            } else {
                (1u64 << take) - 1
            };
            let part = (self.acc >> self.nbits) & mask;
            value = if take == 64 {
                part
            } else {
                (value << take) | part
            };
            remaining -= take;
        }
        Ok(value)
    }

    /// Top up the accumulator to `needed` bits, loading a whole word in the common
    /// aligned case.
    #[inline]
    fn fill(&mut self, needed: u32) -> Result<()> {
        if self.nbits == 0 && self.pos + 8 <= self.bytes.len() {
            let word = self.bytes[self.pos..self.pos + 8].try_into().unwrap();
            self.acc = u64::from_be_bytes(word);
            self.pos += 8;
            self.nbits = 64;
            return Ok(());
        }
        while self.nbits < needed {
            let byte = self
                .bytes
                .get(self.pos)
                .copied()
                .ok_or(FitsError::UnexpectedEof)?;
            self.pos += 1;
            self.acc = (self.acc << 8) | u64::from(byte);
            self.nbits += 8;
        }
        Ok(())
    }

    /// Count and consume leading zero bits up to (and including) the next 1.
    ///
    /// Scans the zero run a whole word at a time via `leading_zeros` rather than one
    /// `read(1)` per bit — the unary quotient decode is the hot path of Rice decode.
    fn read_zeros(&mut self) -> Result<u64> {
        let mut z = 0u64;
        loop {
            if self.nbits == 0 {
                self.fill(1)?;
            }
            // Left-align the valid low `nbits` bits so the next-to-read bit is the
            // MSB, then count zeros up to the first 1 (capped at the valid bits, since
            // the shifted-in low bits read as zero).
            let run = (self.acc << (64 - self.nbits))
                .leading_zeros()
                .min(self.nbits);
            if run < self.nbits {
                // Terminating 1 found within the valid bits: consume the zeros + the 1.
                self.nbits -= run + 1;
                return Ok(z + u64::from(run));
            }
            // All valid bits were zero: consume them and refill on the next pass.
            z += u64::from(self.nbits);
            self.nbits = 0;
        }
    }
}

/// The decode before its refill-once reader, kept to cross-check it.
fn reference_decode(
    codec: rice::Codec,
    bytes: &[u8],
    count: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    let rice::Codec { blocksize, bytepix } = codec;
    out.clear();
    if count == 0 {
        return Ok(());
    }
    let nbits_pp = (8 * bytepix.elem_size()) as u32;
    let SplitField {
        bits: fsbits,
        max: fsmax,
    } = rice::split_field(bytepix);
    let mask = if nbits_pp >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits_pp) - 1
    };

    // `BITPIX = 8` is the one unsigned width (§4.4.1.2), as cfitsio's byte decoder
    // (`fits_rdecomp_byte`) returns it; the wider ones are two's complement.
    let signed = bytepix != IntBitpix::U8;
    let mut br = ReferenceReader::new(bytes);
    let mut lastpix = br.read(nbits_pp)?; // literal first pixel (big-endian)
    out.reserve_exact(count);
    let mut i = 0;
    while i < count {
        // The block's code is its split + 1, so 0 marks an all-zero block; a field
        // of `fsbits` bits keeps the split below 64.
        let split = br.read(fsbits)?.checked_sub(1);
        let imax = (i + blocksize).min(count);
        for _ in i..imax {
            let diff = match split {
                None => 0,
                Some(fs) if fs == u64::from(fsmax) => br.read(nbits_pp)?, // uncompressed block
                Some(fs) => (br.read_zeros()? << fs) | br.read(fs as u32)?,
            };
            // Undo the zigzag mapping, then the differencing (modular at pixel width).
            let d = if diff & 1 == 1 {
                !(diff >> 1)
            } else {
                diff >> 1
            };
            lastpix = lastpix.wrapping_add(d) & mask;
            out.push(if signed {
                rice::sign_extend(lastpix, nbits_pp)
            } else {
                lastpix.cast_signed()
            });
        }
        i = imax;
    }
    Ok(())
}

/// The decode gives the reference decode's samples, or its failure, for random streams of
/// every width and length: valid ones from the encoder, and the same cut short or with a
/// flipped byte, which take every early end and malformed block.
#[test]
fn decode_matches_the_reference_decode() {
    let mut rng = SplitMix::new(0x5eed_41ce);
    let widths = [
        IntBitpix::U8,
        IntBitpix::I16,
        IntBitpix::I32,
        IntBitpix::I64,
    ];
    for round in 0..400 {
        let bytepix = widths[round % widths.len()];
        let codec = rice::Codec {
            blocksize: rice::BLOCKSIZE,
            bytepix,
        };
        let count = 1 + (rng.next() % 300) as usize;
        let bits = 8 * bytepix.elem_size() as u32;
        // A smooth ramp with noise of a random spread: blocks of every split, the
        // all-zero and the uncompressed ones among them.
        let spread = rng.next() % u64::from(bits);
        let values: Vec<i64> = (0..count)
            .map(|i| {
                let noise = rng.next() >> (63 - spread);
                rice::sign_extend((i as u64).wrapping_mul(3).wrapping_add(noise), bits)
            })
            .map(|value| {
                if bytepix == IntBitpix::U8 {
                    value & 0xFF
                } else {
                    value
                }
            })
            .collect();
        let mut scratch = rice::RiceScratch::default();
        let stream = codec.encode(&values, &mut scratch).to_vec();
        let mut variants = vec![stream.clone()];
        variants.push(stream[..stream.len() * 2 / 3].to_vec());
        let mut flipped = stream.clone();
        let at = (rng.next() as usize) % flipped.len();
        flipped[at] ^= 1 << (rng.next() % 8);
        variants.push(flipped);
        for (variant, bytes) in variants.iter().enumerate() {
            let (mut new, mut old) = (Vec::new(), Vec::new());
            let new_result = codec.decode(bytes, count, &mut new);
            let old_result = reference_decode(codec, bytes, count, &mut old);
            assert_eq!(
                format!("{new_result:?}"),
                format!("{old_result:?}"),
                "round {round}, variant {variant}"
            );
            if new_result.is_ok() {
                assert_eq!(new, old, "round {round}, variant {variant}");
                if variant == 0 {
                    assert_eq!(new, values, "round {round}");
                }
            }
        }
    }
}

/// The bit writer the encode used before it flushed 32 bits at a time, kept to cross-check it.
#[derive(Debug)]
struct ReferenceOutput<'a> {
    out: &'a mut Vec<u8>,
    /// Pending bits, right-aligned: the low `pending` bits of `word`.
    word: u64,
    /// Fewer than 8 between calls, so a call may add up to 56 bits.
    pending: u32,
}

impl<'a> ReferenceOutput<'a> {
    /// The most bits one [`ReferenceOutput::push`] takes: the word holds at most 7 pending.
    const CHUNK: u32 = 56;

    const fn new(out: &'a mut Vec<u8>) -> Self {
        ReferenceOutput {
            out,
            word: 0,
            pending: 0,
        }
    }

    /// Append the low `n` bits of `bits` (`n ≤ 64`), most significant first.
    fn output_nbits(&mut self, bits: u64, n: u32) {
        if n > Self::CHUNK {
            self.push(bits >> Self::CHUNK, n - Self::CHUNK);
            self.push(bits, Self::CHUNK);
        } else {
            self.push(bits, n);
        }
    }

    fn output_zeros(&mut self, mut n: u64) {
        while n > 0 {
            let chunk = n.min(u64::from(Self::CHUNK));
            self.push(0, chunk as u32);
            n -= chunk;
        }
    }

    fn push(&mut self, bits: u64, n: u32) {
        debug_assert!(n <= Self::CHUNK && self.pending < 8);
        if n == 0 {
            return;
        }
        self.word = (self.word << n) | (bits & (u64::MAX >> (64 - n)));
        self.pending += n;
        while self.pending >= 8 {
            self.pending -= 8;
            self.out.push((self.word >> self.pending) as u8);
        }
    }

    /// Output one Rice-coded value: `top = v >> fs` zero bits, a 1, then the low
    /// `fs` bits of `v`.
    fn output_rice_value(&mut self, v: u64, fs: u32, fsmask: u64) {
        self.output_zeros(v >> fs);
        self.push(1, 1);
        self.output_nbits(v & fsmask, fs);
    }

    /// Flush the last partial byte, its unused low bits zero.
    fn finish(self) {
        if self.pending > 0 {
            self.out.push((self.word << (8 - self.pending)) as u8);
        }
    }
}

/// The encode before it flushed 32 bits at a time, kept to cross-check it.
fn reference_encode(codec: rice::Codec, values: &[i64]) -> Vec<u8> {
    let rice::Codec { blocksize, bytepix } = codec;
    let (mut diffs, mut stream) = (Vec::new(), Vec::new());
    let out = &mut stream;
    out.clear();
    let nbits = (8 * bytepix.elem_size()) as u32;
    let SplitField {
        bits: fsbits,
        max: fsmax,
    } = rice::split_field(bytepix);
    let mask: u64 = if nbits >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits) - 1
    };
    let mut bo = ReferenceOutput::new(out);
    // Rice output is at most a few bytes per pixel; reserve a pixel's worth up front
    // so the bitstream rarely reallocates mid-tile.
    bo.out.reserve(values.len());
    let first = values.first().copied().unwrap_or(0).cast_unsigned() & mask;
    bo.output_nbits(first, nbits);
    let mut lastpix = first;

    diffs.reserve(blocksize);
    let mut i = 0;
    while i < values.len() {
        let thisblock = blocksize.min(values.len() - i);
        diffs.clear();
        let mut pixelsum = 0u128;
        for j in 0..thisblock {
            let next = values[i + j].cast_unsigned() & mask;
            // signed difference reduced to the pixel width, then zigzag-mapped
            let raw = next.wrapping_sub(lastpix) & mask;
            let s = rice::sign_extend(raw, nbits);
            let d = if s >= 0 {
                s.unsigned_abs() << 1
            } else {
                (s.unsigned_abs() << 1).wrapping_sub(1)
            };
            diffs.push(d);
            pixelsum += u128::from(d);
            lastpix = next;
        }

        let block = thisblock as u128;
        let quotient = pixelsum / block;
        let remainder = pixelsum % block;
        let dpsum = if quotient > 0 && remainder < block / 2 + 1 {
            quotient - 1
        } else {
            quotient
        };
        let mut psum = dpsum >> 1;
        let mut fs = 0u32;
        while psum > 0 {
            fs += 1;
            psum >>= 1;
        }

        if fs >= fsmax {
            bo.output_nbits(u64::from(fsmax + 1), fsbits);
            for &d in &diffs {
                bo.output_nbits(d, nbits);
            }
        } else if fs == 0 && pixelsum == 0 {
            bo.output_nbits(0, fsbits);
        } else {
            bo.output_nbits(u64::from(fs + 1), fsbits);
            let fsmask = (1u64 << fs) - 1;
            for &d in &diffs {
                bo.output_rice_value(d, fs, fsmask);
            }
        }
        i += thisblock;
    }
    bo.finish();
    stream
}

/// The encode writes the reference encode's stream byte for byte, for random samples of
/// every width, length and spread: blocks of every split, the all-zero and the
/// uncompressed ones among them.
#[test]
fn encode_matches_the_reference_encode() {
    let mut rng = SplitMix::new(0x0e5c_0de5);
    let widths = [
        IntBitpix::U8,
        IntBitpix::I16,
        IntBitpix::I32,
        IntBitpix::I64,
    ];
    for round in 0..400 {
        let bytepix = widths[round % widths.len()];
        let codec = rice::Codec {
            blocksize: rice::BLOCKSIZE,
            bytepix,
        };
        let count = (rng.next() % 300) as usize;
        let bits = 8 * bytepix.elem_size() as u32;
        let spread = rng.next() % u64::from(bits);
        let values: Vec<i64> = (0..count)
            .map(|i| {
                let noise = rng.next() >> (63 - spread);
                rice::sign_extend((i as u64).wrapping_mul(3).wrapping_add(noise), bits)
            })
            .collect();
        let mut scratch = rice::RiceScratch::default();
        assert_eq!(
            codec.encode(&values, &mut scratch),
            reference_encode(codec, &values),
            "round {round}"
        );
    }
}
