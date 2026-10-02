//! Read the WCS (World Coordinate System) from a FITS file's header and convert
//! between pixel and sky coordinates. From a checkout, `tests/data/fits/wcs_tan.fits`
//! is a two-axis TAN (gnomonic) sample:
//!
//! ```sh
//! cargo run --example wcs -- path/to/file.fits
//! ```

use std::env;
use std::fs::File;
use std::process;

use fits_well::FitsReader;
use fits_well::wcs::Wcs;

fn main() -> fits_well::Result<()> {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: wcs <file.fits>");
        process::exit(2);
    };
    // A FITS image stores its WCS as header keywords — CTYPEn (projection), CRPIXn
    // (reference pixel), CRVALn (its sky coordinate), CDELTn (scale), and so on.
    let reader = FitsReader::open(File::open(&path)?)?;
    let header = &reader.hdus()[0].header;

    // `Wcs::from_header` parses those keywords into a usable transform. `None` selects
    // the primary WCS (an alternate would be `Some('A')`, etc.).
    let wcs = Wcs::from_header(header, None)?;
    println!("axes: {:?}", wcs.view().axes);

    // Pixel → world: the reference pixel (CRPIXn) maps to the reference sky
    // coordinate (CRVALn).
    let reference_pixel: Vec<f64> = wcs.view().axes.iter().map(|axis| axis.crpix).collect();
    let reference = wcs.pixel_to_world(&reference_pixel)?;
    println!("pixel {reference_pixel:?} -> world {reference:?}");

    // One pixel over on the first axis moves a small amount across the sky.
    let mut next = reference_pixel.clone();
    next[0] += 1.0;
    let neighbour = wcs.pixel_to_world(&next)?;
    println!("pixel {next:?} -> world {neighbour:?}");

    // World → pixel is the inverse — mapping the reference coordinate back lands on
    // the reference pixel again.
    let pixel = wcs.world_to_pixel(&reference)?;
    println!("that world -> pixel {pixel:?}");

    Ok(())
}
