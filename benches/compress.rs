//! Compression-codec throughput.
//!
//! Run with:
//! ```text
//! cargo bench --features compression --bench compress
//! ```
//!
//! Throughput is tagged with the **uncompressed** data-unit size, so the numbers
//! read in GiB/s of pixels produced/consumed — directly comparable to the raw
//! `decode`/`encode` benches. Codecs are compute-bound (no memcpy ceiling), and
//! their speed depends on the *data*, not just its size, so the fixtures are
//! deliberately *realistic* — a structured ramp plus light noise (a science image),
//! a blocky label field (a mask, for PLIO), and a smooth float field — never random
//! bytes (which would push RICE into its uncompressed-block fallback and show
//! GZIP at its worst).

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use fits::{FitsReader, FitsWriter, Image, ImageData, Scaling};

const NX: usize = 2048;
const NY: usize = 2048;
/// 2-D tiles (HCOMPRESS requires 2-D) → 8×8 = 64 independent tiles, representative
/// of a real tiled image and what a future parallel decode would fan out over.
const TILE: [usize; 2] = [256, 256];

/// Fill an `NX×NY` buffer from `f(x, y, noise)`, where `noise` is a deterministic
/// xorshift byte (0–255) — no `rand` dependency, reproducible across runs.
fn fill<T>(f: impl Fn(usize, usize, i64) -> T) -> Vec<T> {
    let mut s = 0x2545_F491_4F6C_DD1Du64;
    (0..NX * NY)
        .map(|i| {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            f(i % NX, i / NX, (s >> 56) as i64)
        })
        .collect()
}

fn image(samples: ImageData) -> Image {
    Image {
        shape: vec![NX, NY],
        samples,
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    }
}

/// A structured 16-bit science image: a smooth diagonal ramp + small noise,
/// non-negative (so every codec, incl. PLIO, accepts it). Values stay small enough
/// for the 32-bit HCOMPRESS transform.
fn science_i16() -> Image {
    image(ImageData::I16(fill(|x, y, n| {
        ((((x + y) % 4096) as i64 + (n % 17) - 8).max(0)) as i16
    })))
}

/// A blocky 16-bit label field — long constant runs, the workload PLIO targets.
fn mask_i16() -> Image {
    image(ImageData::I16(fill(|x, y, _| {
        (((x / 64) + (y / 64)) % 4) as i16
    })))
}

/// A smooth 32-bit float field + light noise (quantized on compression).
fn science_f32() -> Image {
    image(ImageData::F32(fill(|x, y, n| {
        (x as f32 * 0.001).sin() + (y as f32 * 0.001).cos() + (n % 17) as f32 * 0.01
    })))
}

fn compressed(img: &Image, codec: &str) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(img, codec, &TILE).unwrap();
    w.into_inner().into_inner()
}

const INT_BYTES: u64 = (NX * NY * 2) as u64;
const FLOAT_BYTES: u64 = (NX * NY * 4) as u64;

/// `decompress` — `read_compressed_image`, per codec, throughput in uncompressed
/// bytes (the compressed image is HDU 1, after the auto dataless primary).
fn decompress(c: &mut Criterion) {
    let int = science_i16();
    let mask = mask_i16();
    let flt = science_f32();
    let mut g = c.benchmark_group("decompress");

    for &codec in &["GZIP_1", "GZIP_2", "RICE_1", "HCOMPRESS_1"] {
        let file = compressed(&int, codec);
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| {
                let mut r = FitsReader::open(Cursor::new(black_box(file.as_slice()))).unwrap();
                black_box(r.read_compressed_image(1).unwrap())
            })
        });
    }

    let plio = compressed(&mask, "PLIO_1");
    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| {
            let mut r = FitsReader::open(Cursor::new(black_box(plio.as_slice()))).unwrap();
            black_box(r.read_compressed_image(1).unwrap())
        })
    });

    for &codec in &["RICE_1", "GZIP_1"] {
        let file = compressed(&flt, codec);
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| {
                let mut r = FitsReader::open(Cursor::new(black_box(file.as_slice()))).unwrap();
                black_box(r.read_compressed_image(1).unwrap())
            })
        });
    }
    g.finish();
}

/// `compress` — `write_compressed_image`, per codec.
fn compress(c: &mut Criterion) {
    let int = science_i16();
    let mask = mask_i16();
    let flt = science_f32();
    let mut g = c.benchmark_group("compress");

    for &codec in &["GZIP_1", "GZIP_2", "RICE_1", "HCOMPRESS_1"] {
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| black_box(compressed(black_box(&int), codec)))
        });
    }

    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| black_box(compressed(black_box(&mask), "PLIO_1")))
    });

    for &codec in &["RICE_1", "GZIP_1"] {
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| black_box(compressed(black_box(&flt), codec)))
        });
    }
    g.finish();
}

criterion_group!(benches, decompress, compress);
criterion_main!(benches);
