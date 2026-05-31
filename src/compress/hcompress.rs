//! `HCOMPRESS_1` tile codec — a port of cfitsio's `fits_hdecompress`/
//! `fits_hcompress` (32-bit).
//!
//! Decoding is: read the header + quadtree-coded bit planes (`decode`/`dodecode`/
//! `qtree_decode`), undigitize (multiply by the scale), then invert the
//! H-transform (`hinv`). Encoding is the mirror: forward H-transform (`htrans`),
//! digitize, then quadtree bit-plane coding (`encode`/`doencode`/`qtree_encode`).
//! Decode supports `SMOOTH = 1` (inverse-transform smoothing) and lossy
//! `scale > 0`; encode supports lossless and lossy (`scale`) but always
//! `SMOOTH = 0`. Like the decoder this is the 32-bit codec, so tile values must
//! fit the int H-transform without overflow (i16 and moderate i32).

use crate::error::FitsError;
use crate::error::Result;

const MAGIC: [u8; 2] = [0xDD, 0x99];

/// Decode an `HCOMPRESS_1` tile into row-major integer values (`ny` fastest, the
/// FITS axis-1 order the orchestrator expects).
pub(super) fn hcompress_tile(bytes: &[u8], smooth: bool) -> Result<Vec<i64>> {
    let (a, _nx, _ny) = hdecompress(bytes, smooth)?;
    Ok(a.into_iter().map(|v| v as i64).collect())
}

/// Encode one tile (`vals` in `ny`-fastest order, `tdims[0]` = ny = FITS axis-1)
/// as an `HCOMPRESS_1` byte stream. `scale = 0` is lossless. The result decodes
/// back through [`hcompress_tile`] to the original values.
pub(super) fn hcompress_tile_encode(vals: &[i64], tdims: &[usize], scale: i32) -> Result<Vec<u8>> {
    let ny = tdims.first().copied().unwrap_or(vals.len()).max(1);
    let nx = (vals.len() / ny).max(1);
    if nx * ny != vals.len() {
        return Err(FitsError::UnsupportedCompression {
            name: "HCOMPRESS_1: tile is not 2-D".to_string(),
        });
    }
    let mut a: Vec<i32> = vals.iter().map(|&v| v as i32).collect();
    htrans(&mut a, nx, ny);
    digitize(&mut a, nx, ny, scale);
    let mut enc = BitOutput::new();
    enc.encode(&mut a, nx, ny, scale);
    Ok(enc.out)
}

/// `CODE`/`NCODE`: the fixed Huffman code (value, bit length) for each 4-bit
/// quadtree symbol 0–15 (cfitsio `code[]`/`ncode[]`).
const CODE: [i32; 16] = [
    0x3e, 0x00, 0x01, 0x08, 0x02, 0x09, 0x1a, 0x1b, 0x03, 0x1c, 0x0a, 0x1d, 0x0b, 0x1e, 0x3f, 0x0c,
];
const NCODE: [i32; 16] = [6, 3, 3, 4, 3, 4, 5, 5, 3, 5, 4, 5, 4, 5, 6, 4];

/// Bit/byte output buffer (replaces cfitsio's file + `buffer2`/`bits_to_go2`
/// globals). Holds the compressed byte stream.
struct BitOutput {
    out: Vec<u8>,
    buffer2: i32,
    bits_to_go2: i32,
}

impl BitOutput {
    fn new() -> Self {
        BitOutput {
            out: Vec::new(),
            buffer2: 0,
            bits_to_go2: 8,
        }
    }

    fn writeint(&mut self, a: i32) {
        self.out.extend_from_slice(&a.to_be_bytes());
    }

    fn writelonglong(&mut self, a: i64) {
        self.out.extend_from_slice(&a.to_be_bytes());
    }

    fn start_outputing_bits(&mut self) {
        self.buffer2 = 0;
        self.bits_to_go2 = 8;
    }

