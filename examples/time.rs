//! Record an observation time in a FITS header, read it back, and resolve its
//! ISO-8601 and Julian Date representations:
//!
//! ```sh
//! cargo run --example time
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::fs::File;

use fits_well::header::Header;
use fits_well::time::{Datetime, TimeCoordinate, TimeScale};
use fits_well::{FitsReader, FitsWriter};
use std::env;

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_time.fits");

    // A header-only HDU (NAXIS = 0) recording when an observation was taken — the
    // standard §9 time keywords an instrument writes.
    let mut header = Header::new();
    header
        .set("SIMPLE", true)?
        .set("BITPIX", 8)?
        .set("NAXIS", 0)?
        .set("DATE-OBS", "2024-03-14T15:09:26")?
        .set("MJD-OBS", 60_383.631_551)?
        .set("TIMESYS", "UTC")?;
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_raw_hdu(&header, &[])?; // NAXIS=0 → header only, no data unit
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // Read the file and pull the time metadata from its header.
    let reader = FitsReader::open(File::open(&path)?)?;
    let header = &reader.hdus()[0].header;

    // `TimeCoordinate::observation` resolves the observation time (MJD-OBS, else
    // DATE-OBS) with its scale.
    println!("observation = {:?}", TimeCoordinate::observation(header)?);

    let timesys = header
        .get_text("TIMESYS")?
        .expect("example header sets TIMESYS")
        .parse::<TimeScale>()?;

    // The DATE-OBS string itself parses to a `Datetime`, then to Julian Date.
    let t = Datetime::parse(
        header
            .get_text("DATE-OBS")?
            .expect("example header sets DATE-OBS"),
    )?;
    let jd = t.to_jd(&timesys)?;
    println!(
        "DATE-OBS -> JD {:.5}, MJD {:.5} in {timesys:?}",
        jd,
        t.to_mjd(&timesys)?
    );

    Ok(())
}
