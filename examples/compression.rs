//! Write a tile-compressed image (FITS §10) and read it back losslessly. Needs the
//! `compression` feature, which is on by default:
//!
//! ```sh
//! cargo run --example compression
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::fs::File;

use fits_well::image::{Compression, CompressionConfig, Data, Image, Region};
use fits_well::{FitsReader, FitsWriter};
use std::env;

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_compressed.fits");

    let expected = Data::I16((0..256).map(|i| i % 32).collect());
    let image = Image::new(vec![16, 16], expected.clone())?;

    // Compress with RICE in 8×8 tiles. The typed codec prevents invalid or misspelled
    // choices, and `with_tiles` sets the tile shape.
    let config = CompressionConfig::new(Compression::Rice).with_tiles([8, 8]);
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_compressed_image(image.view(), &config, None)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // A compressed image lives in a BINTABLE extension, but `images` reports it as an
    // image all the same — so you find and read it without knowing it sits at HDU 1,
    // or that it's compressed at all. `image` detects `ZIMAGE` and decompresses
    // transparently — the same calls as for a plain image.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let first = reader
        .images()
        .next()
        .expect("the file holds the compressed image")
        .index;
    println!("first image HDU: {first}"); // 1 — the compressed image extension
    let restored = reader.image(first)?.read(Region::All)?;
    let restored_shape = restored.view().shape().to_vec();
    let lossless = restored.into_samples() == expected;
    println!("restored {restored_shape:?}, lossless = {lossless}");

    Ok(())
}