    /// Output the low `n` bits of `bits` (`n` ≤ 8), MSB-first.
    fn output_nbits(&mut self, bits: i32, n: i32) {
        const MASK: [i32; 9] = [0, 1, 3, 7, 15, 31, 63, 127, 255];
        self.buffer2 = self.buffer2.wrapping_shl(n as u32) | (bits & MASK[n as usize]);
        self.bits_to_go2 -= n;
        if self.bits_to_go2 <= 0 {
            self.out
                .push(((self.buffer2 >> (-self.bits_to_go2)) & 0xff) as u8);
            self.bits_to_go2 += 8;
        }
    }

    fn output_nybble(&mut self, bits: i32) {
        self.buffer2 = self.buffer2.wrapping_shl(4) | (bits & 15);
        self.bits_to_go2 -= 4;
        if self.bits_to_go2 <= 0 {
            self.out
                .push(((self.buffer2 >> (-self.bits_to_go2)) & 0xff) as u8);
            self.bits_to_go2 += 8;
        }
    }

    /// Output `n` 4-bit nybbles from `array` (cfitsio's byte-aligned fast path).
    fn output_nnybble(&mut self, n: usize, array: &[u8]) {
        if n == 1 {
            self.output_nybble(array[0] as i32);
            return;
        }
        let mut kk = 0usize;
        if self.bits_to_go2 <= 4 {
            self.output_nybble(array[0] as i32);
            kk += 1;
            if n == 2 {
                self.output_nybble(array[1] as i32);
                return;
            }
        }
        let shift = 8 - self.bits_to_go2;
        let jj = (n - kk) / 2;
        if self.bits_to_go2 == 8 {
            self.buffer2 = 0;
            for _ in 0..jj {
                self.out
                    .push(((array[kk] & 15) << 4) | (array[kk + 1] & 15));
                kk += 2;
            }
        } else {
            for _ in 0..jj {
                self.buffer2 = self.buffer2.wrapping_shl(8)
                    | (((array[kk] as i32 & 15) << 4) | (array[kk + 1] as i32 & 15));
                kk += 2;
                self.out.push(((self.buffer2 >> shift) & 0xff) as u8);
            }
        }
        if kk != n {
            self.output_nybble(array[n - 1] as i32);
        }
    }

    fn done_outputing_bits(&mut self) {
        if self.bits_to_go2 < 8 {
            self.out.push((self.buffer2 << self.bits_to_go2) as u8);
        }
    }

    /// Write the header, extract sign bits, compute per-quadrant bit-plane counts,
    /// quadtree-encode the planes, then append the sign bytes (cfitsio `encode`).
    fn encode(&mut self, a: &mut [i32], nx: usize, ny: usize, scale: i32) {
        let nel = nx * ny;
        self.out.extend_from_slice(&MAGIC);
        self.writeint(nx as i32);
        self.writeint(ny as i32);
        self.writeint(scale);
        self.writelonglong(a[0] as i64);
        a[0] = 0;

        // Sign bits (one per non-zero element, MSB-first within each byte); a[i]
        // is replaced by its absolute value.
        let mut signbits = vec![0u8; nel.div_ceil(8)];
        let mut nsign = 0usize;
        let mut bits_to_go = 8i32;
        for v in a.iter_mut().take(nel) {
            if *v > 0 {
                signbits[nsign] <<= 1;
                bits_to_go -= 1;
            } else if *v < 0 {
                signbits[nsign] = (signbits[nsign] << 1) | 1;
                bits_to_go -= 1;
                *v = -*v;
            }
            if bits_to_go == 0 {
                bits_to_go = 8;
                nsign += 1;
            }
        }
        if bits_to_go != 8 {
            signbits[nsign] <<= bits_to_go;
            nsign += 1;
        }

        // Per-quadrant maximum, then bit-plane count = bits needed for that max.
        let nx2 = nx.div_ceil(2);
        let ny2 = ny.div_ceil(2);
        let mut vmax = [0i32; 3];
        let mut j = 0usize;
        let mut k = 0usize;
        for &v in a.iter().take(nel) {
            let q = (j >= ny2) as usize + (k >= nx2) as usize;
            if vmax[q] < v {
                vmax[q] = v;
            }
            j += 1;
            if j >= ny {
                j = 0;
                k += 1;
            }
        }
        let mut nbitplanes = [0u8; 3];
        for q in 0..3 {
            let mut m = vmax[q];
            while m > 0 {
                m >>= 1;
                nbitplanes[q] += 1;
            }
        }
        self.out.extend_from_slice(&nbitplanes);

        self.doencode(a, nx, ny, &nbitplanes);
        self.out.extend_from_slice(&signbits[..nsign]);
    }

