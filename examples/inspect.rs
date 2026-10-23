//! Open a FITS file and describe its HDUs and headers — the read-only inspection
//! path. From a checkout, `test_resources/UITfuv2582gc.fits` is a sample:
//!
//! ```sh
//! cargo run --example inspect -- path/to/file.fits
//! ```

#![expect(
    clippy::print_stdout,
    clippy::print_stderr,
    reason = "an example reports to the terminal"
)]

use std::env;
use std::fs::File;
use std::process;

use fits_well::FitsReader;

fn main() -> fits_well::Result<()> {
    let Some(path) = env::args().nth(1) else {
        eprintln!("usage: inspect <file.fits>");
        process::exit(2);
    };

    // `open` scans the HDU boundaries from the headers alone — no pixel data read.
    let reader = FitsReader::open(File::open(&path)?)?;
    println!("{path}: {} HDU(s)", reader.hdus().len());

    for (i, hdu) in reader.hdus().iter().enumerate() {
        println!("\nHDU {i}: {:?}", hdu.kind);

        // An image's geometry, compressed or not, from its header alone.
        if let Ok(metadata) = hdu.image_metadata() {
            println!("  dimensions = {:?}", metadata.shape);
        }

        // The typed getters return `None` when a keyword is absent.
        for keyword in ["OBJECT", "TELESCOP", "INSTRUME", "DATE-OBS", "BUNIT"] {
            if let Some(value) = hdu.header.get_text(keyword)? {
                println!("  {keyword:8} = {value}");
            }
        }
    }

    Ok(())
}
