//! `RICE_1` tile codec (a port of cfitsio's `fits_rdecomp` bitstream layout).

use crate::compress;
use crate::error::FitsError;
use crate::error::Result;
use crate::header::Header;
use crate::keyword::key;

/// Rice block size and pixel width, from the `ZNAMEi`/`ZVALi` parameters.
#[derive(Debug, Clone, Copy)]
pub(super) struct RiceParams {
    pub(super) blocksize: usize,
    pub(super) bytepix: usize,
}

/// Rice parameters from the `ZNAMEi`/`ZVALi` keywords, with the Table 37 defaults
/// of 32 pixels per block and four bytes per encoded integer.
pub(super) fn rice_params(header: &Header) -> Result<RiceParams> {
    let mut blocksize = 32;
    let mut bytepix = 4;
    for entry in header.iter() {
        let Some(i) = compress::parameter_index(entry.keyword) else {
            continue;
        };
        let Some(name) = header.get_text(entry.keyword)? else {
            continue;
        };
        match name {
            "BLOCKSIZE" => {
                if let Some(v) = header.get_integer(key!("ZVAL{i}").as_str())? {
                    blocksize = permitted_parameter(v, &[16, 32])?;
                }
            }
            "BYTEPIX" => {
                if let Some(v) = header.get_integer(key!("ZVAL{i}").as_str())? {
                    bytepix = permitted_parameter(v, &[1, 2, 4, 8])?;
                }
            }
            _ => {}
        }
    }
    Ok(RiceParams { blocksize, bytepix })
}

fn permitted_parameter(value: i64, permitted: &[usize]) -> Result<usize> {
    usize::try_from(value)
        .ok()
        .filter(|value| permitted.contains(value))
        .ok_or(FitsError::KeywordOutOfRange { name: "ZVALn" })
}

/// Decode a `RICE_1` tile of `nx` integer values into `out` (cleared first; a reused
/// buffer, so steady-state decode allocates nothing).
pub(super) fn rice_decode_into(
    bytes: &[u8],
    nx: usize,
    bytepix: usize,
    blocksize: usize,
    out: &mut Vec<i64>,
) -> Result<()> {
    out.clear();
    if nx == 0 {
        return Ok(());
    }
    let nbits_pp = (8 * bytepix) as u32;
    let (fsbits, fsmax) = match bytepix {
        1 => (3u32, 6u32),
        2 => (4, 14),
        4 => (5, 25),
        8 => (6, 57),
        _ => {
            return Err(FitsError::UnsupportedCompression {
                name: format!("RICE_1 with BYTEPIX = {bytepix}"),
            });
        }
    };
    let mask = if nbits_pp >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits_pp) - 1
    };

    let mut br = BitReader::new(bytes);
    let mut lastpix = br.read(nbits_pp)?; // literal first pixel (big-endian)
    out.reserve_exact(nx);
    let mut i = 0;
    while i < nx {
        let fs = br.read(fsbits)? as i64 - 1;
        let imax = (i + blocksize).min(nx);
        for _ in i..imax {
            let diff = if fs < 0 {
                0
            } else if fs as u32 == fsmax {
                br.read(nbits_pp)? // uncompressed block
            } else {
                (br.read_zeros()? << fs) | br.read(fs as u32)?
            };
            // Undo the zigzag mapping, then the differencing (modular at pixel width).
            let d = if diff & 1 == 1 {
                !(diff >> 1)
            } else {
                diff >> 1
            };
            lastpix = lastpix.wrapping_add(d) & mask;
            out.push(sign_extend(lastpix, nbits_pp));
        }
        i = imax;
    }
    Ok(())
}

/// Interpret the low `nbits` of `v` as a two's-complement signed value.
fn sign_extend(v: u64, nbits: u32) -> i64 {
    let shift = 64 - nbits;
    ((v << shift) as i64) >> shift
}

/// Encode `values` as a `RICE_1` tile (a port of cfitsio's `fits_rcomp`),
/// parameterized by `bytepix` (1/2/4/8). Differences are taken modulo the pixel
/// width so the stream round-trips through [`rice_decode_into`].
pub(super) fn rice_encode<T: Copy + Into<i64>>(
    values: &[T],
    bytepix: usize,
    blocksize: usize,
    scratch: &mut RiceScratch,
) -> Vec<u8> {
    let nbits = (8 * bytepix) as u32;
    let (fsbits, fsmax) = match bytepix {
        1 => (3i32, 6i32),
        2 => (4, 14),
        4 => (5, 25),
        8 => (6, 57),
        _ => unreachable!("Rice BYTEPIX is validated by the codec dispatcher"),
    };
    let mask: u64 = if nbits >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits) - 1
    };
    let mut bo = BitOutput::new();
    // Rice output is at most a few bytes per pixel; reserve a pixel's worth up front
    // so the bitstream rarely reallocates mid-tile.
    bo.out.reserve(values.len());
    let first = values.first().copied().map(Into::into).unwrap_or(0) as u64 & mask;
    bo.output_nbits(first, nbits);
    let mut lastpix = first;

    scratch.diffs.reserve(blocksize);
    let mut i = 0;
    while i < values.len() {
        let thisblock = blocksize.min(values.len() - i);
        scratch.diffs.clear();
        let mut pixelsum = 0u128;
        for j in 0..thisblock {
            let next = Into::<i64>::into(values[i + j]) as u64 & mask;
            // signed difference reduced to the pixel width, then zigzag-mapped
            let raw = next.wrapping_sub(lastpix) & mask;
            let s = sign_extend(raw, nbits);
            let d = if s >= 0 {
                (s as u64) << 1
            } else {
                (s.unsigned_abs() << 1).wrapping_sub(1)
            };
            scratch.diffs.push(d);
            pixelsum += d as u128;
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
        let mut fs = 0i32;
        while psum > 0 {
            fs += 1;
            psum >>= 1;
        }

        if fs >= fsmax {
            bo.output_nbits((fsmax + 1) as u64, fsbits as u32);
            for &d in &scratch.diffs {
                bo.output_nbits(d, nbits);
            }
        } else if fs == 0 && pixelsum == 0 {
            bo.output_nbits(0, fsbits as u32);
        } else {
            bo.output_nbits((fs + 1) as u64, fsbits as u32);
            let fsmask = (1u64 << fs) - 1;
            for &d in &scratch.diffs {
                bo.output_rice_value(d, fs as u32, fsmask);
            }
        }
        i += thisblock;
    }
    bo.out
}