    /// Quadtree-encode the four quadrants, then an EOF nybble (cfitsio `doencode`).
    fn doencode(&mut self, a: &[i32], nx: usize, ny: usize, nbitplanes: &[u8; 3]) {
        let nx2 = nx.div_ceil(2);
        let ny2 = ny.div_ceil(2);
        self.start_outputing_bits();
        self.qtree_encode(&a[0..], ny, nx2, ny2, nbitplanes[0] as i32);
        self.qtree_encode(&a[ny2..], ny, nx2, ny / 2, nbitplanes[1] as i32);
        self.qtree_encode(&a[ny * nx2..], ny, nx / 2, ny2, nbitplanes[1] as i32);
        self.qtree_encode(
            &a[ny * nx2 + ny2..],
            ny,
            nx / 2,
            ny / 2,
            nbitplanes[2] as i32,
        );
        self.output_nybble(0);
        self.done_outputing_bits();
    }

    /// Quadtree-code one quadrant's bit planes, top plane first (cfitsio
    /// `qtree_encode`). Falls back to a direct bitmap when the quadtree expands.
    fn qtree_encode(&mut self, a: &[i32], n: usize, nqx: usize, nqy: usize, nbitplanes: i32) {
        let nqmax = nqx.max(nqy);
        let mut log2n = ((nqmax as f64).ln() / 2f64.ln() + 0.5) as i32;
        if nqmax > (1 << log2n) {
            log2n += 1;
        }
        let nqx2 = nqx.div_ceil(2);
        let nqy2 = nqy.div_ceil(2);
        let bmax = (nqx2 * nqy2).div_ceil(2);
        let mut scratch = vec![0u8; (nqx2 * nqy2).max(1)];
        let mut buffer = vec![0u8; bmax.max(1)];

        for bit in (0..nbitplanes).rev() {
            let mut b = 0usize;
            let mut bitbuffer = 0i32;
            let mut bits_to_go3 = 0i32;
            qtree_onebit(a, n, nqx, nqy, &mut scratch, bit);
            let mut nx = (nqx + 1) >> 1;
            let mut ny = (nqy + 1) >> 1;
            let mut overflow = bufcopy(
                &scratch,
                nx * ny,
                &mut buffer,
                &mut b,
                bmax,
                &mut bitbuffer,
                &mut bits_to_go3,
            );
            for _ in 1..log2n {
                if overflow {
                    break;
                }
                qtree_reduce(&mut scratch, ny, nx, ny);
                nx = (nx + 1) >> 1;
                ny = (ny + 1) >> 1;
                overflow = bufcopy(
                    &scratch,
                    nx * ny,
                    &mut buffer,
                    &mut b,
                    bmax,
                    &mut bitbuffer,
                    &mut bits_to_go3,
                );
            }
            if overflow {
                self.write_bdirect(a, n, nqx, nqy, &mut scratch, bit);
                continue;
            }
            // Quadtree code: a 0xF marker, the leftover Huffman bits, then the
            // buffered code bytes in reverse order.
            self.output_nybble(0xF);
            if b == 0 {
                if bits_to_go3 > 0 {
                    self.output_nbits(bitbuffer & ((1 << bits_to_go3) - 1), bits_to_go3);
                } else {
                    self.output_nbits(CODE[0], NCODE[0]);
                }
            } else {
                if bits_to_go3 > 0 {
                    self.output_nbits(bitbuffer & ((1 << bits_to_go3) - 1), bits_to_go3);
                }
                for i in (0..b).rev() {
                    self.output_nbits(buffer[i] as i32, 8);
                }
            }
        }
    }

