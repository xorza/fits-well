//! `RICE_1` tile codec (a port of cfitsio's `fits_rdecomp` bitstream layout).

use crate::bitpix::Bitpix;
use crate::header::Header;

/// Rice `(blocksize, bytepix)` from the `ZNAMEi`/`ZVALi` parameters, defaulting to
/// 32 and `|ZBITPIX|/8`.
pub(super) fn rice_params(header: &Header, zbitpix: Bitpix) -> (usize, usize) {
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
    (blocksize, bytepix)
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
        (self.acc >> self.nbits) & ((1u64 << n) - 1)
    }

    /// Count and consume leading zero bits up to (and including) the next 1.
    pub(super) fn read_zeros(&mut self) -> u64 {
        let mut z = 0;
        while self.read(1) == 0 {
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
}
