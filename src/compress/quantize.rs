//! Floating-point quantization (§10.2) — a port of cfitsio's 3rd-order noise
//! estimator (`FnNoise3_float`), the quantizer (`fits_quantize_float`), and the
//! `SUBTRACTIVE_DITHER_1` random sequence (`fits_init_randoms`).
//!
//! A float tile is mapped to integers `i = NINT((f − zzero) / zscale [+ dither])`
//! and those integers are compressed like an integer image; the decoder inverts
//! `f = (i [− dither]) · zscale + zzero`. Constant tiles (zero noise) can't be
//! quantized and are stored as raw gzip'd floats instead.

use std::sync::OnceLock;

const N_RANDOM: usize = 10000;
const N_RESERVED_VALUES: f64 = 10.0;
const INT_MAX: f64 = 2147483647.0;

/// Quantized integer reserved to mark an undefined (null/NaN) pixel.
pub(super) const NULL_VALUE: i32 = -2147483647;
/// Quantized integer reserved by `SUBTRACTIVE_DITHER_2` to mark exact zero.
pub(super) const ZERO_VALUE: i32 = -2147483646;

/// Float quantization dithering method (`ZQUANTIZ`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum DitherMethod {
    /// `NO_DITHER` — plain linear quantization.
    None,
    /// `SUBTRACTIVE_DITHER_1` — per-pixel dither from the random sequence.
    Subtractive1,
    /// `SUBTRACTIVE_DITHER_2` — like 1, but exact zeros map to [`ZERO_VALUE`].
    Subtractive2,
}

impl DitherMethod {
    fn dithered(self) -> bool {
        !matches!(self, DitherMethod::None)
    }
}

/// The shared dither sequence (cfitsio `fits_init_randoms`): a Park–Miller
/// minstd generator (`a = 16807`, `m = 2³¹−1`) seeded at 1, scaled to `[0, 1)`.
pub(super) fn random_values() -> &'static [f32] {
    static VALUES: OnceLock<Vec<f32>> = OnceLock::new();
    VALUES.get_or_init(|| {
        let a = 16807.0f64;
        let m = 2147483647.0f64;
        let mut seed = 1.0f64;
        let mut v = Vec::with_capacity(N_RANDOM);
        for _ in 0..N_RANDOM {
            let temp = a * seed;
            seed = temp - m * ((temp / m) as i64 as f64);
            v.push((seed / m) as f32);
        }
        // cfitsio invariant: the final seed must be exactly 1043618065.
        debug_assert_eq!(seed as i64, 1_043_618_065);
        v
    })
}

/// `(row − 1) mod N_RANDOM` → the starting index into [`random_values`] for a
/// tile, and the first `nextrand` cursor (cfitsio's `iseed`/`nextrand` setup).
pub(super) struct Dither {
    rand: &'static [f32],
    iseed: usize,
    nextrand: usize,
}

impl Dither {
    pub(super) fn new(irow: i64) -> Self {
        let rand = random_values();
        let iseed = (irow - 1).rem_euclid(N_RANDOM as i64) as usize;
        let nextrand = (rand[iseed] * 500.0) as usize;
        Dither {
            rand,
            iseed,
            nextrand,
        }
    }

    /// The current dither value, then advance the cursor (cfitsio's wrap logic).
    pub(super) fn next(&mut self) -> f64 {
        let r = self.rand[self.nextrand] as f64;
        self.nextrand += 1;
        if self.nextrand == N_RANDOM {
            self.iseed += 1;
            if self.iseed == N_RANDOM {
                self.iseed = 0;
            }
            self.nextrand = (self.rand[self.iseed] * 500.0) as usize;
        }
        r
    }
}

/// Invert a quantized integer plane back to floats (cfitsio `unquantize_i4r4`).
/// `f = (i − dither + 0.5)·scale + zero`, with reserved integers handled first:
/// a value equal to `zblank` becomes `NaN`, and (for `Subtractive2`) [`ZERO_VALUE`]
/// becomes exactly `0.0`. The dither cursor advances per pixel regardless.
pub(super) fn dequantize(
    ints: &[i64],
    scale: f64,
    zero: f64,
    method: DitherMethod,
    irow: i64,
    zblank: Option<i64>,
) -> Vec<f64> {
    let dither2 = method == DitherMethod::Subtractive2;
    let mut d = method.dithered().then(|| Dither::new(irow));
    ints.iter()
        .map(|&v| {
            let r = d.as_mut().map_or(0.0, Dither::next);
            if zblank == Some(v) {
                f64::NAN
            } else if dither2 && v == ZERO_VALUE as i64 {
                0.0
            } else if method.dithered() {
                (v as f64 - r + 0.5) * scale + zero
            } else {
                scale * v as f64 + zero
            }
        })
        .collect()
}

