//! Floating-point quantization (§10.2) — a port of cfitsio's noise estimator
//! (`FnNoise5_float`), the quantizer (`fits_quantize_float`), and the
//! `SUBTRACTIVE_DITHER_1` random sequence (`fits_init_randoms`).
//!
//! A float tile is mapped to integers `i = NINT((f − zzero) / zscale [+ dither])`
//! and those integers are compressed like an integer image; the decoder inverts
//! `f = (i [− dither]) · zscale + zzero`. Constant tiles (zero noise) can't be
//! quantized and are stored as raw gzip'd floats instead.

use std::mem;

use crate::compression::plane::FloatSamples;
use crate::compression::quantization::{DitherMethod, QuantizationStep};

const N_RANDOM: usize = 10000;
const N_RESERVED_VALUES: f64 = 10.0;
const INT_MAX: f64 = 2_147_483_647.0;

/// Quantized integer reserved to mark an undefined (null/NaN) pixel.
pub(super) const NULL_VALUE: i32 = -2_147_483_647;
/// Quantized integer reserved by `SUBTRACTIVE_DITHER_2` to mark exact zero.
pub(super) const ZERO_VALUE: i32 = -2_147_483_646;

/// The shared dither sequence (cfitsio `fits_init_randoms`): a Park–Miller minstd
/// generator (`a = 16807`, `m = 2³¹−1`) seeded at 1, scaled to `[0, 1)`, built at compile
/// time. A static, as the 40 KB table is read by address, not copied to each use.
static RANDOM: [f32; N_RANDOM] = random_values();

#[expect(
    clippy::large_stack_arrays,
    reason = "evaluated at compile time into the static, never on a stack"
)]
const fn random_values() -> [f32; N_RANDOM] {
    let a = 16807.0f64;
    let m = 2_147_483_647.0_f64;
    let mut seed = 1.0f64;
    let mut values = [0.0f32; N_RANDOM];
    let mut i = 0;
    while i < N_RANDOM {
        let temp = a * seed;
        seed = temp - m * ((temp / m) as i64 as f64);
        values[i] = (seed / m) as f32;
        i += 1;
    }
    assert!(seed as i64 == 1_043_618_065, "cfitsio's final seed");
    values
}

/// `(row − 1) mod N_RANDOM` → the starting index into [`RANDOM`] for a
/// tile, and the first `nextrand` cursor (cfitsio's `iseed`/`nextrand` setup).
#[derive(Debug)]
pub(super) struct DitherSequence {
    rand: &'static [f32],
    iseed: usize,
    nextrand: usize,
}

#[expect(
    clippy::cast_sign_loss,
    reason = "the random values lie in (0, 1), so an index scaled by 500 is in 0..500"
)]
impl DitherSequence {
    pub(super) fn new(irow: i64) -> Self {
        let rand = &RANDOM;
        let period = i64::try_from(N_RANDOM).unwrap();
        let iseed = usize::try_from((irow - 1).rem_euclid(period)).unwrap();
        let nextrand = (rand[iseed] * 500.0) as usize;
        Self {
            rand,
            iseed,
            nextrand,
        }
    }

