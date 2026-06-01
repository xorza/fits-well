//! Read-path throughput: a seeking (`Read + Seek`) source, which copies each data
//! unit into the reader's scratch before decoding, versus an in-memory source
//! (`from_bytes`, and mmap under the `mmap` feature), which decodes straight from
//! the borrowed bytes — one fewer memory pass over the data.
//!
//! Run with:
//! ```text
//! cargo bench --bench read
//! cargo bench --features mmap --bench read   # adds the mmap arm
//! ```
//!
//! Each type moves a fixed [`UNIT_BYTES`] data unit (well past the last-level
//! cache), so `seek` vs `slice` isolates the cost of the staging copy. `u8` is the
//! zero-swap case where a borrow source can hand back bytes untouched.

use std::hint::black_box;
use std::io::Cursor;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};

use fits_well::{Bitpix, FitsReader, FitsWriter, Image, ImageData, Scaling};

/// Data-unit size per type — 64 MiB clears the last-level cache, so the staging
/// copy is real DRAM traffic rather than a cache hit.
const UNIT_BYTES: usize = 64 << 20;

const TYPES: &[(&str, Bitpix)] = &[
    ("u8", Bitpix::U8),
    ("i16", Bitpix::I16),
    ("f32", Bitpix::F32),
];

fn elem_bytes(b: Bitpix) -> usize {
    (b.code().unsigned_abs() / 8) as usize
}

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

/// A written single-HDU FITS file (`n` samples of `bitpix`) as bytes.
fn fits_bytes(bitpix: Bitpix, n: usize) -> Vec<u8> {
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
    w.into_inner().into_inner()
}

fn read_image(c: &mut Criterion) {
    let mut g = c.benchmark_group("read_image");
    for &(name, bitpix) in TYPES {
        let n = UNIT_BYTES / elem_bytes(bitpix);
        let bytes = fits_bytes(bitpix, n);
        g.throughput(Throughput::Bytes((n * elem_bytes(bitpix)) as u64));

        // Seeking source: each read copies the data unit into the reader's scratch,
        // then decodes out of it (two passes over the data).
        let mut seek = FitsReader::open(Cursor::new(bytes.clone())).unwrap();
        g.bench_function(BenchmarkId::new("seek", name), |b| {
            b.iter(|| black_box(seek.read_image(0).unwrap()))
        });

        // In-memory borrow source: decode straight from the bytes — no staging copy.
        let mut slice = FitsReader::from_bytes(&bytes).unwrap();
        g.bench_function(BenchmarkId::new("slice", name), |b| {
            b.iter(|| black_box(slice.read_image(0).unwrap()))
        });

        // mmap source: same zero-copy decode, but over a real mapped file.
        #[cfg(feature = "mmap")]
        {
            use std::io::Write;
            std::fs::create_dir_all(".tmp").unwrap();
            let path = format!(".tmp/read_bench_{name}.fits");
            std::fs::File::create(&path)
                .unwrap()
                .write_all(&bytes)
                .unwrap();
            let mut mmap = FitsReader::open_mmap(&path).unwrap();
            g.bench_function(BenchmarkId::new("mmap", name), |b| {
                b.iter(|| black_box(mmap.read_image(0).unwrap()))
            });
        }
    }
    g.finish();
}

criterion_group!(benches, read_image);
criterion_main!(benches);
