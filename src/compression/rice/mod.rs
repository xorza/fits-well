//! `RICE_1` tile codec (a port of cfitsio's `fits_rdecomp` bitstream layout).

use crate::compression::plane::IntBitpix;
use crate::fits_error::CompressedDataDefect;
use crate::fits_error::FitsError;
use crate::fits_error::Result;

/// The block size fits-well writes (§10.4.1 Table 37's default), in `ZVAL1` and in
/// every stream.
pub(super) const BLOCKSIZE: usize = 32;

/// The split-field width and the escape value of a pixel width (cfitsio
/// `fits_rcomp`): `fsbits` bits hold `fs + 1`, and `fs = fsmax` marks a block of
/// uncompressed differences.
const fn split_field(bytepix: IntBitpix) -> SplitField {
    match bytepix {
        IntBitpix::U8 => SplitField { bits: 3, max: 6 },
        IntBitpix::I16 => SplitField { bits: 4, max: 14 },
        IntBitpix::I32 => SplitField { bits: 5, max: 25 },
        IntBitpix::I64 => SplitField { bits: 6, max: 57 },
    }
}

#[derive(Debug, Clone, Copy)]
struct SplitField {
    bits: u32,
    max: u32,
}

/// A `RICE_1` stream's parameters: `blocksize` samples a block, each `bytepix` wide.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Codec {
    pub(super) blocksize: usize,
    pub(super) bytepix: IntBitpix,
}

impl Codec {
    /// Decode a tile of `count` integer values into `out` (cleared first; a reused
    /// buffer, so steady-state decode allocates nothing).
    // Out of `TileCodec::decode_ints`, so a change to another codec's arm cannot move this
    // loop: inlined, it ran 10 % slower after the gzip arm changed.
    #[inline(never)]
    pub(super) fn decode(self, bytes: &[u8], count: usize, out: &mut Vec<i64>) -> Result<()> {
        let Self { blocksize, bytepix } = self;
        out.clear();
        if count == 0 {
            return Ok(());
        }
        let nbits_pp = (8 * bytepix.elem_size()) as u32;
        let SplitField {
            bits: fsbits,
            max: fsmax,
        } = split_field(bytepix);
        let mask = if nbits_pp >= 64 {
            u64::MAX
        } else {
            (1u64 << nbits_pp) - 1
        };

        // `BITPIX = 8` is the one unsigned width (§4.4.1.2), as cfitsio's byte decoder
        // (`fits_rdecomp_byte`) returns it; the wider ones are two's complement.
        let signed = bytepix != IntBitpix::U8;
        let mut br = BitReader::new(bytes);
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
                    sign_extend(lastpix, nbits_pp)
                } else {
                    lastpix.cast_signed()
                });
            }
            i = imax;
        }
        Ok(())
    }

    /// Encode `values` as a tile (a port of cfitsio's `fits_rcomp`), and lend the stream
    /// from `scratch`. Differences are taken modulo the pixel width so the stream
    /// round-trips through [`Self::decode`].
    pub(super) fn encode<'s>(self, values: &[i64], scratch: &'s mut RiceScratch) -> &'s [u8] {
        let Self { blocksize, bytepix } = self;
        let RiceScratch { diffs, stream: out } = scratch;
        out.clear();
        let nbits = (8 * bytepix.elem_size()) as u32;
        let SplitField {
            bits: fsbits,
            max: fsmax,
        } = split_field(bytepix);
        let mask: u64 = if nbits >= 64 {
            u64::MAX
        } else {
            (1u64 << nbits) - 1
        };
        let mut bo = BitOutput::new(out);
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
                let s = sign_extend(raw, nbits);
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
                for &d in diffs.iter() {
                    bo.output_nbits(d, nbits);
                }
            } else if fs == 0 && pixelsum == 0 {
                bo.output_nbits(0, fsbits);
            } else {
                bo.output_nbits(u64::from(fs + 1), fsbits);
                let fsmask = (1u64 << fs) - 1;
                for &d in diffs.iter() {
                    bo.output_rice_value(d, fs, fsmask);
                }
            }
            i += thisblock;
        }
        bo.finish();
        out
    }
}

/// Interpret the low `nbits` of `v` as a two's-complement signed value.
const fn sign_extend(v: u64, nbits: u32) -> i64 {
    let shift = 64 - nbits;
    (v << shift).cast_signed() >> shift
}

/// The buffers a Rice encode reuses: the block's differences and the stream.
#[derive(Debug, Default)]
pub(super) struct RiceScratch {
    diffs: Vec<u64>,
    stream: Vec<u8>,
}

/// MSB-first bit output appended to a buffer, mirroring cfitsio's
/// `Buffer`/`output_nbits`: bits gather in a word and leave a byte at a time, and
/// [`BitOutput::finish`] pads the last byte with zero bits.
#[derive(Debug)]
struct BitOutput<'a> {
    out: &'a mut Vec<u8>,
    /// Pending bits, right-aligned: the low `pending` bits of `word`.
    word: u64,
    /// Fewer than 8 between calls, so a call may add up to 56 bits.
    pending: u32,
}

impl<'a> BitOutput<'a> {
    /// The most bits one [`BitOutput::push`] takes: the word holds at most 7 pending.
    const CHUNK: u32 = 56;

    const fn new(out: &'a mut Vec<u8>) -> Self {
        BitOutput {
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

/// A MSB-first bit reader over a compressed byte stream.
#[derive(Debug)]
struct BitReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        BitReader {
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

#[cfg(test)]
mod tests;