    /// The current dither value, then advance the cursor (cfitsio's wrap logic).
    pub(super) fn next(&mut self) -> f64 {
        let r = f64::from(self.rand[self.nextrand]);
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

/// `ints`, cleared, filled with `quantize` of each of `values`, [`NULL_VALUE`] for a value
/// that is not finite. `quantize` sees every value, so a dither cursor advances on each.
fn fill(ints: &mut Vec<i32>, values: &[f64], mut quantize: impl FnMut(f64) -> i32) {
    ints.clear();
    ints.reserve_exact(values.len());
    ints.extend(values.iter().map(|&f| {
        let quantized = quantize(f);
        if f.is_finite() { quantized } else { NULL_VALUE }
    }));
}

/// Round-to-nearest, ties away from zero (cfitsio `NINT`).
fn nint(x: f64) -> i32 {
    if x >= 0.0 {
        (x + 0.5) as i32
    } else {
        (x - 0.5) as i32
    }
}

/// The range and the 2nd, 3rd and 5th order noise estimates of a tile's finite
/// (non-null) pixels.
#[derive(Debug, PartialEq)]
struct TileStatistics {
    min: f64,
    max: f64,
    noise2: f64,
    noise3: f64,
    noise5: f64,
}

impl TileStatistics {
    /// cfitsio's background noise: noise3, or noise2 or noise5 where either is nonzero
    /// and smaller (`fits_quantize_float`, `imcomp_compress_tile`).
    fn noise(&self) -> f64 {
        let mut noise = self.noise3;
        if self.noise2 != 0.0 && self.noise2 < noise {
            noise = self.noise2;
        }
        if self.noise5 != 0.0 && self.noise5 < noise {
            noise = self.noise5;
        }
        noise
    }
}

#[derive(Debug, Default)]
pub(super) struct QuantizeScratch {
    pub(super) ints: Vec<i32>,
    /// An integer tile widened for its noise estimate.
    floats: Vec<f64>,
    row: Vec<f64>,
    /// A row of an f32 image, its differences taken in `f32`.
    row_single: Vec<f32>,
    /// The differences and the per-row medians, each a non-negative float as its bit
    /// pattern, whose integer order is the floats' `total_cmp` order.
    differences: Differences,
    medians2: Vec<u64>,
    medians3: Vec<u64>,
    medians5: Vec<u64>,
}

/// The absolute 2nd, 3rd and 5th order differences of a row, as bit patterns.
#[derive(Debug, Default)]
struct Differences {
    second: Vec<u64>,
    third: Vec<u64>,
    fifth: Vec<u64>,
}

impl Differences {
    /// The differences of the windows of 9 of `row` (cfitsio `FnNoise5_float`), each step
    /// rounded in `F`, a window of equal values skipped as cfitsio skips it.
    fn take<F: NoiseFloat>(&mut self, row: &[F]) {
        self.second.clear();
        self.third.clear();
        self.fifth.clear();
        let [two, four, six] = [2.0, 4.0, 6.0].map(F::of);
        for w in row.windows(9) {
            let [v1, _, v3, v4, v5, v6, v7, _, v9] = *w else {
                unreachable!("a window of 9")
            };
            if !(v5 == v6 && v6 == v7) {
                self.second.push((v5 - v7).abs_bits());
            }
            if !(v3 == v4 && v4 == v5 && v5 == v6 && v6 == v7) {
                self.third.push((two * v5 - v3 - v7).abs_bits());
                self.fifth
                    .push((six * v5 - four * v3 - four * v7 + v1 + v9).abs_bits());
            }
        }
    }
}

/// The float a tile's differences are taken in. Each step rounds once in it, as cfitsio's
/// `float` and `double` do; for an f32 image that is the `f64` step rounded to `f32`, as
/// 53 ≥ 2·24 + 2 bits make the double rounding innocuous for `+`, `−` and `×` (Figueroa,
/// "When is double rounding innocuous?", 1995), without two conversions a step.
trait NoiseFloat:
    Copy
    + PartialEq
    + std::ops::Add<Output = Self>
    + std::ops::Sub<Output = Self>
    + std::ops::Mul<Output = Self>
{
    fn of(value: f64) -> Self;
    /// `|self|` widened to `f64`, as its bit pattern.
    fn abs_bits(self) -> u64;
}

impl NoiseFloat for f32 {
    fn of(value: f64) -> Self {
        value as Self
    }

    fn abs_bits(self) -> u64 {
        f64::from(self.abs()).to_bits()
    }
}

impl NoiseFloat for f64 {
    fn of(value: f64) -> Self {
        value
    }

    fn abs_bits(self) -> u64 {
        self.abs().to_bits()
    }
}

/// The arithmetic a tile's differences are taken in, as cfitsio's estimator for its
/// sample type: `float` for an f32 image (`FnNoise5_float`), and `double` for an f64
/// image or an integer one, whose differences cfitsio takes exactly (`FnNoise5_double`,
/// `FnNoise5_int`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Precision {
    Single,
    Double,
}

impl QuantizeScratch {
    /// The 2nd, 3rd and 5th order median absolute differences of a tile (cfitsio
    /// `FnNoise5_float`, after Stoehr et al., ST-ECF newsletter 42), each the median of the
    /// per-row medians, and the range of its finite pixels; `None` for an all-null tile.
    /// Non-finite pixels are nulls and are skipped, as cfitsio's `nullcheck` does. A row
    /// takes differences only past its first 8 valid pixels, a tile narrower than 9 pixels
    /// is one row, and one of fewer than 9 pixels has no noise. `data` holds rows of
    /// `width` pixels.
    fn noise5(
        &mut self,
        data: &[f64],
        width: usize,
        precision: Precision,
    ) -> Option<TileStatistics> {
        let mut nx = width.max(1);
        let mut ny = data.len() / nx;
        if nx < 9 {
            nx *= ny;
            ny = 1;
        }
        let finite = || data[..nx * ny].iter().copied().filter(|v| v.is_finite());
        let min = finite().fold(f64::INFINITY, f64::min);
        let max = finite().fold(f64::NEG_INFINITY, f64::max);
        if min > max {
            return None;
        }
        if nx < 9 {
            return Some(TileStatistics {
                min,
                max,
                noise2: 0.0,
                noise3: 0.0,
                noise5: 0.0,
            });
        }
        let Self {
            row,
            row_single,
            differences,
            medians2,
            medians3,
            medians5,
            ..
        } = self;
        medians2.clear();
        medians3.clear();
        medians5.clear();
        for pixels in data[..nx * ny].chunks_exact(nx) {
            let finite = pixels.iter().copied().filter(|v| v.is_finite());
            match precision {
                Precision::Single => {
                    row_single.clear();
                    row_single.extend(finite.map(|value| {
                        debug_assert_eq!(f64::from(value as f32), value, "an f32 image's value");
                        value as f32
                    }));
                    differences.take(row_single);
                }
                Precision::Double => {
                    row.clear();
                    row.extend(finite);
                    differences.take(row);
                }
            }
            let Differences {
                second: differences2,
                third: differences3,
                fifth: differences5,
            } = &mut *differences;
            // Each median runs over its own differences. cfitsio takes noise2's over the
            // noise3 count (`quantize.c:1828`), which reads entries left from earlier rows
            // when a run of three equal values drops a noise2 difference that noise3 keeps;
            // on such rows a step can differ from fpack's.
            for (differences, medians) in [
                (&mut *differences2, &mut *medians2),
                (&mut *differences3, &mut *medians3),
                (&mut *differences5, &mut *medians5),
            ] {
                if !differences.is_empty() {
                    medians.push(lower_median(differences));
                }
            }
        }
        Some(TileStatistics {
            min,
            max,
            noise2: 1.048_357_9 * proper_median(medians2),
            noise3: 0.605_269_7 * proper_median(medians3),
            noise5: 0.177_204_8 * proper_median(medians5),
        })
    }

