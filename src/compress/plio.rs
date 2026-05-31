//! `PLIO_1` tile codec — the IRAF PLIO line-list RLE (a port of cfitsio's
//! `pl_l2pi`). The compressed cell is an i16 instruction list (opcode in the top
//! nibble, 12-bit data) that programs a run-length mask, tracking a current
//! "high" value `pv`.

/// Decode an IRAF PLIO line list into `npix` mask values.
pub(super) fn plio_decode(ll: &[i16], npix: usize) -> Vec<i64> {
    let mut px = vec![0i64; npix];
    if npix == 0 {
        return px;
    }
    // List header: a positive ll[2] gives the length directly (older form); else
    // the length is a 30-bit value in ll[3..5] and instructions start at ll[1]+1.
    let v3 = ll.get(2).copied().unwrap_or(0) as i32;
    let (lllen, llfirst) = if v3 > 0 {
        (v3 as usize, 4usize)
    } else {
        let lo = ll.get(3).copied().unwrap_or(0) as u16 as usize;
        let hi = ll.get(4).copied().unwrap_or(0) as u16 as usize;
        let start = ll.get(1).copied().unwrap_or(0) as u16 as usize + 1;
        ((hi << 15) + lo, start)
    };
    if lllen == 0 {
        return px;
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
        let Some(&word) = ll.get(ip - 1) else { break };
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
                let next = ll.get(ip).copied().unwrap_or(0) as u16 as i64;
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
            _ => {}
        }
        if x1 > xe {
            break;
        }
        ip += 1;
    }
    px
}
