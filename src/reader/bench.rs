//! The read cases: the data checksum on one row; and the header scan, an image whole, in row
//! sections and as physical values, its checksum, a table whole, in row chunks, by columns and
//! by cells, and an ASCII table, each from a file on every source and in every storage its
//! input takes.

use std::hint::black_box;
use std::path::Path;

use criterion::{BenchmarkId, Criterion, Throughput};

use crate::ascii::Ascii;
use crate::bench::policy::{self, Kind, Reading, SECTION_ROWS, Scale, SourceKind};
use crate::bench::scene_file::{SceneFile, SceneFiles, SceneInput};
use crate::bintable::Table;
use crate::bintable::selection::Selection;
use crate::checksum;
use crate::checksum::checksum_report::ChecksumReport;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::region::Region;
use crate::internals::rng::SplitMix;
use crate::internals::scene;
use crate::internals::scene::{
    ASCII_CATALOG_ROW_BYTES, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, MOSAIC_CCDS, SPECTRA_ROWS,
    WIDTH,
};
use crate::reader::FitsReader;
use crate::reader::source::Source;

/// The cells a cell read takes, at rows a fixed seed draws: as many as keep the compressed
/// spectra under 300 ms an iteration on a stream, which decodes a tile for each cell.
const CELLS: usize = 100;

/// `atomic/checksum/i16`: the ones'-complement sum of one row of the standard frame's stored
/// bytes, the kernel a section read feeds. Per sample.
pub(crate) fn atomic(c: &mut Criterion) {
    let mut group = Kind::Atomic.group(c, "checksum", Scale::Row);
    group.throughput(policy::throughput(WIDTH, Bitpix::I16.elem_size()));
    group.bench_function(policy::bitpix_id(Bitpix::I16), |b| {
        let mut row = Vec::new();
        scene::sky(Bitpix::I16, WIDTH, 1, 1)
            .samples
            .view()
            .encode_into(&mut row);
        b.iter(|| checksum::accumulate(black_box(&row), 0));
    });
    group.finish();
}

/// `integration/open/<source>/plain/{i16,mosaic}`,
/// `integration/read_image/<source>/<storage>/<input>`,
/// `integration/read_section/<source>/<storage>/<input>`,
/// `integration/read_physical/<source>/<storage>/<input>`,
/// `integration/verify_checksum/<source>/plain/<input>`,
/// `integration/read_table/<source>/<storage>/catalog`,
/// `integration/read_table_rows/<source>/<storage>/catalog`,
/// `integration/read_table_columns/<source>/<storage>/{catalog,spectra}`,
/// `integration/read_table_cells/<source>/<storage>/{catalog,spectra}` and
/// `integration/read_ascii_table/<source>/plain/catalog`, from opening the file to the last
/// sample, on every storage each input takes.
pub(crate) fn integration(c: &mut Criterion) {
    let mut files = SceneFiles::new("read");
    open(c, &mut files);
    read_image(c, &mut files);
    read_section(c, &mut files);
    read_physical(c, &mut files);
    verify_checksum(c, &mut files);
    read_table(c, &mut files);
    read_table_rows(c, &mut files);
    read_table_columns(c, &mut files);
    read_table_cells(c, &mut files);
    read_ascii_table(c, &mut files);
}

/// `integration/open/<source>/plain/{i16,mosaic}`: the header scan of the light, one HDU, and
/// of the mosaic, 61, then the last HDU selected, by `EXTNAME` in the mosaic. Per HDU.
fn open(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "open", Scale::Row);
    for file in [
        SceneFile::plain(SceneInput::Frame(Bitpix::I16)),
        SceneFile::plain(SceneInput::Mosaic),
    ] {
        group.throughput(Throughput::Elements(file.hdu() as u64 + 1));
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                source.bench(b, files.path(file), Open { file });
            });
        }
    }
    group.finish();
}