    /// The background-noise estimate of an integer tile of rows of `width` values, for the
    /// HCOMPRESS scale (cfitsio `fits_img_stats_int`).
    pub(super) fn integer_noise(&mut self, values: &[i64], width: usize) -> f64 {
        let mut floats = mem::take(&mut self.floats);
        floats.clear();
        floats.extend(values.iter().map(|&value| value as f64));
        let noise = self
            .noise5(&floats, width, Precision::Double)
            .map_or(0.0, |statistics| statistics.noise());
        self.floats = floats;
        noise
    }
}

/// Lower median (element at index `(n−1)/2` of the sorted values), matching
/// cfitsio's per-row `quick_select_float`. Quickselect, O(n), on the bit patterns of
/// non-negative floats, whose integer order is `total_cmp`'s: a NaN that overflowing
/// differences of huge values make sorts above every number.
fn lower_median(v: &mut [u64]) -> u64 {
    let k = (v.len() - 1) / 2;
    *v.select_nth_unstable(k).1
}

/// Proper median (average of the two middle values for an even count), matching
/// cfitsio's final cross-row `qsort` median; 0 for no value.
fn proper_median(v: &mut [u64]) -> f64 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    let (below, &mut upper, _) = v.select_nth_unstable(n / 2);
    let upper = f64::from_bits(upper);
    if n % 2 == 1 {
        upper
    } else {
        let lower = below
            .iter()
            .copied()
            .max()
            .map_or(f64::NEG_INFINITY, f64::from_bits);
        f64::midpoint(lower, upper)
    }
}

/// Metadata for the integer plane written into [`QuantizeScratch`]: the
/// `BSCALE`/`BZERO` (`ZSCALE`/`ZZERO`) that invert it and whether any pixel was null.
#[derive(Debug)]
pub(super) struct Quantized {
    pub(super) bscale: f64,
    pub(super) bzero: f64,
    pub(super) has_null: bool,
}

