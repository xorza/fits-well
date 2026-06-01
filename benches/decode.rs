//! Throughput benchmarks for the hot read/write paths.
//!
//! Run with:
//! ```text
//! cargo bench --features internals
//! # for SIMD/AVX2/NEON codegen (non-portable binary), add:
//! RUSTFLAGS="-C target-cpu=native" cargo bench --features internals
//! ```
//!
//! Every bench is tagged with [`Throughput::Bytes`] of the FITS data unit, so
//! Criterion reports GiB/s — directly comparable to memory bandwidth. Inputs and
//! outputs are `black_box`ed so the optimizer can't elide the work. Fixtures are
//! built in memory (no disk), and `read_image` reads from a `Cursor` so the
//! numbers isolate CPU + the staging memcpy, not page-cache/disk variance.
//!
//! Baseline workflow: `cargo bench --features internals -- --save-baseline before`,
//! make a change, then `... -- --baseline before` for deltas with confidence
//! intervals. Pin one machine; results depend on the build profile and target-cpu.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use fits::internals::{decode_image, encode_image};
use fits::{Bitpix, FitsReader, FitsWriter, Image, ImageData, Scaling};

/// Elements per benched image. At this count even the 2-byte type spans ~8 MiB and
/// the 8-byte types ~32 MiB — past L3 on typical machines, so decode is
/// memory-bound rather than cache-resident. Tune for your cache hierarchy.
const N: usize = 4_000_000;

/// All `BITPIX` element types. `u8` has no byte-swap (decode/encode are a plain
/// copy) — it's the memory-bandwidth reference the swapped types are measured
/// against.
const TYPES: &[(&str, Bitpix)] = &[
    ("u8", Bitpix::U8),
    ("i16", Bitpix::I16),
    ("i32", Bitpix::I32),
    ("i64", Bitpix::I64),
    ("f32", Bitpix::F32),
    ("f64", Bitpix::F64),
];

/// Bytes per element from the public `code()` (avoids depending on a crate-internal
/// `elem_size`).
fn elem_bytes(b: Bitpix) -> usize {
    (b.code().unsigned_abs() / 8) as usize
}

/// A big-endian byte buffer of `bytes` bytes. The byte *values* don't affect
/// swap throughput; a non-trivial pattern keeps the optimizer honest.
fn raw_be(bytes: usize) -> Vec<u8> {
    (0..bytes).map(|i| (i as u8).wrapping_mul(31)).collect()
}

/// `N` host-endian samples of the given type.
fn sample_data(bitpix: Bitpix) -> ImageData {
    match bitpix {
        Bitpix::U8 => ImageData::U8((0..N).map(|i| i as u8).collect()),
        Bitpix::I16 => ImageData::I16((0..N).map(|i| i as i16).collect()),
        Bitpix::I32 => ImageData::I32((0..N).map(|i| i as i32).collect()),
        Bitpix::I64 => ImageData::I64((0..N).map(|i| i as i64).collect()),
        Bitpix::F32 => ImageData::F32((0..N).map(|i| i as f32).collect()),
        Bitpix::F64 => ImageData::F64((0..N).map(|i| i as f64).collect()),
    }
}

/// `decode` — big-endian → host byte-swap (`ImageData::decode`).
fn decode(c: &mut Criterion) {
    let mut g = c.benchmark_group("decode");
    for &(name, bitpix) in TYPES {
        let bytes = N * elem_bytes(bitpix);
        let raw = raw_be(bytes);
        g.throughput(Throughput::Bytes(bytes as u64));
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
        let bytes = N * elem_bytes(bitpix);
        let data = sample_data(bitpix);
        g.throughput(Throughput::Bytes(bytes as u64));
        g.bench_function(name, |b| {
            b.iter(|| black_box(encode_image(black_box(&data))))
        });
    }
    g.finish();
}

/// `physical` — the `BZERO + BSCALE·x` scaling plane, with and without a `BLANK`
/// sentinel (the data-dependent branch that inhibits vectorization).
fn physical(c: &mut Criterion) {
    let mut g = c.benchmark_group("physical");
    for &(name, bitpix) in TYPES {
        let bytes = N * elem_bytes(bitpix);
        for (label, blank) in [("plain", None), ("blank", Some(7i64))] {
            let img = Image {
                shape: vec![N],
                samples: sample_data(bitpix),
                scaling: Scaling {
                    bscale: 2.5,
                    bzero: 100.0,
                    blank,
                },
            };
            g.throughput(Throughput::Bytes(bytes as u64));
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
        let bytes = N * elem_bytes(bitpix);
        let img = Image {
            shape: vec![N],
            samples: sample_data(bitpix),
            scaling: Scaling {
                bscale: 1.0,
                bzero: 0.0,
                blank: None,
            },
        };
        let mut w = FitsWriter::new(Cursor::new(Vec::new()));
        w.write_image(&img).unwrap();
        let file = w.into_inner().into_inner();
        g.throughput(Throughput::Bytes(bytes as u64));
        g.bench_function(name, |b| {
            b.iter(|| {
                let mut r = FitsReader::open(Cursor::new(black_box(file.as_slice()))).unwrap();
                black_box(r.read_image(0).unwrap())
            })
        });
    }
    g.finish();
}

criterion_group!(benches, decode, encode, physical, read_image);
criterion_main!(benches);
