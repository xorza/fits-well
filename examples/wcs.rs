//! Read the WCS (World Coordinate System) from a FITS file's header and convert
//! between pixel and sky coordinates:
//!
//! ```sh
//! cargo run --example wcs
//! ```

use std::fs::File;

use fits_well::FitsReader;

fn main() -> fits_well::Result<()> {
    // A FITS image stores its WCS as header keywords — CTYPEn (projection), CRPIXn
    // (reference pixel), CRVALn (its sky coordinate), CDELTn (scale), and so on.
    // This bundled file uses a TAN (gnomonic) projection.
    let reader = FitsReader::open(File::open("tests/data/fits/wcs_tan.fits")?)?;
    let header = &reader.hdus()[0].header;

    // `header.wcs(..)` parses those keywords into a usable transform. `None` selects
    // the primary WCS (an alternate would be `Some('A')`, etc.).
    let wcs = header.wcs(None)?;
    println!("axes: {:?}", wcs.view().axes);

    // Pixel → world: the reference pixel (CRPIXn) maps to the reference sky
    // coordinate (CRVALn). This file's reference pixel is (256, 256).
    let mut reference = [0.0; 2];
    wcs.pixel_to_world(&[256.0, 256.0], &mut reference);
    println!("pixel (256, 256) -> RA/Dec {reference:?}");

    // One pixel over in X moves a small amount across the sky.
    let mut neighbour = [0.0; 2];
    wcs.pixel_to_world(&[257.0, 256.0], &mut neighbour);
    println!("pixel (257, 256) -> RA/Dec {neighbour:?}");

    // World → pixel is the inverse — mapping the reference coordinate back lands on
    // the reference pixel again.
    let mut pixel = [0.0; 2];
    wcs.world_to_pixel(&reference, &mut pixel);
    println!("that RA/Dec       -> pixel {pixel:?}");

    Ok(())
}
