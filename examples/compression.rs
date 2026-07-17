//! Write a tile-compressed image (FITS §10) and read it back losslessly. Needs the
//! `compression` feature, which is on by default:
//!
//! ```sh
//! cargo run --example compression
//! ```

use std::fs::File;

use fits_well::image::{Compression, CompressionOptions, Image, ImageData};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_compressed.fits");

    let expected = ImageData::I16((0..256).map(|i| (i % 32) as i16).collect());
    let image = Image::new(vec![16, 16], expected.clone())?;

    // Compress with RICE in 8×8 tiles. `CompressionOptions::tiled` sets the tile shape
    // while the typed codec prevents invalid or misspelled choices.
    let options = CompressionOptions::tiled([8, 8]);
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_compressed_image(&image, Compression::Rice, &options)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // A compressed image lives in a BINTABLE extension, but `image_indices` reports
    // it as an image all the same — so you find and read it without knowing it sits
    // at HDU 1, or that it's compressed at all. `read_image` detects `ZIMAGE` and
    // decompresses transparently — the same call as for a plain image.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let images = reader.image_indices();
    println!("image HDUs: {images:?}"); // [1] — the compressed image extension
    let restored = reader.read_image(images[0])?;
    let restored_shape = restored.metadata().shape.to_vec();
    let lossless = restored.decode() == expected;
    println!("restored {:?}, lossless = {}", restored_shape, lossless);

    Ok(())
}
