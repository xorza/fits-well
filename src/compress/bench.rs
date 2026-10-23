//! The compression cases: each codec on one tile, and compressed images and tables written to
//! and read from files. The inputs are the scene's, never random bytes, as a codec's speed
//! follows its data: the light for the integer codecs, its defect mask for `PLIO_1`, the float
//! frame for quantized compression, the source catalog for tables.

use std::fs::File;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion};

use crate::bench::fixture::{self, Kind, Reading, Scale, ScratchDir, SourceKind};
use crate::bench::scene::{self, CATALOG_ROW_BYTES, CATALOG_ROWS, HEIGHT, WIDTH};
use crate::bintable::Table;
use crate::bitpix::Bitpix;
use crate::compress::encode::hcompress_default_rows;
use crate::compress::hcompress::Smoothing;
use crate::compress::plane::IntBitpix;
use crate::compress::{
    Compression, CompressionConfig, Hcompress, convert, gzip, hcompress, plio, rice,
};
use crate::fits_writer::FitsWriter;
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::region::Region;
use crate::reader::FitsReader;
use crate::reader::source::Source;

/// The codecs an image case compresses with, and the `BITPIX` of the scene image each takes:
/// the integer codecs the light, `PLIO_1` the mask, the float-capable ones the float frame.
/// `HCOMPRESS_1` at its default, lossless scale.
fn image_codecs() -> [(Compression, Bitpix); 8] {
    [
        (Compression::GZIP, Bitpix::I16),
        (Compression::GZIP_SHUFFLED, Bitpix::I16),
        (Compression::Rice, Bitpix::I16),
        (Compression::Hcompress(Hcompress::default()), Bitpix::I16),
        (Compression::Plio, Bitpix::U8),
        (Compression::GZIP, Bitpix::F32),
        (Compression::GZIP_SHUFFLED, Bitpix::F32),
        (Compression::Rice, Bitpix::F32),
    ]
}

/// The codecs a table case compresses with.
const TABLE_CODECS: [Compression; 3] = [
    Compression::GZIP,
    Compression::GZIP_SHUFFLED,
    Compression::Rice,
];

/// The rows of one table tile: a chunk of the catalog, so it splits into 25 tiles.
const ROWS_PER_TILE: usize = 4096;

/// `atomic/compress/<codec>/<type>` and `atomic/decompress/<codec>/<type>`: one tile of the
/// default tiling of the standard frame ([`tile_rows`] rows), through each integer codec's
/// tile encoder and decoder, into reused buffers. A decode case first checks that the tile
/// round-trips, as each codec is lossless here. Per sample, in the stored bytes.
pub(crate) fn atomic(c: &mut Criterion) {
    let codecs = image_codecs()
        .into_iter()
        .filter(|(_, bitpix)| bitpix.is_integer());
    let mut group = Kind::Atomic.group(c, "compress", Scale::Sample);
    for (compression, bitpix) in codecs.clone() {
        group.throughput(fixture::throughput(
            WIDTH * tile_rows(compression),
            bitpix.elem_size(),
        ));
        group.bench_function(case_id(compression, bitpix), |b| {
            let mut tile = Tile::new(compression, bitpix);
            b.iter(|| tile.encode());
        });
    }
    group.finish();

    let mut group = Kind::Atomic.group(c, "decompress", Scale::Sample);
    for (compression, bitpix) in codecs {
        group.throughput(fixture::throughput(
            WIDTH * tile_rows(compression),
            bitpix.elem_size(),
        ));
        group.bench_function(case_id(compression, bitpix), |b| {
            let mut tile = Tile::new(compression, bitpix);
            tile.encode();
            tile.decode();
            assert_eq!(tile.decoded, tile.values, "the codec round-trips the tile");
            b.iter(|| tile.decode());
        });
    }
    group.finish();
}

/// `integration/write_compressed/file/<codec>/<type>`, `integration/read_compressed/<source>/
/// <codec>/<type>`, `integration/write_compressed_table/file/<codec>` and
/// `integration/read_table/<source>/<codec>`: the standard frame under the light
/// header in the default row tiles, a float frame quantized; the catalog in tiles of 4096 rows.
/// From creating or opening the file to the last byte or sample. Per sample or row,
/// uncompressed.
pub(crate) fn integration(c: &mut Criterion) {
    let dir = ScratchDir::new("compress");
    write_compressed(c, &dir);
    read_compressed(c, &dir);
    write_compressed_table(c, &dir);
    read_table(c, &dir);
}