/// `integration/read_image/<source>/<storage>/<input>`: the image decoded into owned samples,
/// checked once against the scene before it is timed. Per sample, uncompressed.
fn read_image(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_image", Scale::Frame);
    for file in SceneFile::images() {
        group.throughput(image_throughput(file));
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                let path = files.path(file);
                let bytes = fs_read(path);
                let mut reader = FitsReader::from_bytes(&bytes).unwrap();
                let read = ReadImage { hdu: file.hdu() }.read_from(&mut reader);
                file.assert_read(&mut reader, &read);
                source.bench(b, path, ReadImage { hdu: file.hdu() });
            });
        }
    }
    group.finish();
}

/// `integration/read_section/<source>/<storage>/<input>`: the image as the lumos light reader
/// takes it, ten rows a section into one reused buffer, each section fed to the data
/// checksum, then the checksum judged. Per sample, uncompressed.
fn read_section(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_section", Scale::Frame);
    for file in SceneFile::images() {
        group.throughput(image_throughput(file));
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                let reading = ReadSections {
                    hdu: file.hdu(),
                    words: Vec::new(),
                };
                source.bench(b, files.path(file), reading);
            });
        }
    }
    group.finish();
}

/// `integration/read_physical/<source>/<storage>/<input>`: the image's physical values as
/// `f32`, ten rows a section into one reused buffer, as a stacker takes a light. Per sample,
/// uncompressed.
fn read_physical(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_physical", Scale::Frame);
    for file in SceneFile::images() {
        group.throughput(image_throughput(file));
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                let reading = ReadPhysical {
                    hdu: file.hdu(),
                    values: Vec::new(),
                };
                source.bench(b, files.path(file), reading);
            });
        }
    }
    group.finish();
}

/// `integration/verify_checksum/<source>/plain/<input>`: the data checksum of each plain
/// frame, checked once to be valid before it is timed. The sum reads the stored bytes, so a
/// codec does not change the work. Per sample.
fn verify_checksum(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "verify_checksum", Scale::Frame);
    for file in SceneFile::plain_frames() {
        group.throughput(image_throughput(file));
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                let path = files.path(file);
                let bytes = fs_read(path);
                let mut reader = FitsReader::from_bytes(&bytes).unwrap();
                let report = VerifyChecksum { hdu: file.hdu() }.verify(&mut reader);
                assert!(report.is_valid(), "{}: {report:?}", file.id());
                source.bench(b, path, VerifyChecksum { hdu: file.hdu() });
            });
        }
    }
    group.finish();
}

/// `integration/read_table/<source>/<storage>/catalog`: the source catalog read as a table and
/// every column decoded to its physical values into one reused buffer, as a caller reads a
/// catalog. Per row, in the table's bytes.
fn read_table(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_table", Scale::Frame);
    group.throughput(policy::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    for file in SceneFile::tables(SceneInput::Catalog) {
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                source.bench(b, files.path(file), ReadTable);
            });
        }
    }
    group.finish();
}

/// `integration/read_table_rows/<source>/<storage>/catalog`: the source catalog in ten chunks
/// of rows, each read as a table and every column decoded, beside `read_table`. Per row, in
/// the table's bytes.
fn read_table_rows(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_table_rows", Scale::Frame);
    group.throughput(policy::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    for file in SceneFile::tables(SceneInput::Catalog) {
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                source.bench(b, files.path(file), ReadRows);
            });
        }
    }
    group.finish();
}

/// `integration/read_table_columns/<source>/<storage>/{catalog,spectra}`: three fixed columns,
/// 20 bytes of each row, in ten chunks of rows into one reused [`Selection`], as a caller
/// pages through a table larger than it holds: a source's position and flux in the catalog,
/// the columns beside the array in the spectra. Per row, in the bytes of those columns.
fn read_table_columns(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_table_columns", Scale::Frame);
    for input in [SceneInput::Catalog, SceneInput::Spectra] {
        let table = TableInput::of(input);
        group.throughput(policy::throughput(table.rows, 8 + 8 + 4));
        for file in SceneFile::tables(input) {
            for source in SourceKind::all() {
                group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                    let reading = ReadColumns {
                        table,
                        selection: None,
                    };
                    source.bench(b, files.path(file), reading);
                });
            }
        }
    }
    group.finish();
}