    /// Direct (un-quadtree) bitmap fallback: a 0x0 marker, then the packed nybbles.
    fn write_bdirect(
        &mut self,
        a: &[i32],
        n: usize,
        nqx: usize,
        nqy: usize,
        scratch: &mut [u8],
        bit: i32,
    ) {
        self.output_nybble(0x0);
        qtree_onebit(a, n, nqx, nqy, scratch, bit);
        self.output_nnybble(nqx.div_ceil(2) * nqy.div_ceil(2), scratch);
    }
}

/// Forward H-transform (in place), the inverse of [`hinv`] (cfitsio `htrans`).
fn htrans(a: &mut [i32], nx: usize, ny: usize) {
    let nmax = nx.max(ny);
    let mut log2n = ((nmax as f64).ln() / 2f64.ln() + 0.5) as i32;
    if nmax > (1 << log2n) {
        log2n += 1;
    }
    let mut tmp = vec![0i32; nmax.div_ceil(2).max(1)];

    let mut shift = 0u32;
    let mut mask = -2i32;
    let mut mask2 = mask << 1;
    let mut prnd = 1i32;
    let mut prnd2 = prnd << 1;
    let mut nrnd2 = prnd2 - 1;

    let mut nxtop = nx;
    let mut nytop = ny;
    for _ in 0..log2n {
        let oddx = nxtop % 2;
        let oddy = nytop % 2;
        let mut i = 0usize;
        while i < nxtop - oddx {
            let mut s00 = i * ny;
            let mut s10 = s00 + ny;
            let mut j = 0usize;
            while j < nytop - oddy {
                let h0 = (a[s10 + 1]
                    .wrapping_add(a[s10])
                    .wrapping_add(a[s00 + 1])
                    .wrapping_add(a[s00]))
                    >> shift;
                let hx = (a[s10 + 1]
                    .wrapping_add(a[s10])
                    .wrapping_sub(a[s00 + 1])
                    .wrapping_sub(a[s00]))
                    >> shift;
                let hy = (a[s10 + 1]
                    .wrapping_sub(a[s10])
                    .wrapping_add(a[s00 + 1])
                    .wrapping_sub(a[s00]))
                    >> shift;
                let hc = (a[s10 + 1]
                    .wrapping_sub(a[s10])
                    .wrapping_sub(a[s00 + 1])
                    .wrapping_add(a[s00]))
                    >> shift;
                a[s10 + 1] = hc;
                a[s10] = (if hx >= 0 { hx + prnd } else { hx }) & mask;
                a[s00 + 1] = (if hy >= 0 { hy + prnd } else { hy }) & mask;
                a[s00] = (if h0 >= 0 { h0 + prnd2 } else { h0 + nrnd2 }) & mask2;
                s00 += 2;
                s10 += 2;
                j += 2;
            }
            if oddy != 0 {
                let h0 = a[s10].wrapping_add(a[s00]).wrapping_shl(1 - shift);
                let hx = a[s10].wrapping_sub(a[s00]).wrapping_shl(1 - shift);
                a[s10] = (if hx >= 0 { hx + prnd } else { hx }) & mask;
                a[s00] = (if h0 >= 0 { h0 + prnd2 } else { h0 + nrnd2 }) & mask2;
            }
            i += 2;
        }
        if oddx != 0 {
            let mut s00 = i * ny;
            let mut j = 0usize;
            while j < nytop - oddy {
                let h0 = a[s00 + 1].wrapping_add(a[s00]).wrapping_shl(1 - shift);
                let hy = a[s00 + 1].wrapping_sub(a[s00]).wrapping_shl(1 - shift);
                a[s00 + 1] = (if hy >= 0 { hy + prnd } else { hy }) & mask;
                a[s00] = (if h0 >= 0 { h0 + prnd2 } else { h0 + nrnd2 }) & mask2;
                s00 += 2;
                j += 2;
            }
            if oddy != 0 {
                let h0 = a[i * ny].wrapping_shl(2 - shift);
                a[i * ny] = (if h0 >= 0 { h0 + prnd2 } else { h0 + nrnd2 }) & mask2;
            }
        }
        for i in 0..nxtop {
            shuffle(&mut a[ny * i..], nytop, 1, &mut tmp);
        }
        for j in 0..nytop {
            shuffle(&mut a[j..], nxtop, ny, &mut tmp);
        }
        nxtop = (nxtop + 1) >> 1;
        nytop = (nytop + 1) >> 1;
        shift = 1;
        mask = mask2;
        prnd = prnd2;
        mask2 <<= 1;
        prnd2 <<= 1;
        nrnd2 = prnd2 - 1;
    }
}

