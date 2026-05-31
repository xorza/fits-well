//! End-to-end tour of the public `fits` API.
//!
//! Run it on the bundled sample, or pass your own file:
//!
//! ```sh
//! cargo run --example read_write
//! cargo run --example read_write -- path/to/file.fits
//! ```

use std::env;
use std::fs::File;
use std::io::Cursor;

use fits::{FitsReader, FitsWriter, HduKind, ImageData, ZERO_FILL};

fn main() -> fits::Result<()> {
    let path = env::args()
        .nth(1)
        .unwrap_or_else(|| "tests/data/fits/UITfuv2582gc.fits".to_string());

    // ---- open: scans HDU boundaries from headers alone, no data is read ----
    let mut reader = FitsReader::open(File::open(&path)?)?;
    println!("opened {path:?} — {} HDU(s)\n", reader.hdus.len());

    // ---- inspect each header through the typed getters ----
    for (i, hdu) in reader.hdus.iter().enumerate() {
        println!("HDU {i}: {:?}", hdu.kind);
        if let Ok(bitpix) = hdu.header.bitpix() {
            println!(
                "  BITPIX = {:>3} ({bitpix:?}, {} byte/elem)",
                bitpix.code(),
                bitpix.elem_size()
            );
        }
        if let Ok(axes) = hdu.header.axes() {
            println!("  NAXIS  = {} {:?}", axes.len(), axes);
        }
        // Reserved keywords are optional — print whichever are present.
        for keyword in ["OBJECT", "TELESCOP", "INSTRUME", "DATE-OBS"] {
            if let Some(text) = hdu.header.get_text(keyword) {
                println!("  {keyword:8} = {text:?}");
            }
        }
        println!();
    }

    // ---- decode the first image HDU that actually carries pixels ----
    let image_hdu = reader.hdus.iter().position(|hdu| {
        matches!(hdu.kind, HduKind::Primary | HduKind::Image) && hdu.header.naxis().unwrap_or(0) > 0
    });

    if let Some(index) = image_hdu {
        let image = reader.read_image(index)?;
        println!(
            "read_image({index}): shape {:?}, {:?}, {} samples",
            image.shape,
            image.samples.bitpix(),
            image.samples.len()
        );

        // Peek at the raw (stored) plane.
        if let ImageData::I16(samples) = &image.samples {
            println!(
                "  first raw samples: {:?}",
                &samples[..5.min(samples.len())]
            );
        }

        // The physical plane applies BZERO + BSCALE (and turns BLANK into NaN).
        let s = &image.scaling;
        println!(
            "  scaling: BSCALE={}, BZERO={}, BLANK={:?} (identity={})",
            s.bscale,
            s.bzero,
            s.blank,
            s.is_identity()
        );
        let physical = image.physical();
        let mut min = f64::INFINITY;
        let mut max = f64::NEG_INFINITY;
        let mut sum = 0.0;
        let mut n = 0u64;
        for &v in physical.iter().filter(|v| !v.is_nan()) {
            min = min.min(v);
            max = max.max(v);
            sum += v;
            n += 1;
        }
        println!(
            "  physical: min={min}, max={max}, mean={:.2}\n",
            sum / n as f64
        );
    }

    // ---- the raw data unit: padded on-disk bytes plus the real-data range ----
    let unit = reader.read_data_raw(0)?;
    println!(
        "read_data_raw(0): {} bytes on disk, {} bytes of data, {} bytes of block fill\n",
        unit.bytes.len(),
        unit.data().len(),
        unit.bytes.len() - unit.data().len()
    );

    // ---- write the primary HDU back out, then re-open it from memory ----
    let header = reader.hdus[0].header.clone();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_header(&header)?;
    writer.write_data_unit(unit.data(), ZERO_FILL)?; // writer pads to the 2880 grid
    let written = writer.into_inner().into_inner();

    let reopened = FitsReader::open(Cursor::new(written.clone()))?;
    println!(
        "round-trip: wrote {} bytes, re-opened {} HDU(s), primary axes {:?}",
        written.len(),
        reopened.hdus.len(),
        reopened.hdus[0].header.axes()?
    );

    Ok(())
}
