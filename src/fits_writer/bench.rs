//! The write cases: the standard frame whole and in sections, and the source catalog as a
//! binary and an ASCII table, each written to every sink in every storage its input takes.

use std::io::{Seek, Write};

use criterion::{BenchmarkId, Criterion};

use crate::bench::policy::{self, Kind, SECTION_ROWS, Scale, SinkKind, Writing};
use crate::bench::scene_file::{SceneData, SceneFile, SceneFiles, SceneInput};
use crate::fits_writer::FitsWriter;
use crate::header_model::Header;
use crate::image_model::Image;
use crate::internals::scene::{
    self, ASCII_CATALOG_ROW_BYTES, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, WIDTH,
};

/// `integration/write_image/<sink>/<storage>/<input>`,
/// `integration/write_image_stream/<sink>/plain/<input>`,
/// `integration/write_table/<sink>/<storage>/catalog` and
/// `integration/write_ascii_table/<sink>/plain/catalog`, from creating the writer to the last
/// byte written, with the checksums a capture program and lumos write.
pub(crate) fn integration(c: &mut Criterion) {
    let files = SceneFiles::new("write");
    write_image(c, &files);
    write_image_stream(c, &files);
    write_table(c, &files);
    write_ascii_table(c, &files);
}

/// `integration/write_image/<sink>/<storage>/<input>`: the standard frame under the light
/// header, as a capture program writes a light and lumos a master, in each storage a reader
/// meets. Per sample, uncompressed.
fn write_image(c: &mut Criterion, files: &SceneFiles) {
    let mut group = Kind::Integration.group(c, "write_image", Scale::Frame);
    let path = files.scratch("frame.fits");
    for file in SceneFile::images() {
        group.throughput(policy::throughput(
            WIDTH * HEIGHT,
            file.input.bitpix().unwrap().elem_size(),
        ));
        let mut data: Option<SceneData> = None;
        for sink in SinkKind::all() {
            group.bench_function(BenchmarkId::new(sink.id(), file.id()), |b| {
                let data = data.get_or_insert_with(|| file.input.data());
                sink.bench(b, &path, WriteFile { file, data });
            });
        }
    }
    group.finish();
}

/// `integration/write_image_stream/<sink>/plain/<input>`: the standard frame under the light
/// header in sections of [`SECTION_ROWS`] rows, as lumos writes a master, the mirror of
/// `read_section`. Per sample.
fn write_image_stream(c: &mut Criterion, files: &SceneFiles) {
    let mut group = Kind::Integration.group(c, "write_image_stream", Scale::Frame);
    let path = files.scratch("stream.fits");
    for file in SceneFile::plain_frames() {
        group.throughput(policy::throughput(
            WIDTH * HEIGHT,
            file.input.bitpix().unwrap().elem_size(),
        ));
        let mut image: Option<Image> = None;
        for sink in SinkKind::all() {
            group.bench_function(BenchmarkId::new(sink.id(), file.id()), |b| {
                let image = image.get_or_insert_with(|| file.input.image().unwrap());
                let header = scene::light_header();
                sink.bench(
                    b,
                    &path,
                    WriteSections {
                        image,
                        header: &header,
                    },
                );
            });
        }
    }
    group.finish();
}

/// `integration/write_table/<sink>/<storage>/catalog`: the source catalog after an empty
/// primary, compressed in tiles of 4096 rows. Per row, uncompressed.
fn write_table(c: &mut Criterion, files: &SceneFiles) {
    let mut group = Kind::Integration.group(c, "write_table", Scale::Frame);
    group.throughput(policy::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    let path = files.scratch("catalog.fits");
    let mut data: Option<SceneData> = None;
    for file in SceneFile::tables(SceneInput::Catalog) {
        for sink in SinkKind::all() {
            group.bench_function(BenchmarkId::new(sink.id(), file.id()), |b| {
                let data = data.get_or_insert_with(|| file.input.data());
                sink.bench(b, &path, WriteFile { file, data });
            });
        }
    }
    group.finish();
}

/// `integration/write_ascii_table/<sink>/plain/catalog`: the source catalog as an ASCII table
/// after an empty primary, each value formatted to its field. Per row, in the table's bytes.
fn write_ascii_table(c: &mut Criterion, files: &SceneFiles) {
    let mut group = Kind::Integration.group(c, "write_ascii_table", Scale::Frame);
    group.throughput(policy::throughput(CATALOG_ROWS, ASCII_CATALOG_ROW_BYTES));
    let path = files.scratch("ascii_catalog.fits");
    let file = SceneFile::plain(SceneInput::AsciiCatalog);
    let mut data: Option<SceneData> = None;
    for sink in SinkKind::all() {
        group.bench_function(BenchmarkId::new(sink.id(), file.id()), |b| {
            let data = data.get_or_insert_with(|| file.input.data());
            sink.bench(b, &path, WriteFile { file, data });
        });
    }
    group.finish();
}

/// One scene file's input written as the file stores it.
#[derive(Debug)]
struct WriteFile<'a> {
    file: SceneFile,
    data: &'a SceneData,
}

impl Writing for WriteFile<'_> {
    fn write<W: Write + Seek>(&mut self, writer: &mut FitsWriter<W>) {
        self.file.write(writer, self.data);
    }
}

/// An image streamed under `header` a section at a time.
#[derive(Debug)]
struct WriteSections<'a> {
    image: &'a Image,
    header: &'a Header,
}

impl Writing for WriteSections<'_> {
    fn write<W: Write + Seek>(&mut self, writer: &mut FitsWriter<W>) {
        let image = self.image.view();
        let mut stream = writer
            .stream_image(image.metadata(), Some(self.header))
            .unwrap();
        for top in (0..HEIGHT).step_by(SECTION_ROWS) {
            let rows = top..(top + SECTION_ROWS).min(HEIGHT);
            let section = image.samples().slice(rows.start * WIDTH..rows.end * WIDTH);
            stream.write_chunk(section).unwrap();
        }
        stream.finish().unwrap();
    }
}