/// Group coefficients by order: de-interleave even/odd elements (the inverse of
/// the decoder's `unshuffle`; cfitsio `shuffle`).
fn shuffle(a: &mut [i32], n: usize, n2: usize, tmp: &mut [i32]) {
    let mut pt = 0usize;
    let mut i = 1usize;
    while i < n {
        tmp[pt] = a[n2 * i];
        pt += 1;
        i += 2;
    }
    let mut p1 = n2;
    let mut p2 = n2 + n2;
    let mut i = 2usize;
    while i < n {
        a[p1] = a[p2];
        p1 += n2;
        p2 += n2 + n2;
        i += 2;
    }
    let mut pt = 0usize;
    let mut i = 1usize;
    while i < n {
        a[p1] = tmp[pt];
        p1 += n2;
        pt += 1;
        i += 2;
    }
}

/// Digitize: round each coefficient to a multiple of `scale` (no-op for lossless
/// `scale ≤ 1`; cfitsio `digitize`).
fn digitize(a: &mut [i32], nx: usize, ny: usize, scale: i32) {
    if scale <= 1 {
        return;
    }
    let d = (scale + 1) / 2 - 1;
    for v in a.iter_mut().take(nx * ny) {
        *v = if *v > 0 { *v + d } else { *v - d } / scale;
    }
}

/// First quadtree reduction step on bit `bit` of `a` → 4-bit codes in `b`
/// (cfitsio `qtree_onebit`). `a` is non-negative here, so shifts can't sign-fill.
fn qtree_onebit(a: &[i32], n: usize, nx: usize, ny: usize, b: &mut [u8], bit: i32) {
    let b0 = 1i32 << bit;
    let b1 = b0 << 1;
    let b2 = b0 << 2;
    let b3 = b0 << 3;
    let mut k = 0usize;
    let mut i = 0usize;
    while i + 1 < nx {
        let mut s00 = n * i;
        let mut s10 = s00 + n;
        let mut j = 0usize;
        while j + 1 < ny {
            b[k] = (((a[s10 + 1] & b0)
                | (a[s10].wrapping_shl(1) & b1)
                | (a[s00 + 1].wrapping_shl(2) & b2)
                | (a[s00].wrapping_shl(3) & b3))
                >> bit) as u8;
            k += 1;
            s00 += 2;
            s10 += 2;
            j += 2;
        }
        if j < ny {
            b[k] = (((a[s10].wrapping_shl(1) & b1) | (a[s00].wrapping_shl(3) & b3)) >> bit) as u8;
            k += 1;
        }
        i += 2;
    }
    if i < nx {
        let mut s00 = n * i;
        let mut j = 0usize;
        while j + 1 < ny {
            b[k] =
                (((a[s00 + 1].wrapping_shl(2) & b2) | (a[s00].wrapping_shl(3) & b3)) >> bit) as u8;
            k += 1;
            s00 += 2;
            j += 2;
        }
        if j < ny {
            b[k] = ((a[s00].wrapping_shl(3) & b3) >> bit) as u8;
        }
    }
}

