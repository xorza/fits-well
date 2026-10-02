//! Criterion groups for the in-memory sample paths: the big-endian swap both ways
//! and the physical scaling plane.
//!
//! Every typed bench moves a fixed [`UNIT_BYTES`] of data (the element count is
//! derived per type), so the numbers are comparable *and* the working set clears
//! the last-level cache — the swap is DRAM-bandwidth-bound, not cache-resident.

use std::hint::black_box;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::data::scaling::Scaling;

/// Bytes moved per typed bench. 64 MiB is comfortably past the last-level cache
/// (incl. Apple Silicon's large L2 + system-level cache), so even the 1-byte type
/// is DRAM-bound rather than cache-resident, and every type moves the same bytes.
pub(crate) const UNIT_BYTES: usize = 64 << 20;

/// All `BITPIX` element types. `u8` has no byte-swap (decode/encode are a plain
/// copy) — the memory-bandwidth reference the swapped types are measured against.
pub(crate) const TYPES: [(&str, Bitpix); 6] = [
    ("u8", Bitpix::U8),
    ("i16", Bitpix::I16),
    ("i32", Bitpix::I32),
    ("i64", Bitpix::I64),
    ("f32", Bitpix::F32),
    ("f64", Bitpix::F64),
];

/// Element count giving a [`UNIT_BYTES`] data unit for this type.
pub(crate) const fn unit_count(bitpix: Bitpix) -> usize {
    UNIT_BYTES / bitpix.elem_size()
}

/// `n` host-endian samples of the given type.
pub(crate) fn sample_data(bitpix: Bitpix, n: usize) -> ImageData {
    match bitpix {
        Bitpix::U8 => ImageData::U8((0..n).map(|i| i as u8).collect()),
        Bitpix::I16 => ImageData::I16((0..n).map(|i| i as i16).collect()),
        Bitpix::I32 => ImageData::I32((0..n).map(|i| i as i32).collect()),
        Bitpix::I64 => ImageData::I64((0..n).map(|i| i as i64).collect()),
        Bitpix::F32 => ImageData::F32((0..n).map(|i| i as f32).collect()),
        Bitpix::F64 => ImageData::F64((0..n).map(|i| i as f64).collect()),
    }
}

/// `decode` — big-endian → host byte-swap (`ImageData::decode`).
pub fn decode(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode");
    for (name, bitpix) in TYPES {
        // The byte values don't affect swap throughput; a non-trivial pattern
        // keeps the optimizer honest.
        let raw: Vec<u8> = (0..UNIT_BYTES)
            .map(|i| (i as u8).wrapping_mul(31))
            .collect();
        g.throughput(Throughput::Bytes(raw.len() as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(ImageData::decode(black_box(&raw), bitpix)))
        });
    }
    g.finish();
}

/// `encode` — host → big-endian byte-swap into a reused buffer, as the writer does.
pub fn encode(c: &mut Criterion) {
    let mut g = c.benchmark_group("encode");
    let mut out = Vec::new();
    for (name, bitpix) in TYPES {
        let data = sample_data(bitpix, unit_count(bitpix));
        g.throughput(Throughput::Bytes(UNIT_BYTES as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                out.clear();
                black_box(&data).encode_into(&mut out);
                black_box(out.len())
            })
        });
    }
    g.finish();
}

/// `physical` — the `BZERO + BSCALE·x` scaling plane, with and without a `BLANK`
/// sentinel for integer storage (the data-dependent branch). Output is always
/// `f64`, so size by the f64 output (≈ `UNIT_BYTES`, past cache) and report per
/// element — the work is one scaled value per pixel regardless of the stored width.
pub fn physical(c: &mut Criterion) {
    const FLOAT_CASES: &[(&str, Option<i64>)] = &[("plain", None)];
    const INTEGER_CASES: &[(&str, Option<i64>)] = &[("plain", None), ("blank", Some(7i64))];

    let n = UNIT_BYTES / 8;
    let mut g = c.benchmark_group("physical");
    for (name, bitpix) in TYPES {
        let cases = if bitpix.is_integer() {
            INTEGER_CASES
        } else {
            FLOAT_CASES
        };
        for &(label, blank) in cases {
            let img = Image::new_scaled(
                vec![n],
                sample_data(bitpix, n),
                Scaling {
                    bscale: 2.5,
                    bzero: 100.0,
                    blank,
                },
            )
            .unwrap();
            g.throughput(Throughput::Elements(n as u64));
            g.bench_function(BenchmarkId::new(name, label), |b| {
                b.iter(|| black_box(black_box(&img).physical()))
            });
        }
    }
    g.finish();
}
