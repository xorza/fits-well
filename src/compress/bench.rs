//! Criterion groups for the tiled codecs, in uncompressed bytes per second so they
//! compare with the raw `decode` numbers. Codecs are compute-bound and their speed
//! depends on the data, so the fixtures are realistic — a structured ramp plus light
//! noise (a science image), a blocky label field (a mask, for PLIO), and a smooth
//! float field — never random bytes, which would push RICE into its uncompressed
//! block fallback and show GZIP at its worst.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::bintable::BinTable;
use crate::compress::table::internals::mixed_columns;
use crate::compress::{Compression, CompressionOptions, Hcompress};
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::header_model::Header;
use crate::reader::FitsReader;
use crate::writer::FitsWriter;
use crate::writer::table::TableBuilder;

/// Image-compression options pinned to the bench tile shape.
fn opts() -> CompressionOptions {
    CompressionOptions::tiled(TILE)
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
            f(i % NX, i / NX, (s >> 56).cast_signed())
        })
        .collect()
}

fn image(samples: ImageData) -> Image {
    Image::new(vec![NX, NY], samples).unwrap()
}

/// A structured 16-bit science image: a smooth diagonal ramp + small noise,
/// non-negative (so every codec, incl. PLIO, accepts it). Values stay small enough
/// for the 32-bit HCOMPRESS transform.
fn science_i16() -> Image {
    image(ImageData::I16(fill(|x, y, n| {
        (i64::try_from((x + y) % 4096).unwrap() + (n % 17) - 8).max(0) as i16
    })))
}

/// A blocky 16-bit label field — long constant runs, the workload PLIO targets.
fn mask_i16() -> Image {
    image(ImageData::I16(fill(|x, y, _| {
        i16::try_from(((x / 64) + (y / 64)) % 4).unwrap()
    })))
}

/// A smooth 32-bit float field + light noise (quantized on compression).
fn science_f32() -> Image {
    image(ImageData::F32(fill(|x, y, n| {
        (x as f32 * 0.001).sin() + (y as f32 * 0.001).cos() + (n % 17) as f32 * 0.01
    })))
}

fn compressed(img: &Image, compression: Compression) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_image(img, compression, &opts(), None)
        .unwrap();
    w.into_inner().into_inner()
}

const INT_BYTES: u64 = (NX * NY * 2) as u64;
const FLOAT_BYTES: u64 = (NX * NY * 4) as u64;

/// `decompress` — `read_image`, per codec, throughput in uncompressed
/// bytes (the compressed image is HDU 1, after the auto dataless primary).
pub fn decompress(c: &mut Criterion) {
    let int = science_i16();
    let mask = mask_i16();
    let flt = science_f32();
    let mut g = c.benchmark_group("decompress");

    // Open each compressed fixture once and reuse the reader, so we measure
    // decompression per call — not repeated header parsing.
    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
        Compression::Hcompress(Hcompress::default()),
    ] {
        let codec = compression.name();
        let mut r = FitsReader::open(Cursor::new(compressed(&int, compression))).unwrap();
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| {
                let img = r.read_image(1).unwrap();
                black_box(&img);
            });
        });
    }

    let mut rp = FitsReader::open(Cursor::new(compressed(&mask, Compression::Plio))).unwrap();
    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| {
            let img = rp.read_image(1).unwrap();
            black_box(&img);
        });
    });

    for compression in [Compression::Rice, Compression::GZIP] {
        let codec = compression.name();
        let mut r = FitsReader::open(Cursor::new(compressed(&flt, compression))).unwrap();
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| {
                let img = r.read_image(1).unwrap();
                black_box(&img);
            });
        });
    }
    g.finish();
}

