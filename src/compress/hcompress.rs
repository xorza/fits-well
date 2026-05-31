//! `HCOMPRESS_1` tile codec — a port of cfitsio's `fits_hdecompress` (32-bit).
//!
//! Decoding is: read the header + quadtree-coded bit planes (`decode`/`dodecode`/
//! `qtree_decode`), undigitize (multiply by the scale), then invert the
//! H-transform (`hinv`). Smoothing (`SMOOTH = 1`) is not implemented; the common
//! `SMOOTH = 0` path is.

use crate::error::FitsError;
use crate::error::Result;

const MAGIC: [u8; 2] = [0xDD, 0x99];

/// Decode an `HCOMPRESS_1` tile into row-major integer values (`ny` fastest, the
/// FITS axis-1 order the orchestrator expects).
pub(super) fn hcompress_tile(bytes: &[u8]) -> Result<Vec<i64>> {
    let (a, _nx, _ny) = hdecompress(bytes)?;
    Ok(a.into_iter().map(|v| v as i64).collect())
}

/// Bit/byte input over the compressed stream (replaces cfitsio's file globals).
struct BitInput<'a> {
    data: &'a [u8],
    pos: usize,
    buffer: i32,
    bits_to_go: i32,
}

impl<'a> BitInput<'a> {
    fn new(data: &'a [u8]) -> Self {
        BitInput {
            data,
            pos: 0,
            buffer: 0,
            bits_to_go: 0,
        }
    }

    fn byte(&mut self) -> i32 {
        let b = self.data.get(self.pos).copied().unwrap_or(0) as i32;
        self.pos += 1;
        b
    }

    fn read_bytes(&mut self, n: usize) -> Vec<u8> {
        let out = self
            .data
            .get(self.pos..self.pos + n)
            .unwrap_or(&[])
            .to_vec();
        self.pos += n;
        out
    }

    fn readint(&mut self) -> i32 {
        let mut a = self.byte();
        for _ in 1..4 {
            a = (a << 8) + self.byte();
        }
        a
    }

    fn readlonglong(&mut self) -> i64 {
        let mut a = self.byte() as i64;
        for _ in 1..8 {
            a = (a << 8) + self.byte() as i64;
        }
        a
    }

    fn start_inputing_bits(&mut self) {
        self.bits_to_go = 0;
    }

    fn input_bit(&mut self) -> i32 {
        if self.bits_to_go == 0 {
            self.buffer = self.byte();
            self.bits_to_go = 8;
        }
        self.bits_to_go -= 1;
        (self.buffer >> self.bits_to_go) & 1
    }

    fn input_nbits(&mut self, n: i32) -> i32 {
        if self.bits_to_go < n {
            self.buffer = (self.buffer << 8) | self.byte();
            self.bits_to_go += 8;
        }
        self.bits_to_go -= n;
        (self.buffer >> self.bits_to_go) & ((1 << n) - 1)
    }

    fn input_nybble(&mut self) -> i32 {
        self.input_nbits(4)
    }

    /// Read `n` 4-bit nybbles into `array` (faithful to cfitsio's byte-aligned
    /// fast path, including the one-byte backspace).
    fn input_nnybble(&mut self, n: usize, array: &mut [u8]) {
        if n == 1 {
            array[0] = self.input_nybble() as u8;
            return;
        }
        if self.bits_to_go == 8 {
            self.pos -= 1;
            self.bits_to_go = 0;
        }
        let shift1 = self.bits_to_go + 4;
        let shift2 = self.bits_to_go;
        let mut kk = 0;
        let pairs = n / 2;
        if self.bits_to_go == 0 {
            for _ in 0..pairs {
                self.buffer = (self.buffer << 8) | self.byte();
                array[kk] = ((self.buffer >> 4) & 15) as u8;
                array[kk + 1] = (self.buffer & 15) as u8;
                kk += 2;
            }
        } else {
            for _ in 0..pairs {
                self.buffer = (self.buffer << 8) | self.byte();
                array[kk] = ((self.buffer >> shift1) & 15) as u8;
                array[kk + 1] = ((self.buffer >> shift2) & 15) as u8;
                kk += 2;
            }
        }
        if pairs * 2 != n {
            array[n - 1] = self.input_nybble() as u8;
        }
    }

