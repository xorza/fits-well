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
        let output = |pixel: u64| {
            if signed {
                sign_extend(pixel, nbits_pp)
            } else {
                pixel.cast_signed()
            }
        };
        // Undo the zigzag mapping, then the differencing (modular at pixel width).
        let next = |lastpix: u64, diff: u64| {
            let d = if diff & 1 == 1 {
                !(diff >> 1)
            } else {
                diff >> 1
            };
            lastpix.wrapping_add(d) & mask
        };
        // The output is sized once and filled a block at a time, with no length to store
        // and no capacity to check a sample.
        out.resize(count, 0);
        for block in out.chunks_mut(blocksize) {
            // The block's code is its split + 1, so 0 marks an all-zero block; a field
            // of `fsbits` bits keeps the split below 64.
            let split = br.read(fsbits)?.checked_sub(1);
            match split {
                None => block.fill(output(lastpix)),
                Some(fs) if fs == u64::from(fsmax) => {
                    for slot in block {
                        lastpix = next(lastpix, br.read(nbits_pp)?);
                        *slot = output(lastpix);
                    }
                }
                Some(fs) => {
                    let fs = fs as u32;
                    for slot in block {
                        let diff = (br.read_zeros()? << fs) | br.read(fs)?;
                        lastpix = next(lastpix, diff);
                        *slot = output(lastpix);
                    }
                }
            }
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

            let dpsum = mean_less_one(pixelsum, thisblock);
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

/// The block's mean difference, less one unless the remainder is past half the block, as
/// cfitsio rounds it. A sum that fits `u64`, as every width to 32 bits gives, divides there,
/// not through the 128-bit division's library call.
fn mean_less_one(pixelsum: u128, block: usize) -> u128 {
    let (quotient, remainder, block) = if let Ok(sum) = u64::try_from(pixelsum) {
        let block = block as u64;
        (
            u128::from(sum / block),
            u128::from(sum % block),
            u128::from(block),
        )
    } else {
        let block = block as u128;
        (pixelsum / block, pixelsum % block, block)
    };
    if quotient > 0 && remainder < block / 2 + 1 {
        quotient - 1
    } else {
        quotient
    }
}

/// A MSB-first bit writer: the pending bits sit at the top of `acc`, fewer than 32 of them
/// between calls, and leave 32 at a time.
#[derive(Debug)]
struct BitOutput<'a> {
    out: &'a mut Vec<u8>,
    acc: u64,
    pending: u32,
}

impl<'a> BitOutput<'a> {
    const fn new(out: &'a mut Vec<u8>) -> Self {
        BitOutput {
            out,
            acc: 0,
            pending: 0,
        }
    }

    /// Append the low `n` bits of `bits` (`n ≤ 64`), most significant first.
    fn output_nbits(&mut self, bits: u64, n: u32) {
        if n > 32 {
            self.put((bits >> 32) & low_bits(n - 32), n - 32);
            self.put(bits & low_bits(32), 32);
        } else {
            self.put(bits & low_bits(n), n);
        }
    }

    /// Append `bits`, which hold `n ≤ 32` bits.
    #[inline]
    fn put(&mut self, bits: u64, n: u32) {
        debug_assert!(n <= 32 && self.pending < 32 && bits >> n == 0);
        if n == 0 {
            return;
        }
        self.acc |= bits << (64 - self.pending - n);
        self.pending += n;
        if self.pending >= 32 {
            self.out
                .extend_from_slice(&((self.acc >> 32) as u32).to_be_bytes());
            self.acc <<= 32;
            self.pending -= 32;
        }
    }

    fn output_zeros(&mut self, mut n: u64) {
        while n > 0 {
            let chunk = n.min(32);
            self.put(0, chunk as u32);
            n -= chunk;
        }
    }