fn write_compressed(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "write_compressed", Scale::Frame);
    let path = dir.file("frame.fits");
    for (compression, bitpix) in image_codecs() {
        let mut image = None;
        group.throughput(fixture::throughput(WIDTH * HEIGHT, bitpix.elem_size()));
        let id = BenchmarkId::new("file", case_id(compression, bitpix));
        group.bench_function(id, |b| {
            let image = image.get_or_insert_with(|| scene_image(compression, bitpix, HEIGHT));
            let header = scene::light_header();
            b.iter(|| {
                let mut writer = FitsWriter::new(File::create(&path).unwrap()).with_checksums();
                writer
                    .write_compressed_image(
                        image.view(),
                        &CompressionConfig::new(compression),
                        Some(&header),
                    )
                    .unwrap();
            });
        });
    }
    group.finish();
}

fn read_compressed(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_compressed", Scale::Frame);
    for (compression, bitpix) in image_codecs() {
        let mut path = None;
        group.throughput(fixture::throughput(WIDTH * HEIGHT, bitpix.elem_size()));
        for source in SourceKind::all() {
            let id = BenchmarkId::new(source.id(), case_id(compression, bitpix));
            group.bench_function(id, |b| {
                let path = path.get_or_insert_with(|| {
                    let name = format!("{}.fits", case_id(compression, bitpix).replace('/', "_"));
                    dir.write(&name, &compressed_file(compression, bitpix))
                });
                source.bench(b, path, ReadCompressed);
            });
        }
    }
    group.finish();
}

fn write_compressed_table(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "write_compressed_table", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    let path = dir.file("catalog.fits");
    let mut catalog = None;
    for compression in TABLE_CODECS {
        group.bench_function(BenchmarkId::new("file", codec_id(compression)), |b| {
            let catalog = catalog.get_or_insert_with(scene::catalog);
            let config = table_config(compression);
            b.iter(|| {
                let mut writer = FitsWriter::new(File::create(&path).unwrap());
                writer
                    .write_compressed_table(catalog, &config, None)
                    .unwrap();
            });
        });
    }
    group.finish();
}

fn read_table(c: &mut Criterion, dir: &ScratchDir) {
    let mut group = Kind::Integration.group(c, "read_table", Scale::Frame);
    group.throughput(fixture::throughput(CATALOG_ROWS, CATALOG_ROW_BYTES));
    for compression in TABLE_CODECS {
        let mut path: Option<PathBuf> = None;
        for source in SourceKind::all() {
            group.bench_function(BenchmarkId::new(source.id(), codec_id(compression)), |b| {
                let path = path.get_or_insert_with(|| {
                    let mut writer = FitsWriter::new(Vec::new());
                    writer
                        .write_compressed_table(&scene::catalog(), &table_config(compression), None)
                        .unwrap();
                    let name = format!("catalog_{}.fits", codec_id(compression));
                    dir.write(&name, &writer.into_inner())
                });
                source.bench(b, path, ReadCompressedTable);
            });
        }
    }
    group.finish();
}

/// The first `height` rows of the scene image `compression` takes in `bitpix`: the defect mask
/// for `PLIO_1`, else the standard frame.
fn scene_image(compression: Compression, bitpix: Bitpix, height: usize) -> Image {
    if matches!(compression, Compression::Plio) {
        scene::mask(WIDTH, height, 1)
    } else {
        scene::sky(bitpix, WIDTH, height, 1)
    }
}

/// The bytes of a file holding [`scene_image`] compressed by `compression` under the light
/// header, with its checksums.
fn compressed_file(compression: Compression, bitpix: Bitpix) -> Vec<u8> {
    let mut writer = FitsWriter::new(Vec::new()).with_checksums();
    writer
        .write_compressed_image(
            scene_image(compression, bitpix, HEIGHT).view(),
            &CompressionConfig::new(compression),
            Some(&scene::light_header()),
        )
        .unwrap();
    writer.into_inner()
}

/// The catalog's tiles of [`ROWS_PER_TILE`] rows, compressed by `compression`.
fn table_config(compression: Compression) -> CompressionConfig {
    CompressionConfig::new(compression).tiled([ROWS_PER_TILE])
}

fn codec_id(compression: Compression) -> String {
    compression.name().to_lowercase()
}

fn case_id(compression: Compression, bitpix: Bitpix) -> String {
    format!("{}/{}", codec_id(compression), fixture::bitpix_id(bitpix))
}

