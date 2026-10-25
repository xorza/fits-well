//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use crate::allocation;
use crate::fits_error::CompressedDataDefect;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use flate2::{Compress, Compression, Crc, Decompress, FlushCompress, FlushDecompress, Status};

/// Default deflate level used by [`crate::compress::Gzip::default`]. Level 1 favors write
/// speed (gzip was the slowest compress path at the higher default); construct
/// [`crate::compress::Gzip::new`] or [`crate::compress::Gzip::shuffled`] with a higher level for a
/// tighter ratio.
pub(super) const DEFAULT_GZIP_LEVEL: u32 = 1;

/// A gzip stream's parameters: the deflate `level` (0–9), which changes only the
/// speed↔ratio tradeoff, and the `shuffle` width of `GZIP_2`'s significance planes, 1
/// for `GZIP_1`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Codec {
    pub(super) level: u32,
    pub(super) shuffle: usize,
}

/// The buffers a gzip encode and decode reuse: the decoded bytes, the shuffled ones, the
/// stream, and the deflate and inflate states, each made by the first encode or decode
/// and reset by each after it.
#[derive(Debug, Default)]
pub(super) struct GzipScratch {
    bytes: Vec<u8>,
    reordered: Vec<u8>,
    stream: Vec<u8>,
    deflate: Option<Deflate>,
    inflate: Option<Decompress>,
}

/// A deflate state and the level it compresses at, which only a new state changes.
#[derive(Debug)]
struct Deflate {
    level: u32,
    state: Compress,
}

/// A bound on the heap bytes of the inflate state a [`Decompress`] boxes: miniz_oxide's
/// `InflateState`, whose type flate2 does not name — 43 296 bytes at flate2 1.1.10 with
/// miniz_oxide 0.9.1. The read-bytes test fails when it grows past.
pub(crate) const INFLATE_STATE_BYTES: usize = 64 << 10;

/// The elements of the buffers of a [`GzipScratch`] that decoding one tile fills: the
/// inflated bytes, the shuffled ones `GZIP_2` inflates first, and the inflate state.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct GzipCapacities {
    bytes: usize,
    reordered: usize,
    inflate: bool,
}

impl GzipCapacities {
    pub(super) const fn max(self, other: Self) -> Self {
        Self {
            bytes: if self.bytes > other.bytes {
                self.bytes
            } else {
                other.bytes
            },
            reordered: if self.reordered > other.reordered {
                self.reordered
            } else {
                other.reordered
            },
            inflate: self.inflate || other.inflate,
        }
    }

    /// The heap bytes the buffers take.
    pub(super) const fn heap_bytes(&self) -> usize {
        self.bytes + self.reordered + if self.inflate { INFLATE_STATE_BYTES } else { 0 }
    }
}

impl GzipScratch {
    /// Grow the buffers to `capacities`, no room past them. The inflate state the figure
    /// counts is made by the first decode, as a set may meet no gzip tile.
    pub(super) fn reserve(&mut self, capacities: &GzipCapacities) -> Result<()> {
        allocation::try_grow_to(&mut self.bytes, capacities.bytes)?;
        allocation::try_grow_to(&mut self.reordered, capacities.reordered)
    }
}

impl Codec {
    /// What [`Self::decode`] of a tile of `expected` bytes fills.
    pub(super) const fn decode_capacities(self, expected: usize) -> GzipCapacities {
        let (bytes, reordered) = if self.shuffle <= 1 {
            (room(expected), 0)
        } else {
            // The shuffled bytes inflate; the unshuffle fills `bytes` to the size.
            (expected, room(expected))
        };
        GzipCapacities {
            bytes,
            reordered,
            inflate: true,
        }
    }

