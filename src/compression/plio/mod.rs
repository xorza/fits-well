//! `PLIO_1` tile codec — the IRAF PLIO line-list RLE (a port of cfitsio's
//! `pl_l2pi`/`pl_p2li`). The compressed cell is an i16 instruction list (opcode in
//! the top nibble, 12-bit data) that programs a run-length mask, tracking a
//! current "high" value `pv`. PLIO is a *mask* codec: values must be non-negative
//! and fit in 24 bits; the encoder rejects values outside that lossless domain.

use crate::compression::plane::IntSamples;
use crate::fits_error::CompressionFeature;
use crate::fits_error::FitsError;
use crate::fits_error::PlioDefect;
use crate::fits_error::Result;
use std::ops::RangeInclusive;

/// A `PLIO_1` stream's parameter: the `offset` its stream adds to each sample.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct Codec {
    pub(super) offset: i64,
}

/// The values a PLIO stream holds: unsigned, in 24 bits.
const STREAM_RANGE: RangeInclusive<i64> = 0..=0xFF_FFFF;

impl Codec {
    /// Refuses the first sample of an image, in the order of its samples, whose stream
    /// value, the sample plus the offset, the format cannot hold. The whole image is checked
    /// before its tiles, so the sample named is the same for any tiling, and with or
    /// without `parallel`.
    pub(super) fn check(self, samples: IntSamples<'_>) -> Result<()> {
        match samples {
            IntSamples::U8(values) => self.check_values(values, u8::MIN.into()..=u8::MAX.into()),
            IntSamples::I16(values) => self.check_values(values, i16::MIN.into()..=i16::MAX.into()),
            IntSamples::I32(values) => self.check_values(values, i32::MIN.into()..=i32::MAX.into()),
            IntSamples::I64(values) => self.check_values(values, i64::MIN..=i64::MAX),
        }
    }

    /// The check of `values`, of a type that holds `held`: when the stream holds each
    /// value of the type, no sample is read.
    fn check_values<T: Copy + Into<i64>>(
        self,
        values: &[T],
        held: RangeInclusive<i64>,
    ) -> Result<()> {
        let streamed = |value: i64| {
            value
                .checked_add(self.offset)
                .is_some_and(|value| STREAM_RANGE.contains(&value))
        };
        if streamed(*held.start()) && streamed(*held.end()) {
            return Ok(());
        }
        let outside = |&value: &T| !streamed(value.into());
        // A fold without an early exit vectorizes; `position` runs only to name the
        // sample of an image already refused.
        if !values
            .iter()
            .fold(false, |found, value| found | outside(value))
        {
            return Ok(());
        }
        let index = values.iter().position(outside).unwrap();
        Err(FitsError::PlioValueOutOfRange {
            index,
            value: values[index].into(),
        })
    }

