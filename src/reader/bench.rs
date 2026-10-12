//! The read cases: the data checksum on one row, and a whole image, an image in row sections,
//! and a table, each from a file on every source.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion};

use crate::bench::fixture::{self, Kind, Reading, Scale, ScratchDir, SourceKind};
use crate::bench::scene::{self, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, TYPES, WIDTH};
use crate::bintable::BinTable;
use crate::bitpix::Bitpix;
use crate::checksum;
use crate::data::image_data::ImageData;
use crate::reader::source::Source;
use crate::reader::{ChecksumReport, FitsReader};

/// The samples a section read takes at a time, as the lumos light reader does: 64 Ki samples,
/// whole rows of the standard frame, so ten rows a section.
const SECTION_ROWS: usize = (64 * 1024) / WIDTH;

/// `atomic/checksum/i16`: the ones'-complement sum of one row of the standard frame's stored
/// bytes, the kernel a section read feeds. Per sample.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "checksum", Scale::Sample);
    group.throughput(fixture::throughput(WIDTH, Bitpix::I16.elem_size()));
    group.bench_function(fixture::bitpix_id(Bitpix::I16), |b| {
        let mut row = Vec::new();
        scene::sky(Bitpix::I16, WIDTH, 1, 1)
            .samples
            .encode_into(&mut row);
        b.iter(|| checksum::accumulate(black_box(&row), 0));
    });
    group.finish();
}

/// `integration/read_image/<source>/<type>`, `integration/read_section/<source>/<type>` and
/// `integration/read_table/<source>/catalog`, from opening the file to the last sample.
pub(crate) fn integration(c: &mut Criterion) {
    let dir = ScratchDir::new("read");
    read_image(c, &dir);
    read_section(c, &dir);
    read_table(c, &dir);
}

/// `integration/read_image/<source>/<type>`: the standard frame decoded into owned samples. Per
/// sample, in the stored bytes.
fn read_image(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_image", Scale::Frame);
    for bitpix in TYPES {
        let mut path = None;
        group.throughput(fixture::throughput(WIDTH * HEIGHT, bitpix.elem_size()));
        for source in SourceKind::all() {
            let id = BenchmarkId::new(source.id(), fixture::bitpix_id(bitpix));
            group.bench_function(id, |b| {
                let path = frame_path(&mut path, dir, bitpix);
                source.bench(b, path, ReadImage);
            });
        }
    }
    group.finish();
}

/// `integration/read_section/<source>/<type>`: the standard frame as the lumos light reader
/// takes it, ten rows a section into one reused buffer, each section fed to the data checksum,
/// then the checksum judged. Per sample, in the stored bytes.
fn read_section(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_section", Scale::Frame);
    for bitpix in TYPES {
        let mut path = None;
        group.throughput(fixture::throughput(WIDTH * HEIGHT, bitpix.elem_size()));
        for source in SourceKind::all() {
            let id = BenchmarkId::new(source.id(), fixture::bitpix_id(bitpix));
            group.bench_function(id, |b| {
                let path = frame_path(&mut path, dir, bitpix);
                source.bench(b, path, ReadSections { words: Vec::new() });
            });
        }
    }
    group.finish();
}

/// `integration/read_table/<source>/catalog`: the source catalog decoded into a table. Per row,
/// in the table's bytes.
fn read_table(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_table", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    let mut path = None;
    for source in SourceKind::all() {
        group.bench_function(BenchmarkId::new(source.id(), "catalog"), |b| {
            let path =
                path.get_or_insert_with(|| dir.write("catalog.fits", &scene::catalog_file()));
            source.bench(b, path, ReadTable);
        });
    }
    group.finish();
}

/// The file of the standard frame of `bitpix`, written the first time a case needs it.
fn frame_path<'a>(path: &'a mut Option<PathBuf>, dir: &ScratchDir, bitpix: Bitpix) -> &'a PathBuf {
    path.get_or_insert_with(|| {
        let name = format!("frame_{}.fits", fixture::bitpix_id(bitpix));
        dir.write(&name, &scene::frame_file(bitpix))
    })
}

#[derive(Debug)]
struct ReadImage;

impl Reading for ReadImage {
    type Output = ImageData;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> ImageData {
        reader.read_image(0).unwrap().decode()
    }
}

#[derive(Debug)]
struct ReadSections {
    words: Vec<u64>,
}

impl Reading for ReadSections {
    type Output = ChecksumReport;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> ChecksumReport {
        let mut sum = reader.begin_data_checksum(0).unwrap();
        for top in (0..HEIGHT).step_by(SECTION_ROWS) {
            let ranges = [0..WIDTH, top..(top + SECTION_ROWS).min(HEIGHT)];
            let section = reader
                .read_image_section_view_summed(0, &ranges, &mut self.words, &mut sum)
                .unwrap();
            black_box(&section);
        }
        reader.finish_data_checksum(sum).unwrap()
    }
}

#[derive(Debug)]
struct ReadTable;

impl Reading for ReadTable {
    type Output = BinTable;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> BinTable {
        reader.read_table(1).unwrap()
    }
}