    /// Inflate a tile's stream to its `expected` bytes, un-shuffled for `GZIP_2`, and lend
    /// them from `scratch`: the inverse of [`Self::encode`].
    pub(super) fn decode<'s>(
        self,
        bytes: &[u8],
        expected: usize,
        scratch: &'s mut GzipScratch,
    ) -> Result<&'s [u8]> {
        let inflate = scratch
            .inflate
            .get_or_insert_with(|| Decompress::new(false));
        if self.shuffle <= 1 {
            gunzip_into(bytes, expected, &mut scratch.bytes, inflate)?;
        } else {
            gunzip_into(bytes, expected, &mut scratch.reordered, inflate)?;
            unshuffle_bytes_into(&scratch.reordered, self.shuffle, &mut scratch.bytes);
        }
        Ok(&scratch.bytes)
    }

    /// Gzip the big-endian samples `raw`, shuffled first for `GZIP_2`, and lend the stream
    /// from `scratch`.
    pub(super) fn encode<'s>(self, raw: &[u8], scratch: &'s mut GzipScratch) -> &'s [u8] {
        let raw = if self.shuffle <= 1 {
            raw
        } else {
            shuffle_bytes_into(raw, self.shuffle, &mut scratch.reordered);
            &scratch.reordered
        };
        let deflate = match &mut scratch.deflate {
            Some(deflate) if deflate.level == self.level => {
                deflate.state.reset();
                &mut deflate.state
            }
            slot => {
                let state = Compress::new(Compression::new(self.level), false);
                &mut slot
                    .insert(Deflate {
                        level: self.level,
                        state,
                    })
                    .state
            }
        };
        let stream = &mut scratch.stream;
        stream.clear();
        stream.reserve(MEMBER_OVERHEAD + deflate_bound(raw.len()));
        stream.extend_from_slice(&member_header(self.level));
        let mut input = raw;
        loop {
            let before = deflate.total_in();
            let status = deflate
                .compress_vec(input, stream, FlushCompress::Finish)
                .expect("deflate into a buffer with room cannot fail");
            let taken = usize::try_from(deflate.total_in() - before).expect("a slice's length");
            input = &input[taken..];
            if status == Status::StreamEnd {
                break;
            }
            stream.reserve(stream.capacity().max(MEMBER_OVERHEAD));
        }
        let mut crc = Crc::new();
        crc.update(raw);
        stream.extend_from_slice(&crc.sum().to_le_bytes());
        // ISIZE is the size modulo 2^32.
        stream.extend_from_slice(&(raw.len() as u32).to_le_bytes());
        &scratch.stream
    }
}

/// The bytes of a gzip member past its deflate data: the 10-byte header and the 8-byte
/// trailer of CRC-32 and size.
const MEMBER_OVERHEAD: usize = 10 + 8;

/// zlib's `deflateBound` of `len` bytes, the deflate data its worst case takes past the
/// stored blocks it falls back to; a stream past it only grows the buffer again.
const fn deflate_bound(len: usize) -> usize {
    len + (len >> 12) + (len >> 14) + (len >> 25) + 13
}

/// The fixed gzip member header (RFC 1952) flate2's `GzEncoder` writes, so a stream is
/// byte for byte the one it gave: no flags, no time, the `XFL` of the slowest or fastest
/// level, and the unknown system 255.
const fn member_header(level: u32) -> [u8; 10] {
    let xfl = match level {
        9.. => 2,
        0 | 1 => 4,
        _ => 0,
    };
    [0x1f, 0x8b, 8, 0, 0, 0, 0, 0, xfl, 255]
}

/// Shuffle `raw` into `width`-byte significance planes (all byte-0s, then all
/// byte-1s, …) — the `GZIP_2` pre-pass. `width ≤ 1` is a no-op.
fn shuffle_bytes_into(raw: &[u8], width: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(raw.len(), 0);
    if width <= 1 {
        out.copy_from_slice(raw);
        return;
    }
    let n = raw.len() / width;
    for p in 0..width {
        for i in 0..n {
            out[p * n + i] = raw[i * width + p];
        }
    }
}

/// Inverse of [`shuffle_bytes_into`]: gather significance planes back into elements.
fn unshuffle_bytes_into(shuffled: &[u8], width: usize, out: &mut Vec<u8>) {
    out.clear();
    out.resize(shuffled.len(), 0);
    match width {
        2 => interleave::<2>(shuffled, out),
        4 => interleave::<4>(shuffled, out),
        8 => interleave::<8>(shuffled, out),
        _ => out.copy_from_slice(shuffled),
    }
}