    /// Encode `values` plus the offset (one tile of non-negative mask pixels) as an IRAF
    /// PLIO line list — a port of cfitsio's `pl_p2li` with `xs = 1` — appending its
    /// big-endian i16 words to `out`. The list round-trips through [`Self::decode`].
    /// The values are within the format's range, as [`Self::check`] made sure.
    pub(super) fn encode(self, values: &[i64], out: &mut Vec<u8>) -> Result<()> {
        let offset = self.offset;
        debug_assert!(
            values
                .iter()
                .all(|&value| STREAM_RANGE.contains(&(value + offset))),
            "PLIO samples checked before the tiles"
        );

        // Header words (1-based lldst[1..=7]): the `-100` at index 3 selects the
        // 30-bit-length form the decoder reads from words 4/5; index 2 (=7) is the
        // header length, so instructions begin at word 8.
        let mut ll = Words::new(out);
        for word in [0, 7, -100, 0, 0, 0, 0] {
            ll.push(word);
        }
        let npix = values.len();
        if npix == 0 {
            return Ok(());
        }
        let pix = |i1: usize| values[i1 - 1] + offset;

        let xs = 1usize;
        let xe = xs + npix - 1;
        let mut pv = pix(xs);
        let mut x1 = xs as i64;
        let mut iz = xs as i64;
        let mut hi: i64 = 1;
        let mut nv: i64 = 0;

        let mut ip = xs;
        while ip <= xe {
            // A pixel equal to the one before it only extends the run, so a run is
            // passed whole.
            ip += run_len(&values[ip..], pv - offset);
            let mut flush = true;
            if ip < xe {
                nv = pix(ip + 1);
                if nv == pv {
                    flush = false; // extend the current run
                } else if pv == 0 {
                    pv = nv;
                    x1 = (ip + 1) as i64;
                    flush = false;
                }
            } else if pv == 0 {
                x1 = xe as i64 + 1;
            }

            if flush {
                let mut np = ip as i64 - x1 + 1; // high-pixel count in this segment
                let mut nz = x1 - iz; // leading zero count
                let mut done = false;

                if pv > 0 {
                    let dv = pv - hi; // change in the "high" value since last segment
                    if dv != 0 {
                        hi = pv;
                        if dv.abs() > 4095 {
                            // Two-word absolute set: low 12 bits (opcode 1) then high bits.
                            ll.push(((pv & 4095) + 4096) as i16);
                            ll.push((pv / 4096) as i16);
                        } else {
                            // One-word relative adjust: opcode 2 (up) or 3 (down).
                            ll.push(if dv < 0 {
                                (-dv + 12288) as i16
                            } else {
                                (dv + 8192) as i16
                            });
                            // A lone high pixel with no leading zeros folds into the
                            // adjust word as opcode 6/7 (single high pixel).
                            if np == 1 && nz == 0 {
                                ll.update_last(|last| last | 16384);
                                done = true;
                            }
                        }
                    }
                }

                if !done && nz > 0 {
                    while nz > 0 {
                        ll.push(nz.min(4095) as i16);
                        nz -= 4095;
                    }
                    // A lone high pixel after the zeros folds into the last zero word
                    // as opcode 5 (zero run whose final pixel is set high).
                    if np == 1 && pv > 0 {
                        ll.update_last(|last| (i64::from(last) + 20481) as i16);
                        done = true;
                    }
                }

                if !done {
                    while np > 0 {
                        ll.push((np.min(4095) + 16384) as i16); // opcode 4: high run
                        np -= 4095;
                    }
                }

                x1 = (ip + 1) as i64;
                iz = x1;
                pv = nv;
            }
            ip += 1;
        }

        // Total list length (= cfitsio's `op - 1`) split across words 4/5.
        let [low, high] = length_words(ll.len())?;
        ll.set(3, low);
        ll.set(4, high);
        Ok(())
    }

    /// Decode the big-endian line list `bytes` into `count` values, less the offset, in
    /// `px`.
    pub(super) fn decode(self, bytes: &[u8], count: usize, px: &mut Vec<i64>) -> Result<()> {
        if !bytes.len().is_multiple_of(2) {
            return Err(FitsError::UnexpectedEof);
        }
        decode_words(BeWords(bytes), count, px)?;
        if self.offset != 0 {
            for value in px.iter_mut() {
                *value -= self.offset;
            }
        }
        Ok(())
    }
}

/// The big-endian i16 words of a line list, appended to a buffer from `start` on.
#[derive(Debug)]
struct Words<'a> {
    out: &'a mut Vec<u8>,
    start: usize,
}

impl<'a> Words<'a> {
    const fn new(out: &'a mut Vec<u8>) -> Self {
        let start = out.len();
        Self { out, start }
    }

    fn push(&mut self, word: i16) {
        self.out.extend_from_slice(&word.to_be_bytes());
    }

    const fn len(&self) -> usize {
        (self.out.len() - self.start) / 2
    }

    fn set(&mut self, index: usize, word: i16) {
        let at = self.start + 2 * index;
        self.out[at..at + 2].copy_from_slice(&word.to_be_bytes());
    }

    fn update_last(&mut self, update: impl FnOnce(i16) -> i16) {
        let at = self.out.len() - 2;
        let last = i16::from_be_bytes([self.out[at], self.out[at + 1]]);
        self.set((at - self.start) / 2, update(last));
    }
}

/// The count of leading `values` equal to `value`. Whole chunks compare without an early
/// exit, which vectorizes; the first chunk that differs is searched one value at a time.
#[expect(
    clippy::needless_bitwise_bool,
    reason = "a lazy `&&` puts a branch in each compare, which stops the vectorizing"
)]
fn run_len(values: &[i64], value: i64) -> usize {
    let (chunks, _) = values.as_chunks::<16>();
    let whole = chunks
        .iter()
        .take_while(|chunk| chunk.iter().fold(true, |same, &v| same & (v == value)))
        .count()
        * 16;
    whole + values[whole..].iter().take_while(|&&v| v == value).count()
}