    /// Huffman decode a fixed code into a value 0–15.
    fn input_huffman(&mut self) -> u8 {
        let mut c = self.input_nbits(3);
        if c < 4 {
            return (1 << c) as u8;
        }
        c = self.input_bit() | (c << 1);
        if c < 13 {
            return match c {
                8 => 3,
                9 => 5,
                10 => 10,
                11 => 12,
                _ => 15, // c == 12
            };
        }
        c = self.input_bit() | (c << 1);
        if c < 31 {
            return match c {
                26 => 6,
                27 => 7,
                28 => 9,
                29 => 11,
                _ => 13, // c == 30
            };
        }
        c = self.input_bit() | (c << 1);
        if c == 62 { 0 } else { 14 }
    }
}

/// Top-level: header → quadtree decode → undigitize → inverse H-transform.
fn hdecompress(input: &[u8]) -> Result<(Vec<i32>, usize, usize)> {
    let mut bi = BitInput::new(input);
    if bi.read_bytes(2) != MAGIC {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1: bad magic".to_string(),
        });
    }
    let nx = bi.readint() as usize;
    let ny = bi.readint() as usize;
    let scale = bi.readint();
    let sumall = bi.readlonglong();
    let nbitplanes = bi.read_bytes(3);

    let mut a = vec![0i32; nx * ny];
    dodecode(&mut bi, &mut a, nx, ny, &nbitplanes)?;
    a[0] = sumall as i32;

    undigitize(&mut a, scale);
    hinv(&mut a, nx, ny, scale);
    Ok((a, nx, ny))
}

fn undigitize(a: &mut [i32], scale: i32) {
    if scale <= 1 {
        return;
    }
    for v in a.iter_mut() {
        *v *= scale;
    }
}

/// Decode the four quadrant bit planes, then the sign bits.
fn dodecode(
    bi: &mut BitInput,
    a: &mut [i32],
    nx: usize,
    ny: usize,
    nbitplanes: &[u8],
) -> Result<()> {
    let nx2 = nx.div_ceil(2);
    let ny2 = ny.div_ceil(2);

    bi.start_inputing_bits();
    qtree_decode(bi, &mut a[0..], ny, nx2, ny2, nbitplanes[0] as i32)?;
    qtree_decode(bi, &mut a[ny2..], ny, nx2, ny / 2, nbitplanes[1] as i32)?;
    qtree_decode(
        bi,
        &mut a[ny * nx2..],
        ny,
        nx / 2,
        ny2,
        nbitplanes[1] as i32,
    )?;
    qtree_decode(
        bi,
        &mut a[ny * nx2 + ny2..],
        ny,
        nx / 2,
        ny / 2,
        nbitplanes[2] as i32,
    )?;

    if bi.input_nybble() != 0 {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1: bad bit plane values".to_string(),
        });
    }
    // Sign bits.
    bi.start_inputing_bits();
    for v in a.iter_mut() {
        if *v != 0 && bi.input_bit() != 0 {
            *v = -*v;
        }
    }
    Ok(())
}

