//! Floating-point quantization (§10.2) — a port of cfitsio's noise estimator
//! (`FnNoise5_float`), the quantizer (`fits_quantize_float`), and the
//! `SUBTRACTIVE_DITHER_1` random sequence (`fits_init_randoms`).
//!
//! A float tile is mapped to integers `i = NINT((f − zzero) / zscale [+ dither])`
//! and those integers are compressed like an integer image; the decoder inverts
//! `f = (i [− dither]) · zscale + zzero`. Constant tiles (zero noise) can't be
//! quantized and are stored as raw gzip'd floats instead.

use std::mem;
use std::sync::OnceLock;

use crate::compress::plane::FloatSamples;
use crate::compress::quantization::{DitherMethod, QuantizationStep};

const N_RANDOM: usize = 10000;
const N_RESERVED_VALUES: f64 = 10.0;
const INT_MAX: f64 = 2_147_483_647.0;

/// Quantized integer reserved to mark an undefined (null/NaN) pixel.
pub(super) const NULL_VALUE: i32 = -2_147_483_647;
/// Quantized integer reserved by `SUBTRACTIVE_DITHER_2` to mark exact zero.
pub(super) const ZERO_VALUE: i32 = -2_147_483_646;

/// The shared dither sequence (cfitsio `fits_init_randoms`): a Park–Miller
/// minstd generator (`a = 16807`, `m = 2³¹−1`) seeded at 1, scaled to `[0, 1)`.
fn random_values() -> &'static [f32] {
    static VALUES: OnceLock<Vec<f32>> = OnceLock::new();
    VALUES.get_or_init(|| {
        let a = 16807.0f64;
        let m = 2_147_483_647.0_f64;
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
#[derive(Debug)]
struct DitherSequence {
    rand: &'static [f32],
    iseed: usize,
    nextrand: usize,
}

#[expect(
    clippy::cast_sign_loss,
    reason = "the random values lie in (0, 1), so an index scaled by 500 is in 0..500"
)]
impl DitherSequence {
    fn new(irow: i64) -> Self {
        let rand = random_values();
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
    fn next(&mut self) -> f64 {
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

/// Invert a quantized integer plane back to floats (cfitsio `unquantize_i4r4`).
/// `f = (i − dither + 0.5)·scale + zero`, with reserved integers handled first:
/// a value equal to `zblank` becomes `NaN`, and (for `Subtractive2`) [`ZERO_VALUE`]
/// becomes exactly `0.0`. The dither cursor advances per pixel regardless.
pub(super) fn dequantize_into(
    ints: &[i64],
    scale: f64,
    zero: f64,
    method: DitherMethod,
    irow: i64,
    zblank: Option<i64>,
    out: &mut Vec<f64>,
) {
    let dither2 = method == DitherMethod::Subtractive2;
    let mut d = method.dithered().then(|| DitherSequence::new(irow));
    out.clear();
    out.reserve_exact(ints.len());
    out.extend(ints.iter().map(|&v| {
        let r = d.as_mut().map_or(0.0, DitherSequence::next);
        if zblank == Some(v) {
            f64::NAN
        } else if dither2 && v == i64::from(ZERO_VALUE) {
            0.0
        } else if method.dithered() {
            (v as f64 - r + 0.5) * scale + zero
        } else {
            scale * v as f64 + zero
        }
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
    differences2: Vec<f64>,
    differences3: Vec<f64>,
    differences5: Vec<f64>,
    medians2: Vec<f64>,
    medians3: Vec<f64>,
    medians5: Vec<f64>,
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

impl Precision {
    /// `x`, rounded as this arithmetic stores it.
    fn round(self, x: f64) -> f64 {
        match self {
            Self::Single => f64::from(x as f32),
            Self::Double => x,
        }
    }
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
            differences2,
            differences3,
            differences5,
            medians2,
            medians3,
            medians5,
            ..
        } = self;
        medians2.clear();
        medians3.clear();
        medians5.clear();
        let p = |x: f64| precision.round(x);
        for pixels in data[..nx * ny].chunks_exact(nx) {
            row.clear();
            row.extend(pixels.iter().copied().filter(|v| v.is_finite()));
            differences2.clear();
            differences3.clear();
            differences5.clear();
            for w in row.windows(9) {
                let [v1, _, v3, v4, v5, v6, v7, _, v9] = *w else {
                    unreachable!("a window of 9")
                };
                if !(v5 == v6 && v6 == v7) {
                    differences2.push(p(v5 - v7).abs());
                }
                if !(v3 == v4 && v4 == v5 && v5 == v6 && v6 == v7) {
                    differences3.push(p(p(p(2.0 * v5) - v3) - v7).abs());
                    let five = p(p(p(p(p(6.0 * v5) - p(4.0 * v3)) - p(4.0 * v7)) + v1) + v9);
                    differences5.push(five.abs());
                }
            }
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
/// cfitsio's per-row `quick_select_float`. Quickselect, O(n); `total_cmp` orders a NaN
/// that overflowing differences of huge values make, where `partial_cmp` would panic.
fn lower_median(v: &mut [f64]) -> f64 {
    let k = (v.len() - 1) / 2;
    *v.select_nth_unstable_by(k, f64::total_cmp).1
}

/// Proper median (average of the two middle values for an even count), matching
/// cfitsio's final cross-row `qsort` median; 0 for no value.
fn proper_median(v: &mut [f64]) -> f64 {
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    let (below, &mut upper, _) = v.select_nth_unstable_by(n / 2, f64::total_cmp);
    if n % 2 == 1 {
        upper
    } else {
        let lower = below.iter().copied().fold(f64::NEG_INFINITY, f64::max);
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

        let has_null = fdata.iter().take(n).any(|v| !v.is_finite());
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

        scratch.ints.clear();
        scratch.ints.resize(n, 0);
        let mut d = method.dithered().then(|| DitherSequence::new(irow));
        for (i, &f) in fdata.iter().enumerate().take(n) {
            // The dither cursor advances per pixel regardless of branch taken.
            let r = d.as_mut().map_or(0.0, DitherSequence::next);
            scratch.ints[i] = if !f.is_finite() {
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
            bscale: delta,
            bzero: zeropt,
            has_null,
        })
    }
}

#[cfg(test)]
mod tests {

    use crate::compress::quantize::*;

    fn quantizer(step: QuantizationStep, method: DitherMethod, zdither0: i64) -> Quantizer {
        Quantizer {
            precision: Precision::Double,
            step,
            method,
            zdither0,
        }
    }

    #[test]
    fn dither2_quantize_round_trips() {
        // 8×8 field with genuine noise and a scattering of exact zeros.
        let mut data: Vec<f64> = (0u64..64)
            .map(|i| {
                let mut z = i.wrapping_mul(0x9E37_79B9_7F4A_7C15);
                z ^= z >> 31;
                10.0 + (z % 1000) as f64 / 100.0
            })
            .collect();
        for &k in &[0usize, 13, 27, 40, 63] {
            data[k] = 0.0;
        }
        let irow = 7;
        let mut scratch = QuantizeScratch::default();
        let tile = TileValues {
            values: &data,
            width: 8,
            index: 0,
        };
        let q = quantizer(
            QuantizationStep::Relative(4.0),
            DitherMethod::Subtractive2,
            irow,
        )
        .quantize(tile, &mut scratch)
        .unwrap();
        // Exact zeros must encode to the reserved ZERO_VALUE.
        for &k in &[0usize, 13, 27, 40, 63] {
            assert_eq!(scratch.ints[k], ZERO_VALUE, "zero pixel {k}");
        }
        let ints: Vec<i64> = scratch.ints.iter().map(|&v| i64::from(v)).collect();
        let mut back = Vec::new();
        dequantize_into(
            &ints,
            q.bscale,
            q.bzero,
            DitherMethod::Subtractive2,
            irow,
            None,
            &mut back,
        );
        assert_eq!(back.len(), data.len());
        for (i, (&o, &b)) in data.iter().zip(&back).enumerate() {
            if o == 0.0 {
                assert_eq!(b, 0.0, "zero pixel {i} must decode to exactly 0.0");
            } else {
                // Half a quantization step, plus the few roundings of the scale
                // division and the reconstruction, each within an ulp of the value.
                assert!(
                    (o - b).abs() <= 0.5 * q.bscale + 4.0 * f64::EPSILON * o.abs(),
                    "pixel {i}: {o} vs {b}"
                );
            }
        }
    }

    /// One row of the squares 0, 1, 4, …, 121 (12 pixels, so 4 windows of 9) with a NaN
    /// among them, skipped. In window `k` (`v1` = `k²` … `v9` = `(k + 8)²`):
    /// noise2 takes `|v5 − v7| = 4k + 20` → 20, 24, 28, 32, lower median 24;
    /// noise3 takes `|2·v5 − v3 − v7| = 8` each; noise5 takes
    /// `|6·v5 − 4·v3 − 4·v7 + v1 + v9| = 0` each, as a 4th difference kills a quadratic.
    /// cfitsio's noise is noise3: noise2 is larger, and noise5 is zero.
    #[test]
    fn noise5_takes_the_three_median_differences() {
        let mut row: Vec<f64> = (0..12).map(|k| f64::from(k * k)).collect();
        row.insert(5, f64::NAN);
        let mut scratch = QuantizeScratch::default();
        let statistics = scratch.noise5(&row, 13, Precision::Double).unwrap();
        assert_eq!(
            statistics,
            TileStatistics {
                min: 0.0,
                max: 121.0,
                noise2: 1.048_357_9 * 24.0,
                noise3: 0.605_269_7 * 8.0,
                noise5: 0.0,
            }
        );
        assert_eq!(statistics.noise(), 0.605_269_7 * 8.0);

        // A ramp has no third difference: noise3 is 0, and cfitsio keeps it although
        // noise2 is not, so the tile does not quantize.
        let ramp: Vec<f64> = (0..12).map(|k| f64::from(3 * k)).collect();
        let ramp_statistics = scratch.noise5(&ramp, 12, Precision::Double).unwrap();
        assert_eq!(ramp_statistics.noise2, 1.048_357_9 * 6.0);
        assert_eq!(ramp_statistics.noise(), 0.0);
        let tile = TileValues {
            values: &ramp,
            width: 12,
            index: 0,
        };
        assert!(
            quantizer(QuantizationStep::Relative(4.0), DitherMethod::None, 1)
                .quantize(tile, &mut scratch)
                .is_none()
        );

        // Fewer than 9 pixels (0, 1, 4, 9, 16, NaN, 25, 36) have no noise; all nulls
        // have no statistics.
        let short = scratch.noise5(&row[..8], 4, Precision::Double).unwrap();
        assert_eq!((short.min, short.max, short.noise()), (0.0, 36.0, 0.0));
        assert_eq!(scratch.noise5(&[f64::NAN; 16], 16, Precision::Double), None);

        // f32 arithmetic rounds `0 − 2²⁵ − 1` to `−2²⁵`, as cfitsio's `float` does.
        let mut wide = [0.0; 9];
        wide[2] = 33_554_432.0;
        wide[6] = 1.0;
        let single = scratch.noise5(&wide, 9, Precision::Single).unwrap();
        let double = scratch.noise5(&wide, 9, Precision::Double).unwrap();
        assert_eq!(single.noise3, 0.605_269_7 * 33_554_432.0);
        assert_eq!(double.noise3, 0.605_269_7 * 33_554_433.0);

        // Differences of huge values overflow to infinities whose sum is a NaN, which
        // the medians order rather than panic on.
        let huge = [
            f64::MAX,
            -f64::MAX,
            f64::MAX,
            -f64::MAX,
            f64::MAX,
            -f64::MAX,
            f64::MAX,
            -f64::MAX,
            f64::MAX,
            0.0,
        ];
        assert!(scratch.noise5(&huge, 10, Precision::Double).is_some());
    }

    /// The nulls are skipped; an absolute step is the scale, whatever the noise.
    #[test]
    fn an_absolute_step_is_the_scale() {
        let mut values: Vec<f64> = (0..16).map(|k| f64::from((k * 37) % 11)).collect();
        values[3] = f64::NAN;
        values[9] = f64::INFINITY;
        let tile = TileValues {
            values: &values,
            width: 16,
            index: 0,
        };
        let q = quantizer(QuantizationStep::Absolute(0.25), DitherMethod::None, 1)
            .quantize(tile, &mut QuantizeScratch::default())
            .unwrap();
        assert_eq!(q.bscale, 0.25);
        assert!(q.has_null);
    }

    /// 1.0f32 is 0x3F800000 (0x3F + 0x80 = 191) and −2.0f32 is 0xC0000000 (192): 383 + 1.
    /// 1.0f64 is 0x3FF0000000000000 (0x3F + 0xF0 = 303): 304.
    #[test]
    fn the_sample_seed_sums_every_sample_byte() {
        assert_eq!(sample_seed(FloatSamples::F32(&[1.0, -2.0])), 384);
        assert_eq!(sample_seed(FloatSamples::F64(&[1.0])), 304);
        assert_eq!(sample_seed(FloatSamples::F32(&[])), 1);
        // 10000 bytes of 0xFF: 2550000 mod 10000 = 0, so the seed is 1.
        let all_ones = vec![f32::from_bits(u32::MAX); 2500];
        assert_eq!(sample_seed(FloatSamples::F32(&all_ones)), 1);
    }
}