/// The elements of `W` bytes whose byte `p` is plane `p` of `planes`, into `out`: an
/// element at a time from the `W` planes, which the compiler keeps in registers and
/// vectorizes, where a plane at a time strides through `out`.
fn interleave<const W: usize>(planes: &[u8], out: &mut [u8]) {
    let n = planes.len() / W;
    let planes: [&[u8]; W] = std::array::from_fn(|p| &planes[p * n..(p + 1) * n]);
    for (i, element) in out.as_chunks_mut::<W>().0.iter_mut().enumerate() {
        for (byte, plane) in element.iter_mut().zip(planes) {
            *byte = plane[i];
        }
    }
}

/// Inflate the gzip member `stream` (RFC 1952) through `inflate` to its exact declared
/// size, into `out`, which holds a byte past the size: inflating into it shows an
/// expansion bomb without allocating for it. The member's header is read and its
/// trailer checked here, so the deflate state, made once, inflates every tile.
fn gunzip_into(
    stream: &[u8],
    expected: usize,
    out: &mut Vec<u8>,
    inflate: &mut Decompress,
) -> Result<()> {
    let body = member_body(stream)?;
    inflate.reset(false);
    out.clear();
    allocation::try_reserve_exact(out, room(expected))?;
    let status = inflate
        .decompress_vec(body, out, FlushDecompress::Finish)
        .map_err(CompressedDataDefect::GzipStream)?;
    if out.len() > expected {
        return Err(CompressedDataDefect::GzipOverrun.into());
    }
    if status != Status::StreamEnd {
        return Err(CompressedDataDefect::GzipTruncated.into());
    }
    let trailer = body
        .get(consumed(inflate)..)
        .and_then(|rest| rest.first_chunk::<8>())
        .ok_or(CompressedDataDefect::GzipTruncated)?;
    let mut crc = Crc::new();
    crc.update(out);
    let [crc32, size] = [&trailer[..4], &trailer[4..]]
        .map(|field| u32::from_le_bytes(field.try_into().expect("four bytes")));
    // ISIZE is the size modulo 2^32.
    if crc32 != crc.sum() || size != out.len() as u32 {
        return Err(CompressedDataDefect::GzipChecksum.into());
    }
    if out.len() != expected {
        return Err(SizeUnit::Bytes.mismatch(expected, out.len()));
    }
    Ok(())
}

/// The bytes [`gunzip_into`] reserves to inflate `expected` bytes: one past them.
const fn room(expected: usize) -> usize {
    expected.saturating_add(1)
}

/// The input `inflate` consumed of the member's body.
fn consumed(inflate: &Decompress) -> usize {
    usize::try_from(inflate.total_in()).expect("the input is a slice in memory")
}

/// The deflate data of the gzip member `stream` and what follows it: past the fixed
/// header and the optional extra field, name, comment and header CRC its flags announce.
fn member_body(stream: &[u8]) -> Result<&[u8]> {
    const FHCRC: u8 = 0x02;
    const FEXTRA: u8 = 0x04;
    const FNAME: u8 = 0x08;
    const FCOMMENT: u8 = 0x10;
    const RESERVED: u8 = 0xE0;
    let header = || CompressedDataDefect::GzipHeader.into();
    let Some(&[0x1f, 0x8b, 8, flags, ..]) = stream.first_chunk::<10>() else {
        return Err(header());
    };
    if flags & RESERVED != 0 {
        return Err(header());
    }
    let mut at = 10;
    if flags & FEXTRA != 0 {
        let len = stream.get(at..at + 2).ok_or_else(header)?;
        at += 2 + usize::from(u16::from_le_bytes([len[0], len[1]]));
    }
    for field in [FNAME, FCOMMENT] {
        if flags & field != 0 {
            let end = stream
                .get(at..)
                .and_then(|rest| rest.iter().position(|&byte| byte == 0))
                .ok_or_else(header)?;
            at += end + 1;
        }
    }
    if flags & FHCRC != 0 {
        let stored = stream.get(at..at + 2).ok_or_else(header)?;
        let mut crc = Crc::new();
        crc.update(&stream[..at]);
        if u16::from_le_bytes([stored[0], stored[1]]) != crc.sum() as u16 {
            return Err(header());
        }
        at += 2;
    }
    stream.get(at..).ok_or_else(header)
}

#[cfg(test)]
mod tests;