/// `integration/read_table_cells/<source>/<storage>/{catalog,spectra}`: the `FLUX` cell of
/// [`CELLS`] rows a fixed seed draws, one call each, as a viewer reads a source it points at:
/// an `f32` in the catalog, an array of 1000 to 4000 in the spectra. Per cell.
fn read_table_cells(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_table_cells", Scale::Frame);
    group.throughput(Throughput::Elements(CELLS as u64));
    for input in [SceneInput::Catalog, SceneInput::Spectra] {
        let table_rows = TableInput::of(input).rows as u64;
        let mut draws = SplitMix::new(5);
        let rows: Vec<usize> = (0..CELLS)
            .map(|_| usize::try_from(draws.next() % table_rows).unwrap())
            .collect();
        for file in SceneFile::tables(input) {
            for source in SourceKind::all() {
                group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
                    source.bench(b, files.path(file), ReadCells { rows: &rows });
                });
            }
        }
    }
    group.finish();
}

/// `integration/read_ascii_table/<source>/plain/catalog`: the source catalog as an ASCII table,
/// every column parsed to its physical values. Per row, in the table's bytes.
fn read_ascii_table(c: &mut Criterion, files: &mut SceneFiles) {
    let mut group = Kind::Integration.group(c, "read_ascii_table", Scale::Frame);
    group.throughput(policy::throughput(CATALOG_ROWS, ASCII_CATALOG_ROW_BYTES));
    let file = SceneFile::plain(SceneInput::AsciiCatalog);
    for source in SourceKind::all() {
        group.bench_function(BenchmarkId::new(source.id(), file.id()), |b| {
            source.bench(b, files.path(file), ReadAscii);
        });
    }
    group.finish();
}

/// The throughput of reading the image of `file`: its samples, in the bytes they take
/// uncompressed.
const fn image_throughput(file: SceneFile) -> Throughput {
    let bitpix = file.input.bitpix().expect("an image file");
    policy::throughput(WIDTH * HEIGHT, bitpix.elem_size())
}

/// The bytes of the file at `path`, read whole, untimed.
fn fs_read(path: &Path) -> Vec<u8> {
    std::fs::read(path).expect("a bench file is read")
}

#[derive(Debug)]
struct ReadImage {
    hdu: usize,
}

impl ReadImage {
    fn read_from<S: Source>(&self, reader: &mut FitsReader<S>) -> Data {
        reader
            .image(self.hdu)
            .unwrap()
            .read(Region::All)
            .unwrap()
            .into_samples()
    }
}

impl Reading for ReadImage {
    type Output = Data;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Data {
        self.read_from(&mut reader)
    }
}

#[derive(Debug)]
struct ReadSections {
    hdu: usize,
    words: Vec<u64>,
}

impl Reading for ReadSections {
    type Output = ChecksumReport;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> ChecksumReport {
        let mut image = reader.image(self.hdu).unwrap().with_checksums();
        for top in (0..HEIGHT).step_by(SECTION_ROWS) {
            let ranges = [0..WIDTH, top..(top + SECTION_ROWS).min(HEIGHT)];
            let section = image.view(&ranges, &mut self.words).unwrap();
            black_box(&section);
        }
        image.finish().unwrap()
    }
}

#[derive(Debug)]
struct Open {
    file: SceneFile,
}

impl Reading for Open {
    type Output = ();

    fn read<S: Source>(&mut self, reader: FitsReader<S>) {
        let hdu = match self.file.input {
            SceneInput::Mosaic => reader.hdu(scene::ccd_name(MOSAIC_CCDS).as_str()),
            _ => reader.hdu(self.file.hdu()),
        };
        assert_eq!(hdu.unwrap().index, self.file.hdu(), "the last HDU");
    }
}

#[derive(Debug)]
struct ReadPhysical {
    hdu: usize,
    values: Vec<f32>,
}