/// The default `ZDITHER0` of an image: `(Σ bytes of every sample mod 10000) + 1`. A
/// byte sum does not depend on the byte order.
pub(super) fn sample_seed(samples: FloatSamples<'_>) -> i64 {
    fn byte_sum(bits: u64) -> u64 {
        bits.to_ne_bytes().iter().map(|&byte| u64::from(byte)).sum()
    }
    let sum: u64 = match samples {
        FloatSamples::F32(values) => values
            .iter()
            .map(|v| byte_sum(u64::from(v.to_bits())))
            .sum(),
        FloatSamples::F64(values) => values.iter().map(|v| byte_sum(v.to_bits())).sum(),
    };
    i64::try_from(sum % 10_000).expect("below 10000") + 1
}

/// How every tile of one float image quantizes (cfitsio `fits_quantize_float`).
#[derive(Debug, Clone, Copy)]
pub(super) struct Quantizer {
    pub(super) precision: Precision,
    pub(super) step: QuantizationStep,
    pub(super) method: DitherMethod,
    /// `ZDITHER0`, which a tile's 0-based index offsets into the dither sequence; no
    /// sequence runs without a dither.
    pub(super) zdither0: i64,
}

/// One tile's values, row-major with `width` values to a row, and its 0-based index
/// in the image.
#[derive(Debug, Clone, Copy)]
pub(super) struct TileValues<'a> {
    pub(super) values: &'a [f64],
    pub(super) width: usize,
    pub(super) index: usize,
}

impl Quantizer {
    /// Quantize a float tile into `scratch.ints`. Non-finite pixels become
    /// [`NULL_VALUE`] and, for `Subtractive2`, exact zeros become [`ZERO_VALUE`]. Returns
    /// `None` when the tile can't be quantized (no finite data, constant data, or a range
    /// wider than the int domain) — the caller then stores the raw floats losslessly.
    pub(super) fn quantize(
        &self,
        tile: TileValues<'_>,
        scratch: &mut QuantizeScratch,
    ) -> Option<Quantized> {
        let fdata = tile.values;
        let n = fdata.len();
        if n <= 1 {
            return None;
        }
        let method = self.method;
        // (1-based tile row) + ZDITHER0 − 1.
        let irow = i64::try_from(tile.index).expect("a tile index fits i64") + self.zdither0;
        // An all-null tile is stored raw, which keeps its NaNs exactly.
        let est = scratch.noise5(fdata, tile.width, self.precision)?;
        let delta = match self.step {
            QuantizationStep::Relative(level) => est.noise() / level,
            QuantizationStep::Absolute(step) => step,
        };
        if delta == 0.0 {
            return None;
        }
        if (est.max - est.min) / delta > 2.0 * INT_MAX - N_RESERVED_VALUES {
            return None;
        }

        let has_null = fdata.iter().any(|v| !v.is_finite());
        let dither2 = method == DitherMethod::Subtractive2;
        // When nulls are present (or for DITHER_2), shift the range above the reserved
        // values so NULL_VALUE/ZERO_VALUE never collide with real data; otherwise fudge
        // the zero point to an integer multiple of delta (stable across re-compression).
        let zeropt = if has_null || dither2 {
            est.min - delta * (f64::from(NULL_VALUE) + N_RESERVED_VALUES)
        } else if (est.max - est.min) / delta < INT_MAX - N_RESERVED_VALUES {
            let iqfactor = (est.min / delta + 0.5) as i64;
            iqfactor as f64 * delta
        } else {
            f64::midpoint(est.min, est.max)
        };

        let ints = &mut scratch.ints;
        match method {
            DitherMethod::None => fill(ints, fdata, |f| nint((f - zeropt) / delta)),
            DitherMethod::Subtractive1 => {
                let mut dither = DitherSequence::new(irow);
                fill(ints, fdata, |f| {
                    nint((f - zeropt) / delta + dither.next() - 0.5)
                });
            }
            DitherMethod::Subtractive2 => {
                let mut dither = DitherSequence::new(irow);
                fill(ints, fdata, |f| {
                    let r = dither.next();
                    if f == 0.0 {
                        ZERO_VALUE
                    } else {
                        nint((f - zeropt) / delta + r - 0.5)
                    }
                });
            }
        }
        Some(Quantized {
            bscale: delta,
            bzero: zeropt,
            has_null,
        })
    }
}

#[cfg(test)]
mod tests;
