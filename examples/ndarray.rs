//! Read a FITS image as an n-dimensional [`ndarray`] array. Needs the `ndarray`
//! feature:
//!
//! ```sh
//! cargo run --example ndarray --features ndarray
//! ```

use std::fs::File;

use fits_well::{FitsReader, FitsWriter, Image, ImageArray, ImageData, Scaling};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_ndarray.fits");

    // A 4×3 image: NAXIS1 = 4 (columns, the fastest-varying axis), NAXIS2 = 3 (rows).
    // `samples` is the flat buffer in FITS order — each row's 4 pixels, row by row.
    let image = Image {
        shape: vec![4, 3],
        #[rustfmt::skip]
        samples: ImageData::I16(vec![
             0,  1,  2,  3,   // y = 0
            10, 11, 12, 13,   // y = 1
            20, 21, 22, 23,   // y = 2
        ]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(&image)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    let mut reader = FitsReader::open(File::open(&path)?)?;
    let images = reader.image_indices();
    let raw = reader.read_image(images[0])?;

    // `physical_array()` applies BSCALE/BZERO and returns an `ndarray` you can index
    // and reduce. Axes are FITS-native, so index `[x, y]` (x along NAXIS1, y NAXIS2).
    let arr = raw.physical_array();
    println!("shape {:?}", arr.shape()); // [4, 3]
    println!("pixel (x=2, y=1) = {}", arr[[2, 1]]); // 12
    println!("mean = {:?}", arr.mean()); // an ndarray reduction, for free

    // NumPy/Astropy index images `[y, x]`; `reversed_axes()` gives that view for free
    // (a zero-copy stride swap).
    let numpy = arr.reversed_axes();
    println!("numpy [y=1, x=2] = {}", numpy[[1, 2]]); // also 12

    // `to_ndarray()` keeps the exact element type instead of widening to f64.
    if let ImageArray::I16(a) = raw.to_ndarray() {
        println!("typed i16 [x=0, y=2] = {}", a[[0, 2]]); // 20
    }

    Ok(())
}
