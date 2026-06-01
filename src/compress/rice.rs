//! `RICE_1` tile codec (a port of cfitsio's `fits_rdecomp` bitstream layout).

use crate::bitpix::Bitpix;
use crate::header::Header;

/// Rice block size and pixel width, from the `ZNAMEi`/`ZVALi` parameters.
#[derive(Debug, Clone, Copy)]
pub(super) struct RiceParams {
    pub blocksize: usize,
    pub bytepix: usize,
}

/// Rice parameters from the `ZNAMEi`/`ZVALi` keywords, defaulting to a block size
/// of 32 and `bytepix = |ZBITPIX|/8`.
pub(super) fn rice_params(header: &Header, zbitpix: Bitpix) -> RiceParams {
    let mut blocksize = 32;
    let mut bytepix = zbitpix.elem_size();
    let mut i = 1;
    while let Some(name) = header.get_text(&format!("ZNAME{i}")) {
        if let Some(v) = header.get_integer(&format!("ZVAL{i}")) {
            match name {
                "BLOCKSIZE" => blocksize = v.max(1) as usize,
                "BYTEPIX" => bytepix = v.max(1) as usize,
                _ => {}
            }
        }
        i += 1;
    }
    RiceParams { blocksize, bytepix }
}

/// Decode a `RICE_1` tile into `nx` integer values.
pub(super) fn rice_decode(bytes: &[u8], nx: usize, bytepix: usize, blocksize: usize) -> Vec<i64> {
    let nbits_pp = (8 * bytepix) as u32;
    let (fsbits, fsmax) = match bytepix {
        1 => (3u32, 6u32),
        2 => (4, 14),
        _ => (5, 25), // 4-byte (and wider) pixels
    };
    let mask = if nbits_pp >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits_pp) - 1
    };

    let mut br = BitReader::new(bytes);
    let mut lastpix = br.read(nbits_pp); // literal first pixel (big-endian)
    let mut out = Vec::with_capacity(nx);
    let mut i = 0;
    while i < nx {
        let fs = br.read(fsbits) as i64 - 1;
        let imax = (i + blocksize).min(nx);
        for _ in i..imax {
            let diff = if fs < 0 {
                0
            } else if fs as u32 == fsmax {
                br.read(nbits_pp) // uncompressed block
            } else {
                (br.read_zeros() << fs) | br.read(fs as u32)
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
    out
}

/// Interpret the low `nbits` of `v` as a two's-complement signed value.
fn sign_extend(v: u64, nbits: u32) -> i64 {
    let shift = 64 - nbits;
    ((v << shift) as i64) >> shift
}

/// Encode `values` as a `RICE_1` tile (a port of cfitsio's `fits_rcomp`),
/// parameterized by `bytepix` (1/2/4). Differences are taken modulo the pixel
/// width so the stream round-trips through [`rice_decode`].
pub(super) fn rice_encode(values: &[i64], bytepix: usize, blocksize: usize) -> Vec<u8> {
    let nbits = (8 * bytepix) as u32;
    let (fsbits, fsmax) = match bytepix {
        1 => (3i32, 6i32),
        2 => (4, 14),
        _ => (5, 25),
    };
    let mask: u64 = if nbits >= 64 {
        u64::MAX
    } else {
        (1u64 << nbits) - 1
    };
    let half: u64 = 1u64 << (nbits - 1);

    let mut bo = BitOutput::new();
    let first = (*values.first().unwrap_or(&0) as u64) & mask;
    bo.output_nbits(first as i64, nbits as i32);
    let mut lastpix = first;

    let mut i = 0;
    while i < values.len() {
        let thisblock = blocksize.min(values.len() - i);
        let mut diffs = Vec::with_capacity(thisblock);
        let mut pixelsum = 0.0f64;
        for j in 0..thisblock {
            let next = (values[i + j] as u64) & mask;
            // signed difference reduced to the pixel width, then zigzag-mapped
            let raw = next.wrapping_sub(lastpix) & mask;
            let s = if raw >= half {
                raw as i64 - (mask as i64) - 1
            } else {
                raw as i64
            };
            let d = if s >= 0 {
                (s as u64) << 1
            } else {
                (((-s) as u64) << 1) - 1
            };
            diffs.push(d);
            pixelsum += d as f64;
            lastpix = next;
        }

        let dpsum = ((pixelsum - thisblock as f64 / 2.0 - 1.0) / thisblock as f64).max(0.0);
        let mut psum = (dpsum as u64) >> 1;
        let mut fs = 0i32;
        while psum > 0 {
            fs += 1;
            psum >>= 1;
        }

        if fs >= fsmax {
            bo.output_nbits((fsmax + 1) as i64, fsbits);
            for &d in &diffs {
                bo.output_nbits(d as i64, nbits as i32);
            }
        } else if fs == 0 && pixelsum == 0.0 {
            bo.output_nbits(0, fsbits);
        } else {
            bo.output_nbits((fs + 1) as i64, fsbits);
            let fsmask = (1i64 << fs) - 1;
            for &d in &diffs {
                bo.output_rice_value(d as i64, fs, fsmask);
            }
        }
        i += thisblock;
    }
    bo.done();
    bo.out
}

/// MSB-first bit output, mirroring cfitsio's `Buffer`/`output_nbits`.
struct BitOutput {
    out: Vec<u8>,
    bitbuffer: i64,
    bits_to_go: i32,
}

impl BitOutput {
    fn new() -> Self {
        BitOutput {
            out: Vec::new(),
            bitbuffer: 0,
            bits_to_go: 8,
        }
    }

    fn output_nbits(&mut self, bits: i64, mut n: i32) {
        let mask = |k: i32| {
            if k >= 32 {
                0xFFFF_FFFFi64
            } else {
                (1i64 << k) - 1
            }
        };
        let mut lb = self.bitbuffer;
        let mut ltg = self.bits_to_go;
        if ltg + n > 32 {
            lb <<= ltg;
            lb |= (bits >> (n - ltg)) & mask(ltg);
            self.out.push((lb & 0xff) as u8);
            n -= ltg;
            ltg = 8;
        }
        lb <<= n;
        lb |= bits & mask(n);
        ltg -= n;
        while ltg <= 0 {
            self.out.push(((lb >> (-ltg)) & 0xff) as u8);
            ltg += 8;
        }
        self.bitbuffer = lb;
        self.bits_to_go = ltg;
    }

    /// Output one Rice-coded value: `top = v >> fs` zero bits, a 1, then the low
    /// `fs` bits of `v`.
    fn output_rice_value(&mut self, v: i64, fs: i32, fsmask: i64) {
        let top = v >> fs;
        if (self.bits_to_go as i64) > top {
            self.bitbuffer <<= top + 1;
            self.bitbuffer |= 1;
            self.bits_to_go -= (top + 1) as i32;
        } else {
            self.bitbuffer <<= self.bits_to_go;
            self.out.push((self.bitbuffer & 0xff) as u8);
            let mut t = top - self.bits_to_go as i64;
            while t >= 8 {
                self.out.push(0);
                t -= 8;
            }
            self.bitbuffer = 1;
            self.bits_to_go = 7 - t as i32;
        }
        if fs > 0 {
            self.bitbuffer <<= fs;
            self.bitbuffer |= v & fsmask;
            self.bits_to_go -= fs;
            while self.bits_to_go <= 0 {
                self.out
                    .push(((self.bitbuffer >> (-self.bits_to_go)) & 0xff) as u8);
                self.bits_to_go += 8;
            }
        }
    }

    fn done(&mut self) {
        if self.bits_to_go < 8 {
            self.out
                .push(((self.bitbuffer << self.bits_to_go) & 0xff) as u8);
        }
    }
}

/// A MSB-first bit reader over a compressed byte stream.
pub(super) struct BitReader<'a> {
    bytes: &'a [u8],
    pos: usize,
    acc: u64,
    nbits: u32,
}

impl<'a> BitReader<'a> {
    pub(super) fn new(bytes: &'a [u8]) -> Self {
        BitReader {
            bytes,
            pos: 0,
            acc: 0,
            nbits: 0,
        }
    }

    /// Read `n` bits (MSB-first); past end-of-input reads as zero bits.
    pub(super) fn read(&mut self, n: u32) -> u64 {
        if n == 0 {
            return 0;
        }
        while self.nbits < n {
            let byte = self.bytes.get(self.pos).copied().unwrap_or(0);
            self.pos += 1;
            self.acc = (self.acc << 8) | byte as u64;
            self.nbits += 8;
        }
        self.nbits -= n;
        let mask = if n >= 64 { u64::MAX } else { (1u64 << n) - 1 };
        (self.acc >> self.nbits) & mask
    }

    /// Count and consume leading zero bits up to (and including) the next 1.
    /// Stops once the real input is exhausted: `read` zero-fills past EOF, so a
    /// truncated/malformed tile with no terminating 1 bit would otherwise spin this
    /// loop forever (a DoS on untrusted compressed bytes).
    pub(super) fn read_zeros(&mut self) -> u64 {
        let mut z = 0;
        while self.read(1) == 0 {
            if self.pos > self.bytes.len() {
                break;
            }
            z += 1;
        }
        z
    }
}

#[cfg(test)]
mod tests {
    use super::BitReader;

    #[test]
    fn bit_reader_reads_msb_first() {
        let mut br = BitReader::new(&[0b1011_0010, 0b1111_0000]);
        assert_eq!(br.read(1), 1);
        assert_eq!(br.read(3), 0b011);
        assert_eq!(br.read(4), 0b0010);
        assert_eq!(br.read(4), 0b1111);
    }

    #[test]
    fn truncated_stream_terminates_instead_of_hanging() {
        // A stream that enters a Rice zero-run (fs = 0) but ends before the
        // terminating 1-bit. byte0 is the literal first pixel; byte1 = 0b001_00000
        // gives the 3-bit fs field `001` (→ fs = 0), after which only zero bits
        // remain and `read` zero-fills past EOF. Without the exhaustion guard in
        // `read_zeros` this would spin forever; the decode must return (here, two
        // bounded values) — reaching this assert at all is the guarantee.
        let out = super::rice_decode(&[0x00, 0x20], 2, 1, 32);
        assert_eq!(out.len(), 2);
    }
}