#[derive(Debug, Default)]
pub(super) struct RiceScratch {
    diffs: Vec<u64>,
}

/// MSB-first bit output, mirroring cfitsio's `Buffer`/`output_nbits`.
#[derive(Debug)]
struct BitOutput {
    out: Vec<u8>,
    bits_in_last: u8,
}

impl BitOutput {
    fn new() -> Self {
        BitOutput {
            out: Vec::new(),
            bits_in_last: 0,
        }
    }

    fn output_nbits(&mut self, bits: u64, n: u32) {
        for shift in (0..n).rev() {
            self.output_bit(((bits >> shift) & 1) as u8);
        }
    }

    fn output_bit(&mut self, bit: u8) {
        if self.bits_in_last == 0 {
            self.out.push(0);
        }
        let last = self.out.last_mut().unwrap();
        *last |= bit << (7 - self.bits_in_last);
        self.bits_in_last = (self.bits_in_last + 1) % 8;
    }

    /// Output one Rice-coded value: `top = v >> fs` zero bits, a 1, then the low
    /// `fs` bits of `v`.
    fn output_rice_value(&mut self, v: u64, fs: u32, fsmask: u64) {
        let top = v >> fs;
        for _ in 0..top {
            self.output_bit(0);
        }
        self.output_bit(1);
        if fs > 0 {
            self.output_nbits(v & fsmask, fs);
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
    fn new(bytes: &'a [u8]) -> Self {
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
            return Err(FitsError::UnsupportedCompression {
                name: format!("Rice bit field is {n} bits"),
            });
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
            self.acc = (self.acc << 8) | byte as u64;
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
                return Ok(z + run as u64);
            }
            // All valid bits were zero: consume them and refill on the next pass.
            z += self.nbits as u64;
            self.nbits = 0;
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::compress::rice::{self, BitReader};
    use crate::error::FitsError;
    use crate::header::Header;

    #[test]
    fn rice_parameters_reject_invalid_header_values() {
        let mut header = Header::new();
        let defaults = rice::rice_params(&header).unwrap();
        assert_eq!(defaults.blocksize, 32);
        assert_eq!(defaults.bytepix, 4);
        header
            .set_internal("ZNAME1", "BLOCKSIZE")
            .set_internal("ZVAL1", 0);
        assert!(matches!(
            rice::rice_params(&header),
            Err(FitsError::KeywordOutOfRange { name: "ZVALn" })
        ));

        header.set_internal("ZVAL1", "not an integer");
        assert!(matches!(
            rice::rice_params(&header),
            Err(FitsError::TypeMismatch { name, expected })
                if name == "ZVAL1" && expected == "integer"
        ));

        let mut gapped = Header::new();
        gapped
            .set_internal("ZNAME2", "BYTEPIX")
            .set_internal("ZVAL2", 8)
            .set_internal("ZNAME3", "BLOCKSIZE")
            .set_internal("ZVAL3", 16);
        let params = rice::rice_params(&gapped).unwrap();
        assert_eq!(params.bytepix, 8);
        assert_eq!(params.blocksize, 16);

        gapped.set_internal("ZVAL3", 17);
        assert!(matches!(
            rice::rice_params(&gapped),
            Err(FitsError::KeywordOutOfRange { name: "ZVALn" })
        ));
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
        assert!(matches!(br.read_zeros(), Err(FitsError::UnexpectedEof)));

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
        let encoded = rice::rice_encode(&values, 8, 32, &mut rice::RiceScratch::default());
        let mut decoded = Vec::new();
        rice::rice_decode_into(&encoded, values.len(), 8, 32, &mut decoded).unwrap();
        assert_eq!(decoded, values);
    }

    #[test]
    fn odd_final_block_uses_the_canonical_integer_statistic() {
        let encoded = rice::rice_encode(&[0i64, 0, 4], 4, 32, &mut rice::RiceScratch::default());
        // CFITSIO's reference encoder selects split 1 and emits these exact bytes.
        assert_eq!(encoded, [0, 0, 0, 0, 0x15, 0x04]);

        let mut decoded = Vec::new();
        rice::rice_decode_into(&encoded, 3, 4, 32, &mut decoded).unwrap();
        assert_eq!(decoded, [0, 0, 4]);
    }

    #[test]
    fn truncated_stream_is_rejected() {
        // A stream that enters a Rice zero-run (fs = 0) but ends before the
        // terminating 1-bit. byte0 is the literal first pixel; byte1 = 0b001_00000
        // gives the 3-bit fs field `001` (→ fs = 0), after which only zero bits
        // remain. The unary code has no terminating 1 and must report EOF.
        let mut out = Vec::new();
        assert!(matches!(
            rice::rice_decode_into(&[0x00, 0x20], 2, 1, 32, &mut out),
            Err(FitsError::UnexpectedEof)
        ));
    }
}
