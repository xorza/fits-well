//! The read cases: the data checksum on one row, and a whole image, an image in row sections,
//! a table, and some columns of a table in row chunks, each from a file on every source.

use std::hint::black_box;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion};

use crate::bench::fixture::{self, Kind, Reading, SECTION_ROWS, Scale, ScratchDir, SourceKind};
use crate::bintable::Table;
use crate::bintable::selection::Selection;
use crate::bitpix::Bitpix;
use crate::checksum;
use crate::checksum::checksum_report::ChecksumReport;
use crate::image_model::data::Data;
use crate::image_model::region::Region;
use crate::internals::samples::BITPIXES;
use crate::internals::scene::{self, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, WIDTH};
use crate::reader::FitsReader;
use crate::reader::source::Source;

/// The catalog columns a column read takes, a source's position and flux, and the bytes they
/// take in a row.
const CATALOG_COLUMNS: [&str; 3] = ["RA", "DEC", "FLUX"];
const CATALOG_COLUMN_BYTES: usize = 8 + 8 + 4;

/// The catalog rows a column read takes at a time, so ten chunks.
const CHUNK_ROWS: usize = CATALOG_ROWS / 10;

/// `atomic/checksum/i16`: the ones'-complement sum of one row of the standard frame's stored
/// bytes, the kernel a section read feeds. Per sample.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "checksum", Scale::Sample);
    group.throughput(fixture::throughput(WIDTH, Bitpix::I16.elem_size()));
    group.bench_function(fixture::bitpix_id(Bitpix::I16), |b| {
        let mut row = Vec::new();
        scene::sky(Bitpix::I16, WIDTH, 1, 1)
            .samples
            .view()
            .encode_into(&mut row);
        b.iter(|| checksum::accumulate(black_box(&row), 0));
    });
    group.finish();
}

/// `integration/read_image/<source>/<type>`, `integration/read_section/<source>/<type>`,
/// `integration/read_table/<source>/catalog` and `integration/read_table_columns/<source>/
/// catalog`, from opening the file to the last sample.
pub(crate) fn integration(c: &mut Criterion) {
    let dir = ScratchDir::new("read");
    let mut catalog = None;
    read_image(c, &dir);
    read_section(c, &dir);
    read_table(c, &dir, &mut catalog);
    read_table_columns(c, &dir, &mut catalog);
}

/// `integration/read_image/<source>/<type>`: the standard frame decoded into owned samples. Per
/// sample, in the stored bytes.
fn read_image(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_image", Scale::Frame);
    for bitpix in BITPIXES {
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
    for bitpix in BITPIXES {
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
fn read_table(c: &mut Criterion, dir: &ScratchDir, catalog: &mut Option<PathBuf>) {
    let mut group = Kind::Integration.group(c, "read_table", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    for source in SourceKind::all() {
        group.bench_function(BenchmarkId::new(source.id(), "catalog"), |b| {
            source.bench(b, catalog_path(catalog, dir), ReadTable);
        });
    }
    group.finish();
}

/// `integration/read_table_columns/<source>/catalog`: the position and flux columns of the
/// source catalog, in chunks of [`CHUNK_ROWS`] rows into one reused [`Selection`], as a caller
/// pages through a table larger than it holds. Per row, in the bytes of those columns.
fn read_table_columns(c: &mut Criterion, dir: &ScratchDir, catalog: &mut Option<PathBuf>) {
    let mut group = Kind::Integration.group(c, "read_table_columns", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_COLUMN_BYTES));
    for source in SourceKind::all() {
        group.bench_function(BenchmarkId::new(source.id(), "catalog"), |b| {
            let reading = ReadColumns { selection: None };
            source.bench(b, catalog_path(catalog, dir), reading);
        });
    }
    group.finish();
}

/// The file of the source catalog, written the first time a case needs it.
fn catalog_path<'a>(path: &'a mut Option<PathBuf>, dir: &ScratchDir) -> &'a PathBuf {
    path.get_or_insert_with(|| dir.write("catalog.fits", &scene::catalog_file()))
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
    type Output = Data;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Data {
        reader
            .image(0)
            .unwrap()
            .read(Region::All)
            .unwrap()
            .into_samples()
    }
}

#[derive(Debug)]
struct ReadSections {
    words: Vec<u64>,
}

impl Reading for ReadSections {
    type Output = ChecksumReport;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> ChecksumReport {
        let mut image = reader.image(0).unwrap().with_checksums();
        for top in (0..HEIGHT).step_by(SECTION_ROWS) {
            let ranges = [0..WIDTH, top..(top + SECTION_ROWS).min(HEIGHT)];
            let section = image.view(&ranges, &mut self.words).unwrap();
            black_box(&section);
        }
        image.finish().unwrap()
    }
}

#[derive(Debug)]
struct ReadTable;

impl Reading for ReadTable {
    type Output = Table;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Table {
        reader.read_table(1).unwrap()
    }
}

#[derive(Debug)]
struct ReadColumns {
    /// The selection each chunk refills, kept across iterations as a caller keeps it.
    selection: Option<Selection>,
}

impl Reading for ReadColumns {
    type Output = ();

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) {
        for start in (0..CATALOG_ROWS).step_by(CHUNK_ROWS) {
            let rows = start..(start + CHUNK_ROWS).min(CATALOG_ROWS);
            match &mut self.selection {
                Some(selection) => reader
                    .read_table_columns_into(1, rows, CATALOG_COLUMNS, selection)
                    .unwrap(),
                None => {
                    self.selection =
                        Some(reader.read_table_columns(1, rows, CATALOG_COLUMNS).unwrap());
                }
            }
            black_box(&self.selection);
        }
    }
}