impl Reading for ReadPhysical {
    type Output = ();

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) {
        let mut image = reader.image(self.hdu).unwrap();
        for top in (0..HEIGHT).step_by(SECTION_ROWS) {
            let ranges = [0..WIDTH, top..(top + SECTION_ROWS).min(HEIGHT)];
            self.values.clear();
            image.physical_into(&ranges, &mut self.values).unwrap();
            black_box(&self.values);
        }
    }
}

#[derive(Debug)]
struct VerifyChecksum {
    hdu: usize,
}

impl VerifyChecksum {
    fn verify<S: Source>(&self, reader: &mut FitsReader<S>) -> ChecksumReport {
        reader.verify_checksum(self.hdu).unwrap()
    }
}

impl Reading for VerifyChecksum {
    type Output = ChecksumReport;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> ChecksumReport {
        self.verify(&mut reader)
    }
}

/// Decodes every column of `table` to its physical values, into one buffer a caller
/// reuses from column to column.
fn decode_columns(table: &Table) {
    let mut values: Vec<f64> = Vec::new();
    for index in 0..table.schema().columns.len() {
        values.clear();
        table
            .column(index)
            .unwrap()
            .physical_into(&mut values)
            .unwrap();
        black_box(&values);
    }
}

#[derive(Debug)]
struct ReadTable;

impl Reading for ReadTable {
    type Output = Table;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Table {
        let table = reader.read_table(1).unwrap();
        decode_columns(&table);
        table
    }
}

#[derive(Debug)]
struct ReadRows;

impl Reading for ReadRows {
    type Output = ();

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) {
        let chunk = CATALOG_ROWS / 10;
        for start in (0..CATALOG_ROWS).step_by(chunk) {
            let rows = start..(start + chunk).min(CATALOG_ROWS);
            decode_columns(&reader.read_table_rows(1, rows).unwrap());
        }
    }
}

/// A binary table a column read pages through: its rows and the fixed columns it takes.
#[derive(Debug, Clone, Copy)]
struct TableInput {
    rows: usize,
    columns: [&'static str; 3],
}

impl TableInput {
    fn of(input: SceneInput) -> Self {
        match input {
            SceneInput::Catalog => Self {
                rows: CATALOG_ROWS,
                columns: ["RA", "DEC", "FLUX"],
            },
            SceneInput::Spectra => Self {
                rows: SPECTRA_ROWS,
                columns: ["RA", "DEC", "NUMBER"],
            },
            _ => unreachable!("a binary table input"),
        }
    }
}

#[derive(Debug)]
struct ReadColumns {
    table: TableInput,
    /// The selection each chunk refills, kept across iterations as a caller keeps it.
    selection: Option<Selection>,
}

impl Reading for ReadColumns {
    type Output = ();

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) {
        let TableInput {
            rows: table_rows,
            columns,
        } = self.table;
        let chunk = table_rows / 10;
        for start in (0..table_rows).step_by(chunk) {
            let rows = start..(start + chunk).min(table_rows);
            match &mut self.selection {
                Some(selection) => reader
                    .read_table_columns_into(1, rows, columns, selection)
                    .unwrap(),
                None => {
                    self.selection = Some(reader.read_table_columns(1, rows, columns).unwrap());
                }
            }
            black_box(&self.selection);
        }
    }
}

#[derive(Debug)]
struct ReadCells<'a> {
    rows: &'a [usize],
}

impl Reading for ReadCells<'_> {
    type Output = ();

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) {
        for &row in self.rows {
            black_box(reader.read_table_cell(1, row, "FLUX").unwrap());
        }
    }
}

#[derive(Debug)]
struct ReadAscii;

impl Reading for ReadAscii {
    type Output = Ascii;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Ascii {
        let table = reader.read_ascii_table(1).unwrap();
        let mut values: Vec<f64> = Vec::new();
        for index in 0..table.schema().columns.len() {
            values.clear();
            table
                .column(index)
                .unwrap()
                .physical_into(&mut values)
                .unwrap();
            black_box(&values);
        }
        table
    }
}