/// Read one quadrant's bit planes from the stream into `a` (row stride `n`).
fn qtree_decode(
    bi: &mut BitInput,
    a: &mut [i32],
    n: usize,
    nqx: usize,
    nqy: usize,
    nbitplanes: i32,
) -> Result<()> {
    let nqmax = nqx.max(nqy);
    let mut log2n = ((nqmax as f64).ln() / 2f64.ln() + 0.5) as i32;
    if nqmax > (1 << log2n) {
        log2n += 1;
    }
    let nqx2 = nqx.div_ceil(2);
    let nqy2 = nqy.div_ceil(2);
    let mut scratch = vec![0u8; nqx2 * nqy2];

    for bit in (0..nbitplanes).rev() {
        let b = bi.input_nybble();
        if b == 0 {
            read_bdirect(bi, a, n, nqx, nqy, &mut scratch, bit);
        } else if b != 0xf {
            return Err(FitsError::UnsupportedCompression {
                name: "HCOMPRESS_1: bad format code".to_string(),
            });
        } else {
            scratch[0] = bi.input_huffman();
            let mut nx = 1usize;
            let mut ny = 1usize;
            let mut nfx = nqx;
            let mut nfy = nqy;
            let mut c = 1usize << log2n;
            for _ in 1..log2n {
                c >>= 1;
                nx <<= 1;
                ny <<= 1;
                if nfx <= c {
                    nx -= 1;
                } else {
                    nfx -= c;
                }
                if nfy <= c {
                    ny -= 1;
                } else {
                    nfy -= c;
                }
                qtree_expand(bi, &mut scratch, nx, ny);
            }
            qtree_bitins(&scratch, nqx, nqy, a, n, bit);
        }
    }
    Ok(())
}

/// One quadtree expansion step: expand each 4-bit value to 2×2, then read new
/// codes for the non-zero cells.
fn qtree_expand(bi: &mut BitInput, a: &mut [u8], nx: usize, ny: usize) {
    qtree_copy(a, nx, ny, ny);
    for i in (0..nx * ny).rev() {
        if a[i] != 0 {
            a[i] = bi.input_huffman();
        }
    }
}

/// Expand 4-bit values from `a[(nx+1)/2,(ny+1)/2]` to 2×2 pixels in `a[nx,ny]`
/// (declared row stride `n`); operates in place from the end.
fn qtree_copy(a: &mut [u8], nx: usize, ny: usize, n: usize) {
    let nx2 = nx.div_ceil(2);
    let ny2 = ny.div_ceil(2);
    // Spread the packed 4-bit values out to b[2*i, 2*j], from the end so the
    // in-place expansion does not clobber unread source values.
    let mut k = ny2 * (nx2 - 1) + ny2 - 1;
    for i in (0..nx2).rev() {
        let mut s00 = 2 * (n * i + ny2 - 1);
        for _ in (0..ny2).rev() {
            a[s00] = a[k];
            k = k.wrapping_sub(1);
            s00 = s00.wrapping_sub(2);
        }
    }
    expand_blocks(a, nx, ny, n);
}

/// Expand the stored top-left nybbles into 2×2 bit patterns.
fn expand_blocks(a: &mut [u8], nx: usize, ny: usize, n: usize) {
    let mut i = 0;
    while i + 1 < nx {
        let mut s00 = n * i;
        let mut s10 = s00 + n;
        let mut j = 0;
        while j + 1 < ny {
            let v = a[s00];
            a[s10 + 1] = v & 1;
            a[s10] = (v >> 1) & 1;
            a[s00 + 1] = (v >> 2) & 1;
            a[s00] = (v >> 3) & 1;
            s00 += 2;
            s10 += 2;
            j += 2;
        }
        if j < ny {
            let v = a[s00];
            a[s10] = (v >> 1) & 1;
            a[s00] = (v >> 3) & 1;
        }
        i += 2;
    }
    if i < nx {
        let mut s00 = n * i;
        let mut j = 0;
        while j + 1 < ny {
            let v = a[s00];
            a[s00 + 1] = (v >> 2) & 1;
            a[s00] = (v >> 3) & 1;
            s00 += 2;
            j += 2;
        }
        if j < ny {
            let v = a[s00];
            a[s00] = (v >> 3) & 1;
        }
    }
}

