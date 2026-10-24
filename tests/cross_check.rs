//! Files for a cross-check against the reference implementations, which no test here can
//! run: one image for each codec, a quantized float image, and a table plain and
//! compressed. Run it on request:
//!
//! ```sh
//! cargo drive test -- --ignored writes_files_for_the_reference_readers
//! ```
//!
//! Then read each image with astropy (`astropy.io.fits.open(path)[1].data`) and compare
//! it with the formula beside it below, and unpack the table with cfitsio
//! (`funpack -O unpacked.fits compressed_table.fits`) and compare it with `table.fits`.

#![cfg(feature = "compression")]
#![expect(
    clippy::print_stdout,
    reason = "the test reports where it wrote the files"
)]

use std::fs::{self, File};
use std::path::Path;

use fits_well::FitsWriter;
use fits_well::compress::{Compression, CompressionConfig, Hcompress};
use fits_well::image::{Data, Image, Scaling};
use fits_well::table::{Builder, ColumnData, WriteColumn};

const WIDTH: usize = 24;
const HEIGHT: usize = 16;

#[test]
#[ignore = "writes files for a manual check with astropy and funpack"]
fn writes_files_for_the_reference_readers() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("cross-check");
    fs::create_dir_all(&dir).unwrap();
    let xy = |i: usize| (i % WIDTH, i / WIDTH);

    // 7x − 5y.
    let ramp: Vec<i16> = (0..WIDTH * HEIGHT)
        .map(|i| {
            let (x, y) = xy(i);
            i16::try_from(7 * x).unwrap() - i16::try_from(5 * y).unwrap()
        })
        .collect();
    let ramp = Image::new(vec![WIDTH, HEIGHT], ramp, Scaling::IDENTITY).unwrap();
    for (compression, tiles) in [
        (Compression::GZIP, &[][..]),
        (Compression::GZIP_SHUFFLED, &[]),
        (Compression::Rice, &[]),
        (
            Compression::Hcompress(Hcompress::default()),
            &[WIDTH, HEIGHT],
        ),
    ] {
        let name = format!("{}.fits", compression.name().to_lowercase());
        write_image(
            &dir.join(name),
            &ramp,
            &CompressionConfig::new(compression).with_tiles(tiles),
        );
    }

    // (x + y) mod 7: PLIO takes no negative value.
    let mask: Vec<i32> = (0..WIDTH * HEIGHT)
        .map(|i| {
            let (x, y) = xy(i);
            i32::try_from((x + y) % 7).unwrap()
        })
        .collect();
    let mask = Image::new(vec![WIDTH, HEIGHT], mask, Scaling::IDENTITY).unwrap();
    write_image(
        &dir.join("plio_1.fits"),
        &mask,
        &CompressionConfig::new(Compression::Plio),
    );

    // 100 + 3x − 2y, quantized with SUBTRACTIVE_DITHER_1.
    let floats: Vec<f32> = (0..WIDTH * HEIGHT)
        .map(|i| {
            let (x, y) = xy(i);
            100.0 + 3.0 * f32::from(u8::try_from(x).unwrap())
                - 2.0 * f32::from(u8::try_from(y).unwrap())
        })
        .collect();
    let floats = Image::new(vec![WIDTH, HEIGHT], Data::F32(floats), Scaling::IDENTITY).unwrap();
    write_image(
        &dir.join("rice_float.fits"),
        &floats,
        &CompressionConfig::new(Compression::Rice).with_tiles([WIDTH, HEIGHT]),
    );

    // 1000 rows: ID = i, FLUX = 0.5i − 3, FLAG = i mod 7, in tiles of 100 rows.
    let mut table = Builder::new();
    table
        .push(WriteColumn::scalar(
            "ID",
            ColumnData::I32((0..1000).collect()),
        ))
        .unwrap()
        .push(WriteColumn::scalar(
            "FLUX",
            ColumnData::F64((0..1000).map(|i| f64::from(i) * 0.5 - 3.0).collect()),
        ))
        .unwrap()
        .push(WriteColumn::scalar(
            "FLAG",
            ColumnData::I16((0..1000).map(|i| i % 7).collect()),
        ))
        .unwrap();
    let path = dir.join("table.fits");
    FitsWriter::new(File::create(&path).unwrap())
        .write_table(&table, None)
        .unwrap();
    println!("wrote {}", path.display());
    let path = dir.join("compressed_table.fits");
    FitsWriter::new(File::create(&path).unwrap())
        .write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::Rice).with_tiles([100]),
            None,
        )
        .unwrap();
    println!("wrote {}", path.display());
}

fn write_image(path: &Path, image: &Image, config: &CompressionConfig) {
    FitsWriter::new(File::create(path).unwrap())
        .write_compressed_image(image.view(), config, None)
        .unwrap();
    println!("wrote {}", path.display());
}