/// One quadtree reduction step (in place): a 4-bit cell is non-zero where any of
/// its four children are (cfitsio `qtree_reduce` with `a == b`).
fn qtree_reduce(a: &mut [u8], n: usize, nx: usize, ny: usize) {
    let mut k = 0usize;
    let mut i = 0usize;
    while i + 1 < nx {
        let mut s00 = n * i;
        let mut s10 = s00 + n;
        let mut j = 0usize;
        while j + 1 < ny {
            a[k] = (a[s10 + 1] != 0) as u8
                | (((a[s10] != 0) as u8) << 1)
                | (((a[s00 + 1] != 0) as u8) << 2)
                | (((a[s00] != 0) as u8) << 3);
            k += 1;
            s00 += 2;
            s10 += 2;
            j += 2;
        }
        if j < ny {
            a[k] = (((a[s10] != 0) as u8) << 1) | (((a[s00] != 0) as u8) << 3);
            k += 1;
        }
        i += 2;
    }
    if i < nx {
        let mut s00 = n * i;
        let mut j = 0usize;
        while j + 1 < ny {
            a[k] = (((a[s00 + 1] != 0) as u8) << 2) | (((a[s00] != 0) as u8) << 3);
            k += 1;
            s00 += 2;
            j += 2;
        }
        if j < ny {
            a[k] = ((a[s00] != 0) as u8) << 3;
        }
    }
}

