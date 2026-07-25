//! `PLIO_1` tile codec — the IRAF PLIO line-list RLE (a port of cfitsio's
//! `pl_l2pi`/`pl_p2li`). The compressed cell is an i16 instruction list (opcode in
//! the top nibble, 12-bit data) that programs a run-length mask, tracking a
//! current "high" value `pv`. PLIO is a *mask* codec: values must be non-negative
//! and fit in 24 bits; the encoder rejects values outside that lossless domain.

use crate::error::FitsError;
use crate::error::Result;

/// Encode `values` (one tile of non-negative mask pixels) as an IRAF PLIO
/// line list — a port of cfitsio's `pl_p2li` with `xs = 1`. The returned i16 list
/// round-trips through [`plio_decode_be_into`]. Values outside `0..=0xFF_FFFF`
/// are rejected because the format cannot preserve them.
pub(super) fn plio_encode(values: &[i64]) -> Result<Vec<i16>> {
    if let Some((index, &value)) = values
        .iter()
        .enumerate()
        .find(|&(_, value)| !(0..=0xFF_FFFF).contains(value))
    {
        return Err(FitsError::PlioValueOutOfRange { index, value });
    }

    // Header words (1-based lldst[1..=7]): the `-100` at index 3 selects the
    // 30-bit-length form the decoder reads from words 4/5; index 2 (=7) is the
    // header length, so instructions begin at word 8.
    let mut ll: Vec<i16> = vec![0, 7, -100, 0, 0, 0, 0];
    let npix = values.len();
    if npix == 0 {
        return Ok(ll);
    }
    let pix = |i1: usize| values[i1 - 1];

    let xs = 1usize;
    let xe = xs + npix - 1;
    let mut pv = pix(xs);
    let mut x1 = xs as i64;
    let mut iz = xs as i64;
    let mut hi: i64 = 1;
    let mut nv: i64 = 0;

    for ip in xs..=xe {
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
                            let last = ll.last_mut().unwrap();
                            *last |= 16384;
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
                    let last = ll.last_mut().unwrap();
                    *last = (*last as i64 + 20481) as i16;
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
    }

    // Total list length (= cfitsio's `op - 1`) split across words 4/5.
    let total = ll.len();
    ll[3] = (total % 32768) as i16;
    ll[4] = (total / 32768) as i16;
    Ok(ll)
}

pub(super) fn plio_decode_be_into(bytes: &[u8], npix: usize, px: &mut Vec<i64>) -> Result<()> {
    if !bytes.len().is_multiple_of(2) {
        return Err(FitsError::UnexpectedEof);
    }
    decode_words(BeWords(bytes), npix, px)
}

#[derive(Debug, Clone, Copy)]
struct BeWords<'a>(&'a [u8]);

impl BeWords<'_> {
    fn len(self) -> usize {
        self.0.len() / 2
    }

    fn get(self, index: usize) -> Option<i16> {
        let start = index.checked_mul(2)?;
        Some(i16::from_be_bytes(
            self.0.get(start..start + 2)?.try_into().unwrap(),
        ))
    }
}

fn decode_words(ll: BeWords<'_>, npix: usize, px: &mut Vec<i64>) -> Result<()> {
    px.clear();
    px.resize(npix, 0);
    if npix == 0 {
        return Ok(());
    }
    // List header: a positive ll[2] gives the length directly (older form); else
    // the length is a 30-bit value in ll[3..5] and instructions start at ll[1]+1.
    let v3 = i32::from(ll.get(2).ok_or(FitsError::UnexpectedEof)?);
    let (lllen, llfirst) = if v3 > 0 {
        (v3 as usize, 4usize)
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
        return Err(invalid_stream("invalid list bounds"));
    }

    let xe = npix as i64; // pixel coordinates are 1-based; xs = 1
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
        let word = ll.get(ip - 1).ok_or(FitsError::UnexpectedEof)?;
        let word = word as u16 as i64;
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
                let next = ll.get(ip).ok_or(FitsError::UnexpectedEof)? as u16 as i64;
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
            _ => return Err(invalid_stream("invalid opcode")),
        }
        if x1 > xe {
            break;
        }
        ip += 1;
    }
    Ok(())
}

fn nonnegative_word(ll: BeWords<'_>, index: usize) -> Result<usize> {
    let value = ll.get(index).ok_or(FitsError::UnexpectedEof)?;
    usize::try_from(value).map_err(|_| invalid_stream("negative list header value"))
}

fn invalid_stream(detail: &str) -> FitsError {
    FitsError::UnsupportedCompression {
        name: format!("PLIO_1: {detail}"),
    }
}