/// Insert the 4-bit codes of `a[(nqx+1)/2,(nqy+1)/2]` into bit plane `bit` of
/// `b[nqx,nqy]` (declared row stride `n`), expanding each to 2×2.
fn qtree_bitins(a: &[u8], nqx: usize, nqy: usize, b: &mut [i32], n: usize, bit: i32) {
    let plane = 1i32 << bit;
    let mut k = 0;
    let mut i = 0;
    while i + 1 < nqx {
        let mut s00 = n * i;
        let mut j = 0;
        while j + 1 < nqy {
            let v = a[k];
            if v & 1 != 0 {
                b[s00 + n + 1] |= plane;
            }
            if v & 2 != 0 {
                b[s00 + n] |= plane;
            }
            if v & 4 != 0 {
                b[s00 + 1] |= plane;
            }
            if v & 8 != 0 {
                b[s00] |= plane;
            }
            s00 += 2;
            k += 1;
            j += 2;
        }
        if j < nqy {
            let v = a[k];
            if v & 2 != 0 {
                b[s00 + n] |= plane;
            }
            if v & 8 != 0 {
                b[s00] |= plane;
            }
            k += 1;
        }
        i += 2;
    }
    if i < nqx {
        let mut s00 = n * i;
        let mut j = 0;
        while j + 1 < nqy {
            let v = a[k];
            if v & 4 != 0 {
                b[s00 + 1] |= plane;
            }
            if v & 8 != 0 {
                b[s00] |= plane;
            }
            s00 += 2;
            k += 1;
            j += 2;
        }
        if j < nqy {
            let v = a[k];
            if v & 8 != 0 {
                b[s00] |= plane;
            }
            k += 1;
        }
    }
    let _ = k;
}

/// A directly-stored (un-quadtree-coded) bit plane: read nybbles, then insert.
fn read_bdirect(
    bi: &mut BitInput,
    a: &mut [i32],
    n: usize,
    nqx: usize,
    nqy: usize,
    scratch: &mut [u8],
    bit: i32,
) {
    let count = (nqx.div_ceil(2)) * (nqy.div_ceil(2));
    bi.input_nnybble(count, scratch);
    qtree_bitins(scratch, nqx, nqy, a, n, bit);
}