/// Append Huffman codes for the non-zero cells of `a[0..n]` to `buffer`, packing
/// 8 bits at a time. Returns `true` if the buffer fills (the quadtree is
/// expanding the data, so the caller falls back to a direct bitmap).
#[allow(clippy::too_many_arguments)]
fn bufcopy(
    a: &[u8],
    n: usize,
    buffer: &mut [u8],
    b: &mut usize,
    bmax: usize,
    bitbuffer: &mut i32,
    bits_to_go3: &mut i32,
) -> bool {
    for &cell in a.iter().take(n) {
        if cell != 0 {
            *bitbuffer |= CODE[cell as usize] << *bits_to_go3;
            *bits_to_go3 += NCODE[cell as usize];
            if *bits_to_go3 >= 8 {
                buffer[*b] = (*bitbuffer & 0xFF) as u8;
                *b += 1;
                if *b >= bmax {
                    return true;
                }
                *bitbuffer >>= 8;
                *bits_to_go3 -= 8;
            }
        }
    }
    false
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
fn hdecompress(input: &[u8], smooth: bool) -> Result<(Vec<i32>, usize, usize)> {
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
    hinv(&mut a, nx, ny, scale, smooth);
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
fn hinv(a: &mut [i32], nx: usize, ny: usize, scale: i32, smooth: bool) {
    let nmax = nx.max(ny);
    let mut log2n = ((nmax as f64).ln() / 2f64.ln() + 0.5) as i32;
    if nmax > (1 << log2n) {
        log2n += 1;
    }
    let mut tmp = vec![0i32; nmax.div_ceil(2)];

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
        // Smooth by interpolating coefficients (SMOOTH=1, lossy scale>1 only).
        if smooth {
            hsmooth(a, nxtop, nytop, ny, scale);
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

/// Smooth H-transform coefficients by interpolation (cfitsio `hsmooth`): adjust
/// the x, y, and curvature differences toward what the neighbouring zones imply,
/// each change clamped to ±scale/2 (the rounding slack from digitization).
/// Only meaningful for lossy decoding (`scale > 1`, `SMOOTH = 1`). Intermediates
/// use `i64` so the difference/shift arithmetic can't overflow.
fn hsmooth(a: &mut [i32], nxtop: usize, nytop: usize, ny: usize, scale: i32) {
    let smax = (scale >> 1) as i64;
    if smax <= 0 {
        return;
    }
    // Integer divide-by-2^k matching C's rounding-toward-zero on negatives.
    let shr = |s: i64, k: u32| {
        if s >= 0 {
            s >> k
        } else {
            (s + (1 << k) - 1) >> k
        }
    };
    let ny2 = ny * 2;

    // Adjust x difference hx (edges left untouched: i from 2 to nxtop-2).
    let mut i = 2;
    while i + 2 < nxtop {
        let (mut s00, mut s10) = (ny * i, ny * i + ny);
        let mut j = 0;
        while j < nytop {
            let hm = a[s00 - ny2] as i64;
            let h0 = a[s00] as i64;
            let hp = a[s00 + ny2] as i64;
            let mut diff = hp - hm;
            let dmax = ((hp - h0).min(h0 - hm)).max(0) << 2;
            let dmin = ((hp - h0).max(h0 - hm)).min(0) << 2;
            if dmin < dmax {
                diff = diff.clamp(dmin, dmax);
                let s = shr(diff - ((a[s10] as i64) << 3), 3).clamp(-smax, smax);
                a[s10] = (a[s10] as i64 + s) as i32;
            }
            s00 += 2;
            s10 += 2;
            j += 2;
        }
        i += 2;
    }

    // Adjust y difference hy.
    let mut i = 0;
    while i < nxtop {
        let mut s00 = ny * i + 2;
        let mut j = 2;
        while j + 2 < nytop {
            let hm = a[s00 - 2] as i64;
            let h0 = a[s00] as i64;
            let hp = a[s00 + 2] as i64;
            let mut diff = hp - hm;
            let dmax = ((hp - h0).min(h0 - hm)).max(0) << 2;
            let dmin = ((hp - h0).max(h0 - hm)).min(0) << 2;
            if dmin < dmax {
                diff = diff.clamp(dmin, dmax);
                let s = shr(diff - ((a[s00 + 1] as i64) << 3), 3).clamp(-smax, smax);
                a[s00 + 1] = (a[s00 + 1] as i64 + s) as i32;
            }
            s00 += 2;
            j += 2;
        }
        i += 2;
    }

    // Adjust curvature difference hc.
    let mut i = 2;
    while i + 2 < nxtop {
        let (mut s00, mut s10) = (ny * i + 2, ny * i + 2 + ny);
        let mut j = 2;
        while j + 2 < nytop {
            let hmm = a[s00 - ny2 - 2] as i64;
            let hpm = a[s00 + ny2 - 2] as i64;
            let hmp = a[s00 - ny2 + 2] as i64;
            let hpp = a[s00 + ny2 + 2] as i64;
            let h0 = a[s00] as i64;
            let mut diff = hpp + hmm - hmp - hpm;
            let hx2 = (a[s10] as i64) << 1;
            let hy2 = (a[s00 + 1] as i64) << 1;
            let m1 = ((hpp - h0).max(0) - hx2 - hy2).min((h0 - hpm).max(0) + hx2 - hy2);
            let m2 = ((h0 - hmp).max(0) - hx2 + hy2).min((hmm - h0).max(0) + hx2 + hy2);
            let dmax = m1.min(m2) << 4;
            let m1 = ((hpp - h0).min(0) - hx2 - hy2).max((h0 - hpm).min(0) + hx2 - hy2);
            let m2 = ((h0 - hmp).min(0) - hx2 + hy2).max((hmm - h0).min(0) + hx2 + hy2);
            let dmin = m1.max(m2) << 4;
            if dmin < dmax {
                diff = diff.clamp(dmin, dmax);
                let s = shr(diff - ((a[s10 + 1] as i64) << 6), 6).clamp(-smax, smax);
                a[s10 + 1] = (a[s10 + 1] as i64 + s) as i32;
            }
            s00 += 2;
            s10 += 2;
            j += 2;
        }
        i += 2;
    }
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
