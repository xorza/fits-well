//! Create images, write them to a FITS file, and read them back two ways — an owned
//! decode you can keep, and the borrowed `view` hot path:
//!
//! ```sh
//! cargo run --example image
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::fs::File;

use fits_well::image::{Data, Image, Region, Scaling, View};
use fits_well::{FitsReader, FitsWriter};
use std::env;

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_image.fits");

    // A 4×3 image of signed 16-bit pixels. `shape` is fastest-axis-first
    // (NAXIS1 = 4), and `samples` is the flat row-major buffer.
    let i16_image = Image::new_scaled(
        vec![4, 3],
        Data::I16(vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23]),
        Scaling::IDENTITY,
    )?;
    // A second image of a *different* type (32-bit float) — so the file holds two
    // image HDUs of differing BITPIX, which the view loop below reads into one buffer.
    let f32_image = Image::new_scaled(
        vec![2, 2],
        Data::F32(vec![1.5, -2.5, 3.5, -4.5]),
        Scaling::IDENTITY,
    )?;

    // Writing synthesizes the mandatory header (SIMPLE/XTENSION, BITPIX, NAXISn) and
    // the big-endian data unit. The first `write_image` is the primary array; the
    // second becomes an `IMAGE` extension.
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(i16_image.view(), None)?;
    writer.write_image(f32_image.view(), None)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // `images` lists the HDUs that hold images, so you pick indices rather than
    // hard-coding them — here the primary plus the extension, `[0, 1]`.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let images: Vec<usize> = reader.images().map(|hdu| hdu.index).collect();
    println!("image HDUs: {images:?}");

    // `image` resolves the HDU's geometry once; `metadata` reads it without touching
    // the data. `physical_into` applies BSCALE/BZERO, maps any BLANK value to NaN, and
    // appends to a buffer you own; `read` copies the samples into an owned image.
    let mut image = reader.image(images[0])?;
    let metadata = image.metadata();
    println!("hdu 0: shape {:?}, {:?}", metadata.shape, metadata.bitpix);
    let mut physical: Vec<f64> = Vec::new();
    image.physical_into(Region::All, &mut physical)?;
    println!("  physical {physical:?}");
    if let Data::I16(pixels) = image.read(Region::All)?.into_samples() {
        println!("  pixels   {pixels:?}");
    }

    // `view` byte-swaps into a *caller-owned* scratch instead of allocating a fresh
    // buffer per call — so a loop over many images pays the output allocation once and
    // reuses it across reads, even across differing BITPIX (i16 then f32 here). The
    // reader retains nothing image-sized; you own `scratch` and drop it when done.
    let mut scratch: Vec<u64> = Vec::new();
    for &idx in &images {
        // The view borrows the reader + scratch, so use it before the next read. For
        // samples you need past the loop, use the owned `read` above.
        let mut image = reader.image(idx)?;
        let view = image.view(Region::All, &mut scratch)?;
        match view.samples() {
            View::I16(v) => println!("hdu {idx}: i16 view {v:?}"),
            View::F32(v) => println!("hdu {idx}: f32 view {v:?}"),
            other => println!(
                "hdu {idx}: {:?} view, {} samples",
                other.bitpix(),
                other.len()
            ),
        }
    }

    Ok(())
}
