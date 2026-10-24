//! Write tile-compressed images and a compressed table (FITS §10), and read them back
//! with the calls a plain file takes. Needs the `compression` feature, which is on by
//! default:
//!
//! ```sh
//! cargo run --example compression
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use fits_well::compress::{Compression, CompressionConfig, Dither, DitherSeed, Quantization};
use fits_well::image::{Data, Image, Scaling};
use fits_well::table::{Builder, ColumnData, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_compression.fits");

    let counts = Image::new(
        vec![64, 64],
        Data::I16((0..64 * 64).map(|i| (i % 64) as i16).collect()),
        Scaling::IDENTITY,
    )?;
    let sky: Vec<f32> = (0..64 * 64)
        .map(|i| 100.0 + (i % 7) as f32 * 0.25)
        .collect();
    let sky = Image::new(vec![64, 64], Data::F32(sky), Scaling::IDENTITY)?;

    // An integer image compresses losslessly. A float image is quantized: the levels are
    // spaced at each tile's noise divided by 16, with a dither so that the quantization
    // error is not the same in every tile. `Quantization::NONE` keeps every float bit.
    let rice = CompressionConfig::new(Compression::Rice).with_tiles([64, 16]);
    let dither = Dither::Subtractive1(DitherSeed::FROM_SAMPLES);
    let quantized = CompressionConfig::new(Compression::Rice)
        .with_quantization(Quantization::relative(16.0, dither)?);
    let lossless =
        CompressionConfig::new(Compression::GZIP_SHUFFLED).with_quantization(Quantization::NONE);

    let mut catalog = Builder::new();
    catalog
        .push(WriteColumn::scalar(
            "ID",
            ColumnData::I32((0..1000).collect()),
        ))?
        .push(WriteColumn::scalar(
            "FLUX",
            ColumnData::F64((0..1000).map(|i| f64::from(i) * 0.5).collect()),
        ))?;

    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_compressed_image(counts.view(), &rice, None)?;
    writer.write_compressed_image(sky.view(), &quantized, None)?;
    writer.write_compressed_image(sky.view(), &lossless, None)?;
    writer.write_compressed_table(
        &catalog,
        &CompressionConfig::new(Compression::Rice).with_tiles([100]),
        None,
    )?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // Each compressed image is a BINTABLE extension; `images` lists it as an image.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let indices: Vec<usize> = reader.images().map(|hdu| hdu.index).collect();
    println!("image HDUs: {indices:?}");

    let restored = reader.image(indices[0])?.read(..)?;
    println!(
        "RICE i16, lossless: {}",
        restored.samples() == counts.samples()
    );

    let restored = reader.image(indices[1])?.read(..)?;
    let (Data::F32(original), Data::F32(restored)) = (sky.samples(), restored.samples()) else {
        unreachable!("both images hold f32 samples");
    };
    let worst = original
        .iter()
        .zip(restored)
        .map(|(a, b)| (a - b).abs())
        .fold(0.0, f32::max);
    println!("RICE f32, quantized: largest error {worst}");

    let restored = reader.image(indices[2])?.read(..)?;
    println!(
        "GZIP_2 f32, lossless: {}",
        restored.samples() == sky.samples()
    );

    // The compressed table reads as the table it holds, here only its rows 10 to 12.
    let table = reader.read_table_rows(4, 10..13)?;
    println!("FLUX rows 10..13: {:?}", table.column("FLUX")?.physical()?);
    Ok(())
}
