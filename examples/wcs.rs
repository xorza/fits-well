//! Convert between pixel and world (sky) coordinates with a WCS header:
//!
//! ```sh
//! cargo run --example wcs
//! ```

use fits_well::{Header, Wcs};

fn main() -> fits_well::Result<()> {
    // A gnomonic (TAN) projection centred on RA = 250°, Dec = 30°, at 0.001°/pixel.
    // In practice these keywords come straight from a file's header; here we build
    // one by hand so the example is self-contained.
    let mut header = Header::new();
    header
        .set("NAXIS", 2)
        .set("CTYPE1", "RA---TAN")
        .set("CTYPE2", "DEC--TAN")
        .set("CRPIX1", 50.0) // reference pixel …
        .set("CRPIX2", 50.0)
        .set("CRVAL1", 250.0) // … and the sky coordinate it sits at
        .set("CRVAL2", 30.0)
        .set("CDELT1", -0.001)
        .set("CDELT2", 0.001);

    let wcs = Wcs::from_header(&header, None)?;

    // Pixel → world: the reference pixel maps to the reference coordinate.
    let sky = wcs.pixel_to_world(&[50.0, 50.0]);
    println!("pixel (50, 50)  ->  RA/Dec {sky:?}");

    // World → pixel is the inverse transform.
    let pixel = wcs.world_to_pixel(&[250.05, 30.05]);
    println!("RA/Dec (250.05, 30.05)  ->  pixel {pixel:?}");

    Ok(())
}
