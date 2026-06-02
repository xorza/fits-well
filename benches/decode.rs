//! Throughput benchmarks for the hot read/write paths.
//!
//! Run with:
//! ```text
//! cargo bench --features internals --bench decode
//! # for SIMD/AVX2/NEON codegen (non-portable binary), add:
//! RUSTFLAGS="-C target-cpu=native" cargo bench --features internals --bench decode
//! ```
//!
//! Every typed bench moves a fixed [`UNIT_BYTES`] of data (the element count is
//! derived per type), so the numbers are comparable *and* the working set clears
//! the last-level cache — the swap is DRAM-bandwidth-bound, not cache-resident.
//! Inputs/outputs are `black_box`ed; fixtures are in memory (no disk).
//!
//! Baseline workflow: `... --bench decode -- --save-baseline before`, change, then
//! `... -- --baseline before`. Pin one machine; results depend on profile/target-cpu.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use fits_well::internals::{decode_image, encode_image};
use fits_well::{Bitpix, FitsReader, FitsWriter, Image, ImageData, Scaling};

/// Bytes moved per typed bench. 64 MiB is comfortably past the last-level cache
/// (incl. Apple Silicon's large L2 + system-level cache), so even the 1-byte type
/// is DRAM-bound rather than cache-resident, and every type moves the same bytes.
const UNIT_BYTES: usize = 64 << 20;

/// All `BITPIX` element types. `u8` has no byte-swap (decode/encode are a plain
/// copy) — the memory-bandwidth reference the swapped types are measured against.
const TYPES: &[(&str, Bitpix)] = &[
    ("u8", Bitpix::U8),
    ("i16", Bitpix::I16),
    ("i32", Bitpix::I32),
    ("i64", Bitpix::I64),
    ("f32", Bitpix::F32),
    ("f64", Bitpix::F64),
];

/// Bytes per element, from the public `code()` (no dependency on a crate-internal
/// `elem_size`).
fn elem_bytes(b: Bitpix) -> usize {
    (b.code().unsigned_abs() / 8) as usize
}

/// Element count giving a ~[`UNIT_BYTES`] data unit for this type.
fn count(b: Bitpix) -> usize {
    UNIT_BYTES / elem_bytes(b)
}

/// A big-endian byte buffer. The byte *values* don't affect swap throughput; a
/// non-trivial pattern keeps the optimizer honest.
fn raw_be(bytes: usize) -> Vec<u8> {
    (0..bytes).map(|i| (i as u8).wrapping_mul(31)).collect()
}

/// `n` host-endian samples of the given type.
fn sample_data(bitpix: Bitpix, n: usize) -> ImageData {
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
fn decode(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode");
    for &(name, bitpix) in TYPES {
        let raw = raw_be(count(bitpix) * elem_bytes(bitpix));
        g.throughput(Throughput::Bytes(raw.len() as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(decode_image(black_box(&raw), bitpix)))
        });
    }
    g.finish();
}

/// `encode` — host → big-endian byte-swap (`ImageData::encode`).
fn encode(c: &mut Criterion) {
    let mut g = c.benchmark_group("encode");
    for &(name, bitpix) in TYPES {
        let n = count(bitpix);
        let data = sample_data(bitpix, n);
        g.throughput(Throughput::Bytes((n * elem_bytes(bitpix)) as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(encode_image(black_box(&data))))
        });
    }
    g.finish();
}

/// `physical` — the `BZERO + BSCALE·x` scaling plane, with and without a `BLANK`
/// sentinel (the data-dependent branch). Output is always `f64`, so size by the
/// f64 output (≈ [`UNIT_BYTES`], past cache) and report per element — the work is
/// one scaled value per pixel regardless of the stored width.
fn physical(c: &mut Criterion) {
    let n = UNIT_BYTES / 8;
    let mut g = c.benchmark_group("physical");
    for &(name, bitpix) in TYPES {
        for (label, blank) in [("plain", None), ("blank", Some(7i64))] {
            let img = Image {
                shape: vec![n],
                samples: sample_data(bitpix, n),
                scaling: Scaling {
                    bscale: 2.5,
                    bzero: 100.0,
                    blank,
                },
            };
            g.throughput(Throughput::Elements(n as u64));
            g.bench_function(BenchmarkId::new(name, label), |b| {
                b.iter(|| black_box(black_box(&img).physical()))
            });
        }
    }
    g.finish();
}

/// `read_image` — end to end from an in-memory `Cursor`: header scan + staging
/// memcpy + decode. Comparing against `decode` shows the staging/framing overhead.
fn read_image(c: &mut Criterion) {
    let mut g = c.benchmark_group("read_image");
    for &(name, bitpix) in TYPES {
        let n = count(bitpix);
        let img = Image {
            shape: vec![n],
            samples: sample_data(bitpix, n),
            scaling: Scaling {
                bscale: 1.0,
                bzero: 0.0,
                blank: None,
            },
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_image(&img).unwrap();
        // Open once and reuse the reader, whose internal scratch is reused across
        // calls — so we measure the per-call read (seek + staging memcpy + decode),
        // not repeated header parsing or staging allocation. The decoded `Image` is
        // intrinsically fresh per call.
        let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
        g.throughput(Throughput::Bytes((n * elem_bytes(bitpix)) as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(r.read_image(0).unwrap().decode()))
        });
    }
    g.finish();
}

criterion_group!(benches, decode, encode, physical, read_image);
criterion_main!(benches);
