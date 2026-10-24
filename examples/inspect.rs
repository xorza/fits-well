//! Describe the HDUs of a FITS file from their headers, with no data read. From a
//! checkout, `test_resources/UITfuv2582gc.fits` is a sample:
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

    let reader = FitsReader::open(File::open(&path)?)?;
    println!("{path}: {} HDUs", reader.hdus().len());
    for hdu in reader.hdus() {
        print!("\nHDU {}: {:?}", hdu.index, hdu.kind);
        if let Some(name) = hdu.extname()? {
            print!(" {name}");
        }
        println!(", {} data bytes", hdu.data_bytes);
        if hdu.is_image() {
            let metadata = hdu.image_metadata()?;
            println!("  shape {:?}, {:?}", metadata.shape, metadata.bitpix);
        }
        for keyword in ["OBJECT", "TELESCOP", "INSTRUME", "DATE-OBS", "BUNIT"] {
            if let Some(value) = hdu.header.get_text(keyword)? {
                println!("  {keyword:8} = {value}");
            }
        }
        for repair in hdu.header.repairs() {
            println!("  repaired record {}: {:?}", repair.record, repair.kind);
        }
    }
    Ok(())
}
