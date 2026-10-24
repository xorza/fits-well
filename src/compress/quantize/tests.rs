use crate::compress::decode::float_quantization::Dequant;
use crate::internals::rng::SplitMix;

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