    /// Output one Rice-coded value: `top = v >> fs` zero bits, a 1, then the low
    /// `fs` bits of `v`, in one put where they fit 32 bits.
    fn output_rice_value(&mut self, v: u64, fs: u32, fsmask: u64) {
        let top = v >> fs;
        if top + 1 + u64::from(fs) <= 32 {
            self.put((1 << fs) | (v & fsmask), top as u32 + 1 + fs);
            return;
        }
        self.output_zeros(top);
        self.put(1, 1);
        self.output_nbits(v & fsmask, fs);
    }

    /// Flush the last bits, the partial byte's unused low bits zero.
    fn finish(self) {
        let bytes = self.pending.div_ceil(8) as usize;
        self.out.extend_from_slice(&self.acc.to_be_bytes()[..bytes]);
    }
}

/// The low `n ≤ 64` bits set.
const fn low_bits(n: u32) -> u64 {
    if n == 64 { u64::MAX } else { (1 << n) - 1 }
}

/// A MSB-first bit reader over a compressed byte stream. The next bit is the top bit of
/// `acc`, which holds `nbits` of them; the bits below are zeros or the stream's next bits,
/// which the next refill writes again.
#[derive(Debug)]
struct BitReader<'a> {
    bytes: &'a [u8],
    /// The byte the next refill loads from: the stream bit `8·pos − nbits` is the next.
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

    /// Top up the accumulator to at least 56 bits, or to every bit the stream has left:
    /// one unaligned load where 8 bytes remain, as Fabian Giesen's "variant 4" refill.
    #[inline]
    fn refill(&mut self) {
        debug_assert!(self.nbits < 64, "a refill below a full accumulator");
        if let Some(word) = self.bytes.get(self.pos..self.pos + 8) {
            self.acc |= u64::from_be_bytes(word.try_into().unwrap()) >> self.nbits;
            let added = (63 - self.nbits) / 8;
            self.pos += added as usize;
            self.nbits += 8 * added;
            return;
        }
        while self.nbits <= 56 {
            let Some(&byte) = self.bytes.get(self.pos) else {
                return;
            };
            self.acc |= u64::from(byte) << (56 - self.nbits);
            self.pos += 1;
            self.nbits += 8;
        }
    }

    /// Read `n` bits (MSB-first, `n ≤ 64`).
    #[inline]
    fn read(&mut self, n: u32) -> Result<u64> {
        if n <= 56 {
            self.read_short(n)
        } else {
            self.read_wide(n)
        }
    }

    /// Read `n ≤ 56` bits, which one refill holds. `(acc >> 1) >> (63 − n)` is the top `n`
    /// bits, 0 for `n = 0`, with no shift by 64.
    #[inline]
    fn read_short(&mut self, n: u32) -> Result<u64> {
        if self.nbits < n {
            self.refill();
            if self.nbits < n {
                return Err(FitsError::UnexpectedEof);
            }
        }
        let value = (self.acc >> 1) >> (63 - n);
        self.acc <<= n;
        self.nbits -= n;
        Ok(value)
    }

    /// Read `n` bits past 56, as two reads; past 64, the stream is malformed.
    #[cold]
    #[inline(never)]
    fn read_wide(&mut self, n: u32) -> Result<u64> {
        if n > 64 {
            return Err(CompressedDataDefect::RiceBitField { bits: n }.into());
        }
        let high = self.read_short(n - 32)?;
        Ok((high << 32) | self.read_short(32)?)
    }

    /// Count and consume leading zero bits up to (and including) the next 1, a whole
    /// accumulator at a time: the unary quotient decode is the hot path of Rice decode.
    fn read_zeros(&mut self) -> Result<u64> {
        let mut z = 0u64;
        loop {
            if self.nbits == 0 {
                self.refill();
                if self.nbits == 0 {
                    return Err(FitsError::UnexpectedEof);
                }
            }
            let run = self.acc.leading_zeros();
            if run < self.nbits {
                self.acc <<= run + 1;
                self.nbits -= run + 1;
                return Ok(z + u64::from(run));
            }
            // Every valid bit is zero: drop them, and the bits past them, which the next
            // refill loads again from `pos`.
            z += u64::from(self.nbits);
            self.acc = 0;
            self.nbits = 0;
        }
    }
}

#[cfg(test)]
mod tests;