/// `compress` — `write_compressed_image`, per codec.
pub fn compress(c: &mut Criterion) {
    let int = science_i16();
    let mask = mask_i16();
    let flt = science_f32();
    let mut g = c.benchmark_group("compress");

    // Reuse the sink `Vec` across iterations so the per-iter output allocation
    // isn't measured — only the codec work (which still allocates per tile, as the
    // implementation inherently does).
    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
        Compression::Hcompress(Hcompress::default()),
    ] {
        let codec = compression.name();
        let mut buf = Vec::new();
        g.throughput(Throughput::Bytes(INT_BYTES));
        g.bench_function(codec, |b| {
            b.iter(|| {
                buf.clear();
                FitsWriter::new(&mut buf)
                    .write_compressed_image(black_box(&int), compression, &opts(), None)
                    .unwrap();
                black_box(buf.len())
            });
        });
    }

    let mut buf = Vec::new();
    g.throughput(Throughput::Bytes(INT_BYTES));
    g.bench_function("PLIO_1", |b| {
        b.iter(|| {
            buf.clear();
            FitsWriter::new(&mut buf)
                .write_compressed_image(black_box(&mask), Compression::Plio, &opts(), None)
                .unwrap();
            black_box(buf.len())
        });
    });

    for compression in [Compression::Rice, Compression::GZIP] {
        let codec = compression.name();
        let mut buf = Vec::new();
        g.throughput(Throughput::Bytes(FLOAT_BYTES));
        g.bench_function(BenchmarkId::new("float", codec), |b| {
            b.iter(|| {
                buf.clear();
                FitsWriter::new(&mut buf)
                    .write_compressed_image(black_box(&flt), compression, &opts(), None)
                    .unwrap();
                black_box(buf.len())
            });
        });
    }
    g.finish();
}

const TABLE_ROWS: usize = 200_000;
/// Rows per §10.3 tile — a chunk, so the table splits into ~49 independent tiles
/// (each column transposed and compressed per tile).
const ROWS_PER_TILE: usize = 4096;

/// The mixed-column table, written then read back — the input to table compression.
#[derive(Debug)]
struct TableFixture {
    header: Header,
    table: BinTable,
}

fn table_fixture() -> TableFixture {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    let table = TableBuilder::explicit(TABLE_ROWS, mixed_columns(TABLE_ROWS)).unwrap();
    w.write_table(&table, None).unwrap();
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();
    TableFixture {
        table: r.read_table(1).unwrap(),
        header: r.hdus()[1].header.clone(),
    }
}

/// Uncompressed data-unit size = `NAXIS1` (row width, from the public header) ×
/// `NAXIS2` rows.
fn table_bytes(header: &Header, table: &BinTable) -> u64 {
    u64::try_from(header.get_integer("NAXIS1").unwrap().unwrap()).unwrap()
        * table.schema().nrows as u64
}

fn compressed_table(header: &Header, table: &BinTable, compression: Compression) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_compressed_table(header, table, ROWS_PER_TILE, compression)
        .unwrap();
    w.into_inner().into_inner()
}

/// `decompress_table` — `read_compressed_table` per column codec (uncompressed
/// bytes/s); the compressed table is HDU 1.
pub fn decompress_table(c: &mut Criterion) {
    let TableFixture { header, table } = table_fixture();
    let bytes = table_bytes(&header, &table);
    let mut g = c.benchmark_group("decompress_table");
    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
    ] {
        let algo = compression.name();
        let mut r =
            FitsReader::open(Cursor::new(compressed_table(&header, &table, compression))).unwrap();
        g.throughput(Throughput::Bytes(bytes));
        g.bench_function(algo, |b| {
            b.iter(|| black_box(r.read_compressed_table(1).unwrap()));
        });
    }
    g.finish();
}

/// `compress_table` — `write_compressed_table` per column codec (reused sink).
pub fn compress_table(c: &mut Criterion) {
    let TableFixture { header, table } = table_fixture();
    let bytes = table_bytes(&header, &table);
    let mut g = c.benchmark_group("compress_table");
    for compression in [
        Compression::GZIP,
        Compression::GZIP_SHUFFLED,
        Compression::Rice,
    ] {
        let algo = compression.name();
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
                        compression,
                    )
                    .unwrap();
                black_box(buf.len())
            });
        });
    }
    g.finish();
}
