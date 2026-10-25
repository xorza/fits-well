use crate::compression::decode::float_quantization::Dequant;
use crate::compression::half_step::assert_within_half_step;
use crate::image_model::bitpix::Bitpix;
use crate::internals::rng::SplitMix;

use crate::compression::quantize::*;

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
        .map(|i| 10.0 + (SplitMix::new(i).next() % 1000) as f64 / 100.0)
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
    let mut back: Vec<f64> = Vec::new();
    Dequant {
        scale: q.bscale,
        zero: q.bzero,
        method: DitherMethod::Subtractive2,
        irow,
        zblank: None,
    }
    .apply(&ints, &mut back);
    assert_eq!(back.len(), data.len());
    for (i, (&o, &b)) in data.iter().zip(&back).enumerate() {
        if o == 0.0 {
            assert_eq!(b, 0.0, "zero pixel {i} must decode to exactly 0.0");
        } else {
            assert_within_half_step(o, b, q.bscale, Bitpix::F64, &format!("pixel {i}"));
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
    // The range is the finite extremes; the second-order differences cancel to 0, the
    // third-order ones overflow to an infinity, and the fifth-order ones to a NaN.
    let stats = scratch.noise5(&huge, 10, Precision::Double).unwrap();
    assert_eq!([stats.min, stats.max], [-f64::MAX, f64::MAX]);
    assert_eq!([stats.noise2, stats.noise3], [0.0, f64::INFINITY]);
    assert!(stats.noise5.is_nan());
}

/// The nulls are skipped; an absolute step is the scale, whatever the noise.
#[test]
fn absolute_step_is_the_scale() {
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
fn sample_seed_sums_every_sample_byte() {
    assert_eq!(sample_seed(FloatSamples::F32(&[1.0, -2.0])), 384);
    assert_eq!(sample_seed(FloatSamples::F64(&[1.0])), 304);
    assert_eq!(sample_seed(FloatSamples::F32(&[])), 1);
    // 10000 bytes of 0xFF: 2550000 mod 10000 = 0, so the seed is 1.
    let all_ones = vec![f32::from_bits(u32::MAX); 2500];
    assert_eq!(sample_seed(FloatSamples::F32(&all_ones)), 1);
}

/// The estimator before it took an f32 image's differences in `f32` and selected medians
/// on bit patterns, kept to cross-check it.
fn reference_noise5(data: &[f64], width: usize, precision: Precision) -> Option<TileStatistics> {
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
    let (mut row, mut differences2, mut differences3, mut differences5) =
        (Vec::new(), Vec::new(), Vec::new(), Vec::new());
    let (mut medians2, mut medians3, mut medians5) = (Vec::new(), Vec::new(), Vec::new());
    let p = |x: f64| match precision {
        Precision::Single => f64::from(x as f32),
        Precision::Double => x,
    };
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
            (&mut differences2, &mut medians2),
            (&mut differences3, &mut medians3),
            (&mut differences5, &mut medians5),
        ] {
            if !differences.is_empty() {
                medians.push(reference_lower_median(differences));
            }
        }
    }
    Some(TileStatistics {
        min,
        max,
        noise2: 1.048_357_9 * reference_proper_median(&mut medians2),
        noise3: 0.605_269_7 * reference_proper_median(&mut medians3),
        noise5: 0.177_204_8 * reference_proper_median(&mut medians5),
    })
}

fn reference_lower_median(v: &mut [f64]) -> f64 {
    let k = (v.len() - 1) / 2;
    *v.select_nth_unstable_by(k, f64::total_cmp).1
}

/// Proper median (average of the two middle values for an even count), matching
/// cfitsio's final cross-row `qsort` median; 0 for no value.
fn reference_proper_median(v: &mut [f64]) -> f64 {
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

/// The estimate gives the reference estimate's values, for random rows of f32 values
/// in single precision and of f64 values in double: equal runs, nulls, huge values that
/// overflow, and widths under 9 among them.
#[test]
fn noise5_matches_the_reference_estimate() {
    let mut rng = SplitMix::new(0x0015_e5ed);
    let mut scratch = QuantizeScratch::default();
    for round in 0..300 {
        let width = 1 + (rng.next() % 40) as usize;
        let len = width * (1 + (rng.next() % 8) as usize);
        let scale = [1.0, 1e-3, 3e38, 1e300][round % 4];
        let data: Vec<f64> = (0..len)
            .map(|i| match rng.next() % 16 {
                0 => f64::NAN,
                1 => 7.0,
                _ => (rng.unit() - 0.5) * scale + i as f64,
            })
            .collect();
        let single: Vec<f64> = data.iter().map(|&v| f64::from(v as f32)).collect();
        for (precision, data) in [(Precision::Single, &single), (Precision::Double, &data)] {
            let got = scratch.noise5(data, width, precision);
            let want = reference_noise5(data, width, precision);
            assert_eq!(
                format!("{got:?}"),
                format!("{want:?}"),
                "round {round}, {precision:?}"
            );
        }
    }
}