/// The rows of one default tile of the standard frame: one, or cfitsio's `HCOMPRESS_1` height.
const fn tile_rows(compression: Compression) -> usize {
    match compression {
        Compression::Hcompress(_) => hcompress_default_rows(HEIGHT),
        _ => 1,
    }
}

/// One tile of the light, or of its mask for `PLIO_1`, and the reused buffers its codec writes.
#[derive(Debug)]
struct Tile {
    compression: Compression,
    bitpix: IntBitpix,
    values: Vec<i64>,
    be: Vec<u8>,
    encoded: Vec<u8>,
    decoded: Vec<i64>,
    gzip: gzip::GzipScratch,
    rice: rice::RiceScratch,
    hcompress: hcompress::HcompressScratch,
}

impl Tile {
    fn new(compression: Compression, bitpix: Bitpix) -> Self {
        let plane = scene_image(compression, bitpix, tile_rows(compression));
        let values = match plane.samples {
            Data::U8(values) => values.into_iter().map(i64::from).collect(),
            Data::I16(values) => values.into_iter().map(i64::from).collect(),
            _ => unreachable!("the integer codecs take u8 and i16 tiles"),
        };
        let bitpix = match bitpix {
            Bitpix::U8 => IntBitpix::U8,
            _ => IntBitpix::I16,
        };
        Self {
            compression,
            bitpix,
            values,
            be: Vec::new(),
            encoded: Vec::new(),
            decoded: Vec::new(),
            gzip: gzip::GzipScratch::default(),
            rice: rice::RiceScratch::default(),
            hcompress: hcompress::HcompressScratch::default(),
        }
    }

    /// Encodes the tile into [`Self::encoded`], as the tile encoder of `compress_image` does.
    fn encode(&mut self) {
        let level = self.compression.gzip_level();
        self.encoded = match self.compression {
            Compression::Gzip(config) if config.shuffle => {
                convert::i64_to_be_into(&self.values, self.bitpix, &mut self.be);
                gzip::gzip2_encode(&self.be, self.bitpix.elem_size(), level, &mut self.gzip)
            }
            Compression::Gzip(_) => {
                convert::i64_to_be_into(&self.values, self.bitpix, &mut self.be);
                gzip::gzip_encode(&self.be, level)
            }
            Compression::Rice => {
                rice::rice_encode(&self.values, self.bitpix, rice::BLOCKSIZE, &mut self.rice)
            }
            Compression::Hcompress(_) => {
                let tdims = [WIDTH, tile_rows(self.compression)];
                hcompress::hcompress_tile_encode(&self.values, &tdims, 0, &mut self.hcompress)
                    .unwrap()
            }
            Compression::Plio => plio::plio_encode(&self.values)
                .unwrap()
                .into_iter()
                .flat_map(i16::to_be_bytes)
                .collect(),
            Compression::None => unreachable!("no case stores a tile uncompressed"),
        };
    }

    /// Decodes [`Self::encoded`] into [`Self::decoded`], as the tile decoder of a read does.
    fn decode(&mut self) {
        let (bytes, out) = (&self.encoded, &mut self.decoded);
        match self.compression {
            Compression::Gzip(config) if config.shuffle => {
                gzip::gzip2_tile_into(bytes, self.bitpix, WIDTH, out, &mut self.gzip)
            }
            Compression::Gzip(_) => {
                gzip::gzip_tile_into(bytes, self.bitpix, WIDTH, out, &mut self.gzip)
            }
            Compression::Rice => {
                rice::rice_decode_into(bytes, WIDTH, self.bitpix, rice::BLOCKSIZE, out)
            }
            Compression::Hcompress(_) => {
                let elems = WIDTH * tile_rows(self.compression);
                hcompress::hcompress_tile_into(
                    bytes,
                    Smoothing::Off,
                    elems,
                    out,
                    &mut self.hcompress,
                )
            }
            Compression::Plio => plio::plio_decode_be_into(bytes, WIDTH, out),
            Compression::None => unreachable!("no case stores a tile uncompressed"),
        }
        .unwrap();
    }
}

#[derive(Debug)]
struct ReadCompressed;

impl Reading for ReadCompressed {
    type Output = Data;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Data {
        reader
            .image(1)
            .unwrap()
            .read(Region::All)
            .unwrap()
            .into_samples()
    }
}

#[derive(Debug)]
struct ReadCompressedTable;

impl Reading for ReadCompressedTable {
    type Output = Table;

    fn read<S: Source>(&mut self, mut reader: FitsReader<S>) -> Table {
        reader.read_table(1).unwrap()
    }
}
