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

    // A compressed image is stored in a BINTABLE extension (HDU 1). `read_image`
    // detects `ZIMAGE` and transparently decompresses it — same call as a plain image.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let restored = reader.read_image(1)?;
    println!(
        "restored {:?}, lossless = {}",
        restored.shape,
        restored.decode() == image.samples
    );

    Ok(())
}
