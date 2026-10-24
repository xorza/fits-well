//! `GZIP_1` and `GZIP_2` tile codecs (via `flate2`).

use std::io::Read;
use std::io::Write;

use crate::error::CompressedDataDefect;
use crate::error::Result;
use crate::error::SizeUnit;
use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;

/// Default deflate level used by [`crate::image::Gzip::default`]. Level 1 favors write
/// speed (gzip was the slowest compress path at the higher default); construct
/// [`crate::image::Gzip::new`] or [`crate::image::Gzip::shuffled`] with a higher level for a
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

/// The buffers a gzip encode and decode reuse: the decoded bytes, the shuffled ones, and
/// the stream.
#[derive(Debug, Default)]
pub(super) struct GzipScratch {
    bytes: Vec<u8>,
    reordered: Vec<u8>,
    stream: Vec<u8>,
}

impl Codec {
    /// Inflate a tile's stream to its `expected` bytes, un-shuffled for `GZIP_2`, and lend
    /// them from `scratch`: the inverse of [`Self::encode`].
    pub(super) fn decode<'s>(
        self,
        bytes: &[u8],
        expected: usize,
        scratch: &'s mut GzipScratch,
    ) -> Result<&'s [u8]> {
        if self.shuffle <= 1 {
            gunzip_into(bytes, expected, &mut scratch.bytes)?;
        } else {
            gunzip_into(bytes, expected, &mut scratch.reordered)?;
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

/// Inflate a gzip stream to its exact declared size. Reading one byte past the
/// expected size detects expansion bombs without allowing an unbounded allocation.
/// The stream is already in memory, so a read that fails is a stream that does not
/// inflate.
fn gunzip_into(bytes: &[u8], expected: usize, out: &mut Vec<u8>) -> Result<()> {
    out.clear();
    GzDecoder::new(bytes)
        .take(expected.saturating_add(1) as u64)
        .read_to_end(out)
        .map_err(CompressedDataDefect::GzipStream)?;
    if out.len() > expected {
        return Err(CompressedDataDefect::GzipOverrun.into());
    }
    if out.len() != expected {
        return Err(SizeUnit::Bytes.mismatch(expected, out.len()));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::compress::gzip;

    use crate::error::{CompressedDataDefect, FitsError, SizeUnit};

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
        // Bounded at 1 KiB (a small tile): inflating to 100 KB overruns → error.
        assert!(matches!(
            gzip::gunzip_into(&bomb, 1024, &mut decoded),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipOverrun,
            })
        ));
        // One byte short of the true size still overruns (the +1 detection boundary).
        assert!(matches!(
            gzip::gunzip_into(&bomb, big.len() - 1, &mut decoded),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipOverrun
            })
        ));
        // Bounded at the true size: decodes to exactly the original bytes.
        gzip::gunzip_into(&bomb, big.len(), &mut decoded).unwrap();
        assert_eq!(decoded, big);

        assert!(matches!(
            gzip::gunzip_into(b"not a gzip stream", 4, &mut decoded),
            Err(FitsError::CorruptCompressedData {
                defect: CompressedDataDefect::GzipStream(_),
            })
        ));

        let short = gzip::Codec {
            level: 1,
            shuffle: 1,
        }
        .encode(&[1, 2, 3], &mut scratch)
        .to_vec();
        assert!(matches!(
            gzip::gunzip_into(&short, 4, &mut decoded),
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
}
