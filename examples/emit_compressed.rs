//! Write compressed files for a cross-check against the reference implementations:
//! one image per codec, a quantized float image, and — given cfitsio's reference
//! table — that table compressed with RICE.
//!
//! ```sh
//! cargo run --example emit_compressed -- OUT_DIR [test_resources/comp_table_ref.fits]
//! ```
//!
//! Then read the images with astropy (`astropy.io.fits.open(path)[1].data`) and
//! compare them with the formulas below, and unpack the table with cfitsio
//! (`funpack -O unpacked.fits OUT_DIR/compressed_table.fits`) and compare it with
//! the reference it came from.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "an example reports to the terminal"
)]

use std::env;
use std::fs::{self, File};
use std::path::Path;
use std::process;

use fits_well::image::{Compression, CompressionOptions, Hcompress, Image, ImageData};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let Some(out_dir) = env::args().nth(1) else {
        eprintln!("usage: emit_compressed OUT_DIR [comp_table_ref.fits]");
        process::exit(2);
    };
    let out_dir = Path::new(&out_dir);
    fs::create_dir_all(out_dir)?;

    // value(x, y) = 7x − 5y over a 24×16 i16 image.
    let samples: Vec<i16> = (0..24 * 16)
        .map(|i| (i % 24) as i16 * 7 - (i / 24) as i16 * 5)
        .collect();
    let image = Image::new(vec![24, 16], samples)?;
    for (compression, tiles) in [
        (Compression::GZIP, &[][..]),
        (Compression::GZIP_SHUFFLED, &[]),
        (Compression::Rice, &[]),
        (Compression::Hcompress(Hcompress::default()), &[24, 16]),
    ] {
        let path = out_dir.join(format!("{}.fits", compression.name().to_lowercase()));
        write_image(
            &path,
            &image,
            compression,
            &CompressionOptions::tiled(tiles),
        )?;
    }

    // PLIO needs a non-negative mask: (x + y) mod 7.
    let mask: Vec<i32> = (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect();
    let path = out_dir.join("plio_1.fits");
    write_image(
        &path,
        &Image::new(vec![24, 16], mask)?,
        Compression::Plio,
        &CompressionOptions::default(),
    )?;

    // A quantized float (SUBTRACTIVE_DITHER_1): 100 + 3x − 2y.
    let floats: Vec<f32> = (0..24 * 16)
        .map(|i| 100.0 + 3.0 * (i % 24) as f32 - 2.0 * (i / 24) as f32)
        .collect();
    let path = out_dir.join("rice_float.fits");
    write_image(
        &path,
        &Image::new(vec![24, 16], ImageData::F32(floats))?,
        Compression::Rice,
        &CompressionOptions::tiled([24, 16]),
    )?;

    if let Some(reference) = env::args().nth(2) {
        let mut reader = FitsReader::open(File::open(&reference)?)?;
        let table = reader.read_table(1)?;
        let header = reader.hdus()[1].header.clone();
        let path = out_dir.join("compressed_table.fits");
        let mut writer = FitsWriter::new(File::create(&path)?);
        writer.write_compressed_table(&header, &table, 100, Compression::Rice)?;
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn write_image(
    path: &Path,
    image: &Image,
    compression: Compression,
    options: &CompressionOptions,
) -> fits_well::Result<()> {
    let mut writer = FitsWriter::new(File::create(path)?);
    writer.write_compressed_image(image, compression, options, None)?;
    println!("wrote {}", path.display());
    Ok(())
}