/// Inverse H-transform (in place), `SMOOTH = 0`.
fn hinv(a: &mut [i32], nx: usize, ny: usize, scale: i32) {
    let nmax = nx.max(ny);
    let mut log2n = ((nmax as f64).ln() / 2f64.ln() + 0.5) as i32;
    if nmax > (1 << log2n) {
        log2n += 1;
    }
    let mut tmp = vec![0i32; nmax.div_ceil(2)];
    let _ = scale; // only used for smoothing, which is not implemented

    let mut shift = 1;
    let mut bit0 = 1i32 << (log2n - 1);
    let mut bit1 = bit0 << 1;
    let bit2 = bit0 << 2;
    let mut mask0 = -bit0;
    let mut mask1 = mask0 << 1;
    let mask2 = mask0 << 2;
    let mut prnd0 = bit0 >> 1;
    let mut prnd1 = bit1 >> 1;
    let prnd2 = bit2 >> 1;
    let mut nrnd0 = prnd0 - 1;
    let mut nrnd1 = prnd1 - 1;

    // Round h0 to a multiple of bit2 (nrnd2 = prnd2 - 1).
    a[0] = round_signed(a[0], prnd2, prnd2 - 1, mask2);

    let mut nxtop = 1usize;
    let mut nytop = 1usize;
    let mut nxf = nx;
    let mut nyf = ny;
    let mut c = 1usize << log2n;
    for k in (0..log2n).rev() {
        c >>= 1;
        nxtop <<= 1;
        nytop <<= 1;
        if nxf <= c {
            nxtop -= 1;
        } else {
            nxf -= c;
        }
        if nyf <= c {
            nytop -= 1;
        } else {
            nyf -= c;
        }
        if k == 0 {
            nrnd0 = 0;
            shift = 2;
        }
        for i in 0..nxtop {
            unshuffle(&mut a[ny * i..], nytop, 1, &mut tmp);
        }
        for j in 0..nytop {
            unshuffle(&mut a[j..], nxtop, ny, &mut tmp);
        }
        let oddx = nxtop % 2;
        let oddy = nytop % 2;
        let mut i = 0;
        while i < nxtop - oddx {
            let mut s00 = ny * i;
            let mut s10 = s00 + ny;
            let mut j = 0;
            while j < nytop - oddy {
                let h0 = a[s00];
                // Round hx,hy to a multiple of bit1, hc to bit0 (h0 is already bit2).
                let mut hx = round_signed(a[s10], prnd1, nrnd1, mask1);
                let mut hy = round_signed(a[s00 + 1], prnd1, nrnd1, mask1);
                let hc = round_signed(a[s10 + 1], prnd0, nrnd0, mask0);
                let lowbit0 = hc & bit0;
                hx = if hx >= 0 { hx - lowbit0 } else { hx + lowbit0 };
                hy = if hy >= 0 { hy - lowbit0 } else { hy + lowbit0 };
                let lowbit1 = (hc ^ hx ^ hy) & bit1;
                let h0 = if h0 >= 0 {
                    h0 + lowbit0 - lowbit1
                } else {
                    h0 + if lowbit0 == 0 {
                        lowbit1
                    } else {
                        lowbit0 - lowbit1
                    }
                };
                a[s10 + 1] = (h0 + hx + hy + hc) >> shift;
                a[s10] = (h0 + hx - hy - hc) >> shift;
                a[s00 + 1] = (h0 - hx + hy - hc) >> shift;
                a[s00] = (h0 - hx - hy + hc) >> shift;
                s00 += 2;
                s10 += 2;
                j += 2;
            }
            if oddy != 0 {
                let h0 = a[s00];
                let hx = round_signed(a[s10], prnd1, nrnd1, mask1);
                let lowbit1 = hx & bit1;
                let h0 = if h0 >= 0 { h0 - lowbit1 } else { h0 + lowbit1 };
                a[s10] = (h0 + hx) >> shift;
                a[s00] = (h0 - hx) >> shift;
            }
            i += 2;
        }
        if oddx != 0 {
            let mut s00 = ny * i;
            let mut j = 0;
            while j < nytop - oddy {
                let h0 = a[s00];
                let hy = round_signed(a[s00 + 1], prnd1, nrnd1, mask1);
                let lowbit1 = hy & bit1;
                let h0 = if h0 >= 0 { h0 - lowbit1 } else { h0 + lowbit1 };
                a[s00 + 1] = (h0 + hy) >> shift;
                a[s00] = (h0 - hy) >> shift;
                s00 += 2;
                j += 2;
            }
            if oddy != 0 {
                a[ny * i] >>= shift;
            }
        }
        bit1 = bit0;
        bit0 >>= 1;
        mask1 = mask0;
        mask0 >>= 1;
        prnd1 = prnd0;
        prnd0 >>= 1;
        nrnd1 = nrnd0;
        nrnd0 = prnd0 - 1;
    }
}

/// Round `v` to a multiple of `-mask`, using the positive or negative rounding
/// constant per the sign of `v`.
fn round_signed(v: i32, prnd: i32, nrnd: i32, mask: i32) -> i32 {
    (v + if v >= 0 { prnd } else { nrnd }) & mask
}

/// Interleave coefficients: inverse of the shuffle done during compression.
fn unshuffle(a: &mut [i32], n: usize, n2: usize, tmp: &mut [i32]) {
    let nhalf = n.div_ceil(2);
    // Copy 2nd half to tmp.
    for i in nhalf..n {
        tmp[i - nhalf] = a[n2 * i];
    }
    // Distribute 1st half to even elements (from the end).
    for i in (0..nhalf).rev() {
        a[n2 * i * 2] = a[n2 * i];
    }
    // Distribute 2nd half (tmp) to odd elements.
    let mut pt = 0;
    let mut i = 1;
    while i < n {
        a[n2 * i] = tmp[pt];
        pt += 1;
        i += 2;
    }
}
