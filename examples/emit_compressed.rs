//! Write compressed files for a cross-check against the reference implementations:
//! one image per codec, a quantized float image, and a table both plain and
//! compressed with RICE.
//!
//! ```sh
//! cargo run --example emit_compressed -- OUT_DIR
//! ```
//!
//! Then read the images with astropy (`astropy.io.fits.open(path)[1].data`) and
//! compare them with the formulas below, and unpack the table with cfitsio
//! (`funpack -O unpacked.fits OUT_DIR/compressed_table.fits`) and compare it with
//! `OUT_DIR/table.fits`.

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "an example reports to the terminal"
)]

use std::env;
use std::fs::{self, File};
use std::path::Path;
use std::process;

use fits_well::FitsWriter;
use fits_well::image::{Compression, CompressionConfig, Data, Hcompress, Image};
use fits_well::table::{Builder, ColumnData, WriteColumn};

fn main() -> fits_well::Result<()> {
    let Some(out_dir) = env::args().nth(1) else {
        eprintln!("usage: emit_compressed OUT_DIR");
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
            &CompressionConfig::new(compression).with_tiles(tiles),
        )?;
    }

    // PLIO needs a non-negative mask: (x + y) mod 7.
    let mask: Vec<i32> = (0..24 * 16).map(|i| (i % 24 + i / 24) % 7).collect();
    let path = out_dir.join("plio_1.fits");
    write_image(
        &path,
        &Image::new(vec![24, 16], mask)?,
        &CompressionConfig::new(Compression::Plio),
    )?;

    // A quantized float (SUBTRACTIVE_DITHER_1): 100 + 3x − 2y.
    let floats: Vec<f32> = (0..24 * 16)
        .map(|i| 100.0 + 3.0 * (i % 24) as f32 - 2.0 * (i / 24) as f32)
        .collect();
    let path = out_dir.join("rice_float.fits");
    write_image(
        &path,
        &Image::new(vec![24, 16], Data::F32(floats))?,
        &CompressionConfig::new(Compression::Rice).with_tiles([24, 16]),
    )?;

    // 1000 rows: id = i, flux = 0.5i − 3, flag = i mod 7, in tiles of 100 rows.
    let table = Builder::new()
        .column(WriteColumn::scalar(
            "ID",
            ColumnData::I32((0..1000).collect()),
        ))?
        .column(WriteColumn::scalar(
            "FLUX",
            ColumnData::F64((0..1000).map(|i| f64::from(i) * 0.5 - 3.0).collect()),
        ))?
        .column(WriteColumn::scalar(
            "FLAG",
            ColumnData::I16((0..1000).map(|i| i % 7).collect()),
        ))?;
    let path = out_dir.join("table.fits");
    FitsWriter::new(File::create(&path)?).write_table(&table, None)?;
    println!("wrote {}", path.display());
    let path = out_dir.join("compressed_table.fits");
    FitsWriter::new(File::create(&path)?).write_compressed_table(
        &table,
        &CompressionConfig::new(Compression::Rice).with_tiles([100]),
        None,
    )?;
    println!("wrote {}", path.display());
    Ok(())
}

fn write_image(path: &Path, image: &Image, config: &CompressionConfig) -> fits_well::Result<()> {
    let mut writer = FitsWriter::new(File::create(path)?);
    writer.write_compressed_image(image.view(), config, None)?;
    println!("wrote {}", path.display());
    Ok(())
}
