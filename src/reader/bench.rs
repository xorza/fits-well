//! Criterion groups for the read paths: a seeking (`Read + Seek`) source, which
//! copies each data unit into the reader's scratch before decoding, against an
//! in-memory source (`from_bytes`, and mmap under the `mmap` feature), which
//! decodes straight from the borrowed bytes — one fewer memory pass over the data.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::bench::{TYPES, UNIT_BYTES, sample_data, unit_count};
use crate::reader::FitsReader;
use crate::writer::FitsWriter;

/// A single-HDU file of a [`UNIT_BYTES`] data unit of `bitpix`, as bytes.
fn unit_file(bitpix: Bitpix) -> Vec<u8> {
    let n = unit_count(bitpix);
    let image = Image::new(vec![n], sample_data(bitpix, n)).unwrap();
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_image(&image, None).unwrap();
    w.into_inner().into_inner()
}

/// `read_image` — an owned decode end to end: `seek` stages each data unit into the
/// reader's scratch and decodes from it, `slice` (and `mmap`) decode straight from
/// the borrowed bytes. The reader is opened once, so the header scan is not timed.
pub fn read_image(c: &mut Criterion) {
    let mut g = c.benchmark_group("read_image");
    for (name, bitpix) in TYPES {
        let bytes = unit_file(bitpix);
        g.throughput(Throughput::Bytes(UNIT_BYTES as u64));

        let mut seek = FitsReader::open(Cursor::new(bytes.clone())).unwrap();
        g.bench_function(BenchmarkId::new("seek", name), |b| {
            b.iter(|| black_box(seek.read_image(0).unwrap().decode()));
        });

        let mut slice = FitsReader::from_bytes(&bytes).unwrap();
        g.bench_function(BenchmarkId::new("slice", name), |b| {
            b.iter(|| black_box(slice.read_image(0).unwrap().decode()));
        });

        #[cfg(feature = "mmap")]
        {
            let path = std::env::temp_dir().join(format!(
                "fits-well-read-bench-{}-{name}.fits",
                std::process::id()
            ));
            std::fs::write(&path, &bytes).unwrap();
            let mut mmap = FitsReader::open_mmap(&path).unwrap();
            g.bench_function(BenchmarkId::new("mmap", name), |b| {
                b.iter(|| black_box(mmap.read_image(0).unwrap().decode()));
            });
            drop(mmap);
            std::fs::remove_file(&path).unwrap();
        }
    }
    g.finish();
}

/// `read_image_view` — the borrowed view into a caller-owned reused scratch: no
/// per-call output allocation, the cost the owned decode pays every call. Over the
/// seeking source every type still stages one copy; over the slice source a `u8`
/// view is a zero-copy borrow in constant time, so it reports calls per second, not
/// bytes.
pub fn read_image_view(c: &mut Criterion) {
    let mut g = c.benchmark_group("read_image_view");
    for (name, bitpix) in TYPES {
        let bytes = unit_file(bitpix);

        g.throughput(Throughput::Bytes(UNIT_BYTES as u64));
        let mut seek = FitsReader::open(Cursor::new(bytes.clone())).unwrap();
        let mut seek_scratch: Vec<u64> = Vec::new();
        g.bench_function(BenchmarkId::new("seek", name), |b| {
            // The view borrows reader and scratch, so it cannot leave the closure;
            // `black_box(&view)` keeps the swap from being elided.
            b.iter(|| {
                let view = seek.read_image_view(0, &mut seek_scratch).unwrap();
                black_box(&view);
            });
        });

        if bitpix == Bitpix::U8 {
            g.throughput(Throughput::Elements(1));
        }
        let mut slice = FitsReader::from_bytes(&bytes).unwrap();
        let mut slice_scratch: Vec<u64> = Vec::new();
        g.bench_function(BenchmarkId::new("slice", name), |b| {
            b.iter(|| {
                let view = slice.read_image_view(0, &mut slice_scratch).unwrap();
                black_box(&view);
            });
        });
    }
    g.finish();
}

/// `read_compressed_image_section_view` — a section of a RICE image in 64×64 tiles:
/// one tile, then 64, over both sources.
#[cfg(feature = "compression")]
pub fn read_compressed_image_section(c: &mut Criterion) {
    use crate::compress::{Compression, CompressionOptions};

    const WIDTH: usize = 1024;
    const HEIGHT: usize = 1024;

    let image = Image::new(
        vec![WIDTH, HEIGHT],
        (0..WIDTH * HEIGHT)
            .map(|index| index as i16)
            .collect::<Vec<_>>(),
    )
    .unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(
            &image,
            Compression::Rice,
            &CompressionOptions::tiled([64, 64]),
            None,
        )
        .unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut group = c.benchmark_group("read_compressed_image_section_view");
    for (name, ranges) in [
        ("one_tile", [8..56, 8..56]),
        ("sixty_four_tiles", [8..504, 8..504]),
    ] {
        let section_bytes = ranges.iter().map(ExactSizeIterator::len).product::<usize>() * 2;
        group.throughput(Throughput::Bytes(section_bytes as u64));

        let mut seek = FitsReader::open(Cursor::new(bytes.clone())).unwrap();
        let mut seek_scratch = Vec::new();
        group.bench_function(BenchmarkId::new("seek", name), |bench| {
            bench.iter(|| {
                let view = seek
                    .read_image_section_view(1, &ranges, &mut seek_scratch)
                    .unwrap();
                black_box(&view);
            });
        });

        let mut slice = FitsReader::from_bytes(&bytes).unwrap();
        let mut slice_scratch = Vec::new();
        group.bench_function(BenchmarkId::new("slice", name), |bench| {
            bench.iter(|| {
                let view = slice
                    .read_image_section_view(1, &ranges, &mut slice_scratch)
                    .unwrap();
                black_box(&view);
            });
        });
    }
    group.finish();
}
