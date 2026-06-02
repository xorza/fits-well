//! Create an image, write it to a FITS file, and read it back:
//!
//! ```sh
//! cargo run --example image
//! ```

use std::fs::File;

use fits_well::{FitsReader, FitsWriter, Image, ImageData, Scaling};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_image.fits");

    // A 4×3 image of signed 16-bit pixels. `shape` is fastest-axis-first
    // (NAXIS1 = 4), and `samples` is the flat row-major buffer.
    let image = Image {
        shape: vec![4, 3],
        #[rustfmt::skip]
        samples: ImageData::I16(vec![
             0,  1,  2,  3,
            10, 11, 12, 13,
            20, 21, 22, 23,
        ]),
        // physical value = bzero + bscale * stored; identity here, no blanks.
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };

    // Writing synthesizes the mandatory header (SIMPLE, BITPIX, NAXISn) and the
    // big-endian data unit. `FitsWriter::new` takes any `Write` (a `File` here).
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(&image)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // `image_indices` lists the HDUs that hold images, so you pick an index rather
    // than hard-coding one — a FITS file may hold several. Here it's just the primary
    // array, `[0]`.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let images = reader.image_indices();
    println!("image HDUs: {images:?}");

    // `read_image` borrows the data unit in place (zero-copy) as a `RawImage`: shape
    // and BITPIX are ready at once, while the samples stay undecoded until you ask.
    let raw = reader.read_image(images[0])?;
    println!("shape {:?}, {:?}", raw.shape, raw.bitpix);

    // `decode()` byte-swaps the stored big-endian samples into an owned, host-endian
    // buffer. (For a BITPIX=8 image, `raw.u8()` hands the bytes back with no copy.)
    if let ImageData::I16(pixels) = raw.decode() {
        println!("pixels  {pixels:?}");
    }
    // `physical()` applies BSCALE/BZERO and turns any BLANK value into NaN.
    println!("physical {:?}", raw.physical());

    Ok(())
}
