//! A seeded generator for tests and benches, so two runs draw the same values.

use std::f64::consts::TAU;

/// Steele, Lea and Flood's SplitMix64: one 64-bit state, a fresh output per call.
#[derive(Debug)]
pub(crate) struct SplitMix(u64);

impl SplitMix {
    pub(crate) const fn new(seed: u64) -> Self {
        Self(seed)
    }

    pub(crate) const fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        z ^ (z >> 31)
    }

    /// A draw on `[0, 1)` from the output's top 53 bits, every one exact in an `f64`.
    pub(crate) fn unit(&mut self) -> f64 {
        (self.next() >> 11) as f64 / (1u64 << 53) as f64
    }

    /// A draw of a unit normal, by Box–Muller.
    pub(crate) fn gaussian(&mut self) -> f64 {
        let (u, v) = (1.0 - self.unit(), self.unit());
        (-2.0 * u.ln()).sqrt() * (TAU * v).cos()
    }
}
