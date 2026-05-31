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

/// Round-to-nearest, ties away from zero (cfitsio `NINT`).
fn nint(x: f64) -> i32 {
    if x >= 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// Background-noise estimate of a tile (cfitsio `FnNoise3_float`, no null check).
struct Noise {
    min: f64,
    max: f64,
    noise: f64,
}

/// 3rd-order MAD noise: `0.6052697 · median(|2·f(i) − f(i−2) − f(i+2)|)`, taken as
/// the median of per-row medians. Returns `noise = 0` for constant data.
fn noise3(data: &[f64], nx_in: usize, ny_in: usize) -> Noise {
    let (mut nx, mut ny) = (nx_in.max(1), ny_in.max(1));
    if nx < 5 {
        nx *= ny;
        ny = 1;
    }
    let mut xmin = f64::MAX;
    let mut xmax = f64::MIN;
    if nx < 5 {
        for &v in data.iter().take(nx) {
            xmin = xmin.min(v);
            xmax = xmax.max(v);
        }
        return Noise {
            min: xmin,
            max: xmax,
            noise: 0.0,
        };
    }

    let mut row_meds: Vec<f64> = Vec::with_capacity(ny);
    for jj in 0..ny {
        let row = &data[jj * nx..jj * nx + nx];
        let (mut v1, mut v2, mut v3, mut v4) = (row[0], row[1], row[2], row[3]);
        for &v in [v1, v2, v3, v4].iter() {
            xmin = xmin.min(v);
            xmax = xmax.max(v);
        }
        let mut diffs: Vec<f64> = Vec::with_capacity(nx);
        for &v5 in &row[4..] {
            xmin = xmin.min(v5);
            xmax = xmax.max(v5);
            if !(v1 == v2 && v2 == v3 && v3 == v4 && v4 == v5) {
                diffs.push((2.0 * v3 - v1 - v5).abs());
            }
            v1 = v2;
            v2 = v3;
            v3 = v4;
            v4 = v5;
        }
        if diffs.is_empty() {
            continue;
        }
        row_meds.push(lower_median(&mut diffs));
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
    }
}

/// Lower median (element at index `(n−1)/2` of the sorted values), matching
/// cfitsio's per-row `quick_select_float`.
fn lower_median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[(v.len() - 1) / 2]
}

/// Proper median (average of the two middle values for even counts), matching
/// cfitsio's final cross-row `qsort` median.
fn proper_median(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    (v[(v.len() - 1) / 2] + v[v.len() / 2]) / 2.0
}

/// A quantized tile: the integer plane plus the `BSCALE`/`BZERO` (`ZSCALE`/`ZZERO`)
/// that invert it.
pub(super) struct Quantized {
    pub(super) idata: Vec<i32>,
    pub(super) bscale: f64,
    pub(super) bzero: f64,
}

/// Quantize a float tile (cfitsio `fits_quantize_float`, no-null branch). `qlevel`
/// is the noise divisor (0 ⇒ default of 4). When `dither` is set, `irow` drives
/// the subtractive-dither sequence. Returns `None` when the tile can't be
/// quantized (constant data, or a range wider than the int domain).
pub(super) fn quantize_tile(
    fdata: &[f64],
    nx: usize,
    ny: usize,
    qlevel: f64,
    dither: bool,
    irow: i64,
) -> Option<Quantized> {
    let n = nx * ny;
    if n <= 1 {
        return None;
    }
    let est = noise3(fdata, nx, ny);
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

    // Zero point fudged to an integer multiple of delta so repeated compress/
    // decompress cycles reproduce the same scaling (the common cfitsio branch).
    let zeropt = if (est.max - est.min) / delta < INT_MAX - N_RESERVED_VALUES {
        let iqfactor = (est.min / delta + 0.5) as i64;
        iqfactor as f64 * delta
    } else {
        (est.min + est.max) / 2.0
    };

    let mut idata = vec![0i32; n];
    if dither {
        let mut d = Dither::new(irow);
        for (i, &f) in fdata.iter().enumerate().take(n) {
            idata[i] = nint((f - zeropt) / delta + d.next() - 0.5);
        }
    } else {
        for (i, &f) in fdata.iter().enumerate().take(n) {
            idata[i] = nint((f - zeropt) / delta);
        }
    }
    Some(Quantized {
        idata,
        bscale: delta,
        bzero: zeropt,
    })
}
