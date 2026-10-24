//! Describe a world coordinate system in a header, read it from a file, convert between
//! pixel and sky coordinates, and derive the WCS of a cut-out:
//!
//! ```sh
//! cargo run --example wcs
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use fits_well::header::Header;
use fits_well::image::{Image, Scaling};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_wcs.fits");

    // A gnomonic (TAN) projection centred on the Crab nebula, 1 arcsecond a pixel.
    let mut header = Header::new();
    header
        .set("CTYPE1", "RA---TAN")?
        .set("CTYPE2", "DEC--TAN")?
        .set("CRPIX1", 50.5)?
        .set("CRPIX2", 50.5)?
        .set("CRVAL1", 83.633)?
        .set("CRVAL2", 22.0145)?
        .set("CDELT1", -1.0 / 3600.0)?
        .set("CDELT2", 1.0 / 3600.0)?;
    let image = Image::new(vec![100, 100], vec![0i16; 100 * 100], Scaling::IDENTITY)?;
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(image.view(), Some(&header))?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // `None` selects the primary description; an alternate is a letter, `'A'` to `'Z'`.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let wcs = reader.read_wcs(0, None)?;
    for axis in wcs.description().axes {
        println!(
            "{}: CRVAL {} at CRPIX {}",
            axis.ctype, axis.crval, axis.crpix
        );
    }

    // Each transform writes one point into a slice the caller owns.
    let mut sky = [0.0; 2];
    wcs.pixel_to_world(&[1.0, 1.0], &mut sky)?;
    println!(
        "pixel (1, 1) -> RA {:.6} deg, Dec {:.6} deg",
        sky[0], sky[1]
    );
    let mut pixel = [0.0; 2];
    wcs.world_to_pixel(&sky, &mut pixel)?;
    println!("and back -> pixel ({:.9}, {:.9})", pixel[0], pixel[1]);
    let declination = wcs.axis_world(1, &[1.0, 1.0])?;
    println!("axis 2 alone: {} {}", declination.value, declination.unit);

    // A cut-out of columns 21 to 40 and rows 31 to 50 keeps its sky: its pixel (1, 1)
    // is pixel (21, 31) of the full image.
    let cutout = wcs.crop((&[20..40, 30..50]).into())?;
    let mut corner = [0.0; 2];
    cutout.pixel_to_world(&[1.0, 1.0], &mut corner)?;
    wcs.pixel_to_world(&[21.0, 31.0], &mut sky)?;
    println!("cut-out corner {corner:?} = full image {sky:?}");

    // The cut-out's description, written into the header that goes with its pixels.
    let mut cutout_header = Header::new();
    cutout.write_into(&mut cutout_header, None)?;
    println!(
        "CRPIX1 of the cut-out: {:?}",
        cutout_header.get_real("CRPIX1")?
    );
    Ok(())
}
