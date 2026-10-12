//! The write cases: the standard frame and the source catalog, each written to a file.

use std::fs::File;
use std::hint::black_box;

use criterion::{BenchmarkId, Criterion};

use crate::bench::fixture::{self, Kind, Scale, ScratchDir};
use crate::bench::scene::{self, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, TYPES, WIDTH};
use crate::fits_writer::FitsWriter;

/// `integration/write_image/file/<type>` and `integration/write_table/file/catalog`, from
/// creating the file to the last byte written.
pub(crate) fn integration(c: &mut Criterion) {
    let dir = ScratchDir::new("write");
    write_image(c, &dir);
    write_table(c, &dir);
}

/// `integration/write_image/file/<type>`: the standard frame under the light header with its
/// checksums, as a capture program writes a light and lumos a master. Per sample, in the stored
/// bytes.
fn write_image(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "write_image", Scale::Frame);
    let path = dir.file("frame.fits");
    for bitpix in TYPES {
        let mut frame = None;
        group.throughput(fixture::throughput(WIDTH * HEIGHT, bitpix.elem_size()));
        group.bench_function(BenchmarkId::new("file", fixture::bitpix_id(bitpix)), |b| {
            let frame = frame.get_or_insert_with(|| scene::sky(bitpix, WIDTH, HEIGHT, 1));
            let header = scene::light_header();
            b.iter(|| {
                let mut writer = FitsWriter::new(File::create(&path).unwrap()).with_checksums();
                writer.write_image(black_box(frame), Some(&header)).unwrap();
            });
        });
    }
    group.finish();
}

/// `integration/write_table/file/catalog`: the source catalog after an empty primary. Per row,
/// in the table's bytes.
fn write_table(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "write_table", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    let path = dir.file("catalog.fits");
    let mut catalog = None;
    group.bench_function(BenchmarkId::new("file", "catalog"), |b| {
        let catalog = catalog.get_or_insert_with(scene::catalog);
        b.iter(|| {
            let mut writer = FitsWriter::new(File::create(&path).unwrap());
            writer.write_table(black_box(catalog), None).unwrap();
        });
    });
    group.finish();
}
