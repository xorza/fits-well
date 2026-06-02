//! Open a FITS file and describe its HDUs and headers — the read-only inspection
//! path. Pass a path, or it falls back to a bundled sample:
//!
//! ```sh
//! cargo run --example inspect
//! cargo run --example inspect -- path/to/file.fits
//! ```

use std::fs::File;

use fits_well::FitsReader;

fn main() -> fits_well::Result<()> {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/data/fits/UITfuv2582gc.fits".into());

    // `open` scans the HDU boundaries from the headers alone — no pixel data read.
    let reader = FitsReader::open(File::open(&path)?)?;
    println!("{path}: {} HDU(s)", reader.hdus.len());

    for (i, hdu) in reader.hdus.iter().enumerate() {
        println!("\nHDU {i}: {:?}", hdu.kind);

        // `axes()` returns the NAXISn dimensions (empty for a header-only HDU).
        if let Ok(axes) = hdu.header.axes()
            && !axes.is_empty()
        {
            println!("  dimensions = {axes:?}");
        }

        // The typed getters return `None` when a keyword is absent.
        for keyword in ["OBJECT", "TELESCOP", "INSTRUME", "DATE-OBS", "BUNIT"] {
            if let Some(value) = hdu.header.get_text(keyword) {
                println!("  {keyword:8} = {value}");
            }
        }
    }

    Ok(())
}
