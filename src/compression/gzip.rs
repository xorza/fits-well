//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use std::io::Write;

use crate::allocation;
use crate::fits_error::CompressedDataDefect;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use flate2::write::GzEncoder;
use flate2::{Compression, Crc, Decompress, FlushDecompress, Status};

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
/// stream, and the inflate state, made by the first decode and reset by each after it.
#[derive(Debug, Default)]
pub(super) struct GzipScratch {
    bytes: Vec<u8>,
    reordered: Vec<u8>,
    stream: Vec<u8>,
    inflate: Option<Decompress>,
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
        scratch.stream.clear();
        let mut enc = GzEncoder::new(&mut scratch.stream, Compression::new(self.level));
        enc.write_all(raw).expect("gzip into a Vec cannot fail");
        enc.finish().expect("gzip finish into a Vec cannot fail");
        &scratch.stream
    }
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
    if width <= 1 {
        out.copy_from_slice(shuffled);
        return;
    }
    let n = shuffled.len() / width;
    for p in 0..width {
        for i in 0..n {
            out[i * width + p] = shuffled[p * n + i];
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
mod tests {
    use std::io::Write;

    use flate2::{Compression, Crc, Decompress, GzBuilder};

    use crate::compression::gzip;

    use crate::fits_error::{CompressedDataDefect, FitsError, SizeUnit};

    #[test]
    fn gunzip_rejects_a_stream_larger_than_its_tile() {
        // A decompression bomb: a tiny gzip stream inflating far past the tile's known
        // byte size must be rejected, not allocated unbounded.
        let big = vec![0u8; 100_000];
        let mut scratch = gzip::GzipScratch::default();
        let bomb = gzip::Codec {
            level: 9,
            shuffle: 1,
        }
        .encode(&big, &mut scratch)
        .to_vec();
        assert!(bomb.len() < 1000, "an all-zero buffer compresses tiny");
        let mut decoded = Vec::new();
        let mut inflate = Decompress::new(false);
        // Bounded at 1 KiB (a small tile): inflating to 100 KB overruns → error.
        assert!(matches!(
            gzip::gunzip_into(&bomb, 1024, &mut decoded, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipOverrun,
            })
        ));
        // One byte short of the true size still overruns (the +1 detection boundary).
        assert!(matches!(
            gzip::gunzip_into(&bomb, big.len() - 1, &mut decoded, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipOverrun
            })
        ));
        // Bounded at the true size: decodes to exactly the original bytes.
        gzip::gunzip_into(&bomb, big.len(), &mut decoded, &mut inflate).unwrap();
        assert_eq!(decoded, big);

        assert!(matches!(
            gzip::gunzip_into(b"not a gzip stream", 4, &mut decoded, &mut inflate),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipHeader,
            })
        ));

        let short = gzip::Codec {
            level: 1,
            shuffle: 1,
        }
        .encode(&[1, 2, 3], &mut scratch)
        .to_vec();
        assert!(matches!(
            gzip::gunzip_into(&short, 4, &mut decoded, &mut inflate),
            Err(FitsError::DataSizeMismatch {
                unit: SizeUnit::Bytes,
                expected: 4,
                got: 3
            })
        ));

        let raw = [1, 2, 3, 4, 5, 6];
        let mut shuffled = Vec::new();
        gzip::shuffle_bytes_into(&raw, 2, &mut shuffled);
        assert_eq!(shuffled, [1, 3, 5, 2, 4, 6]);
        gzip::unshuffle_bytes_into(&shuffled, 2, &mut decoded);
        assert_eq!(decoded, raw);
        gzip::shuffle_bytes_into(&raw, 1, &mut shuffled);
        assert_eq!(shuffled, raw, "width-one shuffle is an exact no-op");
    }

    /// A member of `raw` whose header names a file, a comment and an extra field.
    fn member_with_fields(raw: &[u8]) -> Vec<u8> {
        let mut encoder = GzBuilder::new()
            .filename("tile")
            .comment("a comment")
            .extra(vec![1, 2, 3])
            .write(Vec::new(), Compression::fast());
        encoder.write_all(raw).unwrap();
        encoder.finish().unwrap()
    }

    /// The header's every optional field is skipped, and a header CRC, which flate2 never
    /// writes, is checked: its low 16 bits of the CRC-32 of the header before it.
    #[test]
    fn gunzip_reads_past_every_header_field_and_checks_the_header_crc() {
        let raw: Vec<u8> = (0..200u8).collect();
        let mut out = Vec::new();
        let mut inflate = Decompress::new(false);
        let fields = member_with_fields(&raw);
        gzip::gunzip_into(&fields, raw.len(), &mut out, &mut inflate).unwrap();
        assert_eq!(out, raw);

        // A plain 10-byte header, its FHCRC flag set and the CRC inserted after it.
        let plain = gzip::Codec {
            level: 1,
            shuffle: 1,
        }
        .encode(&raw, &mut gzip::GzipScratch::default())
        .to_vec();
        let mut header = plain[..10].to_vec();
        header[3] |= 0x02;
        let mut crc = Crc::new();
        crc.update(&header);
        let good = crc.sum() as u16;
        let with_crc = |crc: u16| [header.as_slice(), &crc.to_le_bytes(), &plain[10..]].concat();
        gzip::gunzip_into(&with_crc(good), raw.len(), &mut out, &mut inflate).unwrap();
        assert_eq!(out, raw);
        for member in [with_crc(good ^ 1), {
            let mut reserved = plain.clone();
            reserved[3] |= 0x20;
            reserved
        }] {
            assert!(matches!(
                gzip::gunzip_into(&member, raw.len(), &mut out, &mut inflate),
                Err(FitsError::CorruptCompressedData {
                    defect: CompressedDataDefect::GzipHeader
                })
            ));
        }
    }

    /// The trailer's CRC-32 and size are each checked, and a member cut short anywhere
    /// past its header is truncated.
    #[test]
    fn gunzip_checks_the_trailer_and_refuses_a_cut_member() {
        let raw: Vec<u8> = (0..200u8).rev().collect();
        let member = gzip::Codec {
            level: 1,
            shuffle: 1,
        }
        .encode(&raw, &mut gzip::GzipScratch::default())
        .to_vec();
        let mut out = Vec::new();
        let mut inflate = Decompress::new(false);
        let trailer = member.len() - 8;
        for field in [trailer, trailer + 4] {
            let mut broken = member.clone();
            broken[field] ^= 0x01;
            assert!(matches!(
                gzip::gunzip_into(&broken, raw.len(), &mut out, &mut inflate),
                Err(FitsError::CorruptCompressedData {
                    defect: CompressedDataDefect::GzipChecksum
                })
            ));
        }
        for cut in [trailer + 7, trailer, 12] {
            assert!(matches!(
                gzip::gunzip_into(&member[..cut], raw.len(), &mut out, &mut inflate),
                Err(FitsError::CorruptCompressedData {
                    defect: CompressedDataDefect::GzipTruncated
                })
            ));
        }
        gzip::gunzip_into(&member, raw.len(), &mut out, &mut inflate).unwrap();
        assert_eq!(out, raw, "the state resets after each failure");
    }
}