/// A list length as the two 15-bit header words that hold it, low first; a list of
/// 2³⁰ words or more has no such form.
fn length_words(total: usize) -> Result<[i16; 2]> {
    let word = |value: usize| i16::try_from(value).ok();
    match (word(total % 32768), word(total / 32768)) {
        (Some(low), Some(high)) => Ok([low, high]),
        _ => Err(FitsError::UnsupportedCompression {
            feature: CompressionFeature::PlioListLength { words: total },
        }),
    }
}

#[derive(Debug, Clone, Copy)]
struct BeWords<'a>(&'a [u8]);

impl BeWords<'_> {
    const fn len(self) -> usize {
        self.0.len() / 2
    }

    /// The word at `index`, or the end of the stream: the error is built on its own path,
    /// not on each word an eager `ok_or` would build and drop.
    fn word(self, index: usize) -> Result<i16> {
        let Some(bytes) = index
            .checked_mul(2)
            .and_then(|start| self.0.get(start..)?.first_chunk::<2>())
        else {
            return Err(FitsError::UnexpectedEof);
        };
        Ok(i16::from_be_bytes(*bytes))
    }
}

#[expect(
    clippy::cast_sign_loss,
    reason = "an output position runs from 1 to the pixel count, so position − 1 indexes `px`"
)]
fn decode_words(ll: BeWords<'_>, npix: usize, px: &mut Vec<i64>) -> Result<()> {
    px.clear();
    px.resize(npix, 0);
    if npix == 0 {
        return Ok(());
    }
    // List header: a positive ll[2] gives the length directly (older form); else
    // the length is a 30-bit value in ll[3..5] and instructions start at ll[1]+1.
    let v3 = i32::from(ll.word(2)?);
    let (lllen, llfirst) = if v3 > 0 {
        (usize::try_from(v3).unwrap(), 4usize)
    } else {
        let lo = nonnegative_word(ll, 3)?;
        let hi = nonnegative_word(ll, 4)?;
        let start = nonnegative_word(ll, 1)? + 1;
        ((hi << 15) + lo, start)
    };
    if lllen > ll.len() {
        return Err(FitsError::UnexpectedEof);
    }
    if lllen == 0 || llfirst > lllen + 1 {
        return Err(PlioDefect::ListBounds.into());
    }

    let xe = i64::try_from(npix).expect("a tile's pixel count fits i64"); // 1-based; xs = 1
    let mut skip_word = false;
    let mut op = 1i64; // next output position (1-based)
    let mut x1 = 1i64; // current pixel coordinate
    let mut pv = 1i64; // current "high" value
    let mut ip = llfirst;
    while ip <= lllen {
        if skip_word {
            skip_word = false;
            ip += 1;
            continue;
        }
        let word = ll.word(ip - 1)?;
        let word = i64::from(word.cast_unsigned());
        let opcode = word >> 12;
        let data = word & 4095;
        match opcode {
            // Run of `data` pixels: opcode 4 = high (pv), 0/5 = zero (opcode 5
            // sets the final pixel of the run to pv).
            0 | 4 | 5 => {
                let x2 = x1 + data - 1;
                let i2 = x2.min(xe);
                let np = i2 - x1 + 1;
                if np > 0 {
                    let otop = op + np - 1;
                    if opcode == 4 {
                        for i in op..=otop {
                            px[(i - 1) as usize] = pv;
                        }
                    } else if opcode == 5 && i2 == x2 {
                        px[(otop - 1) as usize] = pv;
                    }
                    op = otop + 1;
                }
                x1 = x2 + 1;
            }
            1 => {
                // Set pv absolutely from this word's data plus the next word.
                if ip >= lllen {
                    return Err(FitsError::UnexpectedEof);
                }
                let next = i64::from(ll.word(ip)?.cast_unsigned());
                pv = (next << 12) + data;
                skip_word = true;
            }
            2 => pv += data,
            3 => pv -= data,
            // Single high pixel after adjusting pv.
            6 => {
                pv += data;
                if x1 <= xe {
                    px[(op - 1) as usize] = pv;
                    op += 1;
                }
                x1 += 1;
            }
            7 => {
                pv -= data;
                if x1 <= xe {
                    px[(op - 1) as usize] = pv;
                    op += 1;
                }
                x1 += 1;
            }
            _ => return Err(PlioDefect::Opcode.into()),
        }
        if x1 > xe {
            break;
        }
        ip += 1;
    }
    Ok(())
}

fn nonnegative_word(ll: BeWords<'_>, index: usize) -> Result<usize> {
    let value = ll.word(index)?;
    usize::try_from(value).map_err(|_| PlioDefect::NegativeHeader.into())
}

#[cfg(test)]
mod tests;