/// Round-to-nearest, ties away from zero (cfitsio `NINT`).
fn nint(x: f64) -> i32 {
    if x >= 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// Background-noise estimate of a tile (cfitsio `FnNoise3_float`).
struct Noise {
    min: f64,
    max: f64,
    noise: f64,
    /// True when at least one finite (non-null) pixel was seen.
    any_good: bool,
}

/// 3rd-order MAD noise: `0.6052697 · median(|2·f(i) − f(i−2) − f(i+2)|)`, taken as
/// the median of per-row medians. Non-finite pixels (NaN/Inf) are treated as nulls
/// and skipped, matching cfitsio's `nullcheck`. Returns `noise = 0` for constant
/// data and `any_good = false` for an all-null tile.
fn noise3(data: &[f64], nx_in: usize, ny_in: usize) -> Noise {
    let (mut nx, mut ny) = (nx_in.max(1), ny_in.max(1));
    if nx < 5 {
        nx *= ny;
        ny = 1;
    }
    let mut xmin = f64::MAX;
    let mut xmax = f64::MIN;
    let mut any_good = false;
    let mut row_meds: Vec<f64> = Vec::with_capacity(ny);

    for jj in 0..ny {
        // Compact the row to its finite pixels — cfitsio advances past nulls, so
        // differences are taken between consecutive *valid* pixels.
        let good: Vec<f64> = data[jj * nx..jj * nx + nx]
            .iter()
            .copied()
            .filter(|v| v.is_finite())
            .collect();
        for &v in &good {
            xmin = xmin.min(v);
            xmax = xmax.max(v);
            any_good = true;
        }
        if good.len() < 5 || nx < 5 {
            continue; // need 4 skipped + ≥1 more for a 3rd-order difference
        }
        let (mut v1, mut v2, mut v3, mut v4) = (good[0], good[1], good[2], good[3]);
        let mut diffs: Vec<f64> = Vec::with_capacity(good.len());
        for &v5 in &good[4..] {
            if !(v1 == v2 && v2 == v3 && v3 == v4 && v4 == v5) {
                diffs.push((2.0 * v3 - v1 - v5).abs());
            }
            v1 = v2;
            v2 = v3;
            v3 = v4;
            v4 = v5;
        }
        if !diffs.is_empty() {
            row_meds.push(lower_median(&mut diffs));
        }
    }

    let noise = if row_meds.is_empty() {
        0.0
    } else {
        0.6052697 * proper_median(&mut row_meds)
    };
    Noise {
        min: xmin,
        max: xmax,
        noise,
        any_good,
    }
}

/// Lower median (element at index `(n−1)/2` of the sorted values), matching
/// cfitsio's per-row `quick_select_float`.
fn lower_median(v: &mut [f64]) -> f64 {
    // `noise3` filters to finite values before any median, so `partial_cmp` is a
    // total order here and the `unwrap` cannot fire on a NaN.
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[(v.len() - 1) / 2]
}

/// Proper median (average of the two middle values for even counts), matching
/// cfitsio's final cross-row `qsort` median.
fn proper_median(v: &mut [f64]) -> f64 {
    // Finite input (see `lower_median`), so the comparison never sees a NaN.
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
}

/// A quantized tile: the integer plane plus the `BSCALE`/`BZERO` (`ZSCALE`/`ZZERO`)
/// that invert it, and whether any pixel was a null (mapped to [`NULL_VALUE`]).
pub(super) struct Quantized {
    pub(super) idata: Vec<i32>,
    pub(super) bscale: f64,
    pub(super) bzero: f64,
    pub(super) has_null: bool,
}

/// Quantize a float tile (cfitsio `fits_quantize_float`). `qlevel` is the noise
/// divisor (0 ⇒ default of 4). `method` selects dithering; `irow` drives the
/// subtractive-dither sequence. Non-finite pixels become [`NULL_VALUE`] and, for
/// `Subtractive2`, exact zeros become [`ZERO_VALUE`]. Returns `None` when the tile
/// can't be quantized (no finite data, constant data, or a range wider than the
/// int domain) — the caller then stores the raw floats losslessly.
pub(super) fn quantize_tile(
    fdata: &[f64],
    nx: usize,
    ny: usize,
    qlevel: f64,
    method: DitherMethod,
    irow: i64,
) -> Option<Quantized> {
    let n = nx * ny;
    if n <= 1 {
        return None;
    }
    let est = noise3(fdata, nx, ny);
    if !est.any_good {
        return None; // all-null tile → store raw (preserves the NaNs exactly)
    }
    let delta = if qlevel == 0.0 {
        est.noise / 4.0
    } else {
        est.noise / qlevel
    };
    if delta == 0.0 {
        return None;
    }
    if (est.max - est.min) / delta > 2.0 * INT_MAX - N_RESERVED_VALUES {
        return None;
    }

    let has_null = fdata.iter().take(n).any(|v| !v.is_finite());
    let dither2 = method == DitherMethod::Subtractive2;
    // When nulls are present (or for DITHER_2), shift the range above the reserved
    // values so NULL_VALUE/ZERO_VALUE never collide with real data; otherwise fudge
    // the zero point to an integer multiple of delta (stable across re-compression).
    let zeropt = if has_null || dither2 {
        est.min - delta * (NULL_VALUE as f64 + N_RESERVED_VALUES)
    } else if (est.max - est.min) / delta < INT_MAX - N_RESERVED_VALUES {
        let iqfactor = (est.min / delta + 0.5) as i64;
        iqfactor as f64 * delta
    } else {
        (est.min + est.max) / 2.0
    };

    let mut idata = vec![0i32; n];
    let mut d = method.dithered().then(|| Dither::new(irow));
    for (i, &f) in fdata.iter().enumerate().take(n) {
        // The dither cursor advances per pixel regardless of branch taken.
        let r = d.as_mut().map_or(0.0, Dither::next);
        idata[i] = if !f.is_finite() {
            NULL_VALUE
        } else if dither2 && f == 0.0 {
            ZERO_VALUE
        } else if method.dithered() {
            nint((f - zeropt) / delta + r - 0.5)
        } else {
            nint((f - zeropt) / delta)
        };
    }
    Some(Quantized {
        idata,
        bscale: delta,
        bzero: zeropt,
        has_null,
    })
}
