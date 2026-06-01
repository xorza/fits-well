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

use fits_well::{
    BinTable, ColumnData, CompressOptions, FitsReader, FitsWriter, Header, Image, ImageData,
    Scaling, WriteColumn,
};

/// Image-compression options pinned to the bench tile shape.
fn opts() -> CompressOptions {
    CompressOptions::tiled(TILE)
}

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
    w.write_compressed_image(img, codec, &opts()).unwrap();
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

    // Open each compressed fixture once and reuse the reader, so we measure
    // decompression per call — not repeated header parsing.
    for &codec in &["GZIP_1", "GZIP_2", "RICE_1", "HCOMPRESS_1"] {
        let mut r = FitsReader::open(Cursor::new(compressed(&int, codec))).unwrap();
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| black_box(r.read_compressed_image(1).unwrap()))
        });
    }

    let mut rp = FitsReader::open(Cursor::new(compressed(&mask, "PLIO_1"))).unwrap();
    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| black_box(rp.read_compressed_image(1).unwrap()))
    });

    for &codec in &["RICE_1", "GZIP_1"] {
        let mut r = FitsReader::open(Cursor::new(compressed(&flt, codec))).unwrap();
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| black_box(r.read_compressed_image(1).unwrap()))
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

    // Reuse the sink `Vec` across iterations so the per-iter output allocation
    // isn't measured — only the codec work (which still allocates per tile, as the
    // implementation inherently does).
    for &codec in &["GZIP_1", "GZIP_2", "RICE_1", "HCOMPRESS_1"] {
        let mut buf = Vec::new();
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| {
                buf.clear();
                FitsWriter::new(&mut buf)
                    .write_compressed_image(black_box(&int), codec, &opts())
                    .unwrap();
                black_box(buf.len())
            })
        });
    }

    let mut buf = Vec::new();
    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| {
            buf.clear();
            FitsWriter::new(&mut buf)
                .write_compressed_image(black_box(&mask), "PLIO_1", &opts())
                .unwrap();
            black_box(buf.len())
        })
    });

    for &codec in &["RICE_1", "GZIP_1"] {
        let mut buf = Vec::new();
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| {
                buf.clear();
                FitsWriter::new(&mut buf)
                    .write_compressed_image(black_box(&flt), codec, &opts())
                    .unwrap();
                black_box(buf.len())
            })
        });
    }
    g.finish();
}

// --- §10.3 tiled table compression (a separate path from image tiles) ---

const TABLE_ROWS: usize = 200_000;
/// Rows per §10.3 tile — a chunk, so the table splits into ~49 independent tiles
/// (each column transposed and compressed per tile).
const ROWS_PER_TILE: usize = 4096;

/// A mixed-column binary table (i16/i32/f32/f64/byte + a repeat-3 vector), written
/// then read back as a `BinTable` + its header — the input to table compression.
fn table_fixture() -> (Header, BinTable) {
    let n = TABLE_ROWS;
    let columns = vec![
        WriteColumn::fixed(
            "SHORT",
            ColumnData::I16((0..n).map(|i| i as i16).collect()),
            1,
        ),
        WriteColumn::fixed(
            "INT",
            ColumnData::I32((0..n).map(|i| i as i32 * 7).collect()),
            1,
        ),
        WriteColumn::fixed(
            "FLT",
            ColumnData::F32((0..n).map(|i| i as f32 * 1.5).collect()),
            1,
        ),
        WriteColumn::fixed(
            "DBL",
            ColumnData::F64((0..n).map(|i| i as f64 * 0.1).collect()),
            1,
        ),
        WriteColumn::fixed(
            "BYTE",
            ColumnData::Bytes((0..n).map(|i| i as u8).collect()),
            1,
        ),
        WriteColumn::fixed(
            "VEC",
            ColumnData::I16((0..n * 3).map(|i| i as i16).collect()),
            3,
        ),
    ];
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_table(n, &columns).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    let table = r.read_table(1).unwrap();
    let header = r.hdu(1).header.clone();
    (header, table)
}

/// Uncompressed data-unit size = `NAXIS1` (row width, from the public header) ×
/// `NAXIS2` rows.
fn table_bytes(header: &Header, table: &BinTable) -> u64 {
    header.get_integer("NAXIS1").unwrap() as u64 * table.nrows as u64
}

fn compressed_table(header: &Header, table: &BinTable, algo: &str) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_table(header, table, ROWS_PER_TILE, algo)
        .unwrap();
    w.into_inner().into_inner()
}

/// `decompress_table` — `read_compressed_table` per column codec (uncompressed
/// bytes/s); the compressed table is HDU 1.
fn decompress_table(c: &mut Criterion) {
    let (header, table) = table_fixture();
    let bytes = table_bytes(&header, &table);
    let mut g = c.benchmark_group("decompress_table");
    for &algo in &["GZIP_1", "GZIP_2", "RICE_1"] {
        let mut r = FitsReader::open(Cursor::new(compressed_table(&header, &table, algo))).unwrap();
        g.throughput(Throughput::Bytes(bytes));
        g.bench_function(algo, |b| {
            b.iter(|| black_box(r.read_compressed_table(1).unwrap()))
        });
    }
    g.finish();
}

/// `compress_table` — `write_compressed_table` per column codec (reused sink).
fn compress_table(c: &mut Criterion) {
    let (header, table) = table_fixture();
    let bytes = table_bytes(&header, &table);
    let mut g = c.benchmark_group("compress_table");
    for &algo in &["GZIP_1", "GZIP_2", "RICE_1"] {
        let mut buf = Vec::new();
        g.throughput(Throughput::Bytes(bytes));
        g.bench_function(algo, |b| {
            b.iter(|| {
                buf.clear();
                FitsWriter::new(&mut buf)
                    .write_compressed_table(
                        black_box(&header),
                        black_box(&table),
                        ROWS_PER_TILE,
                        algo,
                    )
                    .unwrap();
                black_box(buf.len())
            })
        });
    }
    g.finish();
}

criterion_group!(
    benches,
    decompress,
    compress,
    decompress_table,
    compress_table
);
criterion_main!(benches);
