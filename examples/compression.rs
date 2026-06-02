//! Write a tile-compressed image (FITS §10) and read it back losslessly. Needs the
//! `compression` feature, which is on by default:
//!
//! ```sh
//! cargo run --example compression
//! ```

use std::fs::File;

use fits_well::{CompressOptions, FitsReader, FitsWriter, Image, ImageData, Scaling};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_compressed.fits");

    let image = Image {
        shape: vec![16, 16],
        samples: ImageData::I16((0..256).map(|i| (i % 32) as i16).collect()),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };

    // Compress with RICE in 8×8 tiles. `CompressOptions::tiled` sets the tile shape
    // and leaves the gzip level / HCOMPRESS scale at their defaults.
    let options = CompressOptions::tiled([8, 8]);
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_compressed_image(&image, "RICE_1", &options)?;
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
    println!(
        "restored {:?}, lossless = {}",
        restored.shape,
        restored.decode() == image.samples
    );

    Ok(())
}
