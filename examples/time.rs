//! Record an observation time in a FITS header, read it back, and convert between
//! ISO-8601, Julian Date, and time scales:
//!
//! ```sh
//! cargo run --example time
//! ```

use std::fs::File;

use fits_well::{Datetime, FitsReader, FitsWriter, Header, TimeScale};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_time.fits");

    // A header-only HDU (NAXIS = 0) recording when an observation was taken — the
    // standard §9 time keywords an instrument writes.
    let mut header = Header::new();
    header
        .try_set("SIMPLE", true)?
        .try_set("BITPIX", 8)?
        .try_set("NAXIS", 0)?
        .try_set("DATE-OBS", "2024-03-14T15:09:26")?
        .try_set("MJD-OBS", 60383.631551)?
        .try_set("TIMESYS", "UTC")?;
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_raw_hdu(&header, &[])?; // NAXIS=0 → header only, no data unit
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // Read the file and pull the time metadata from its header.
    let reader = FitsReader::open(File::open(&path)?)?;
    let header = &reader.hdus()[0].header;

    // `header.obs_mjd()` resolves the observation time (MJD-OBS, else DATE-OBS).
    println!("observation MJD = {:?}", header.obs_mjd()?);

    let timesys = TimeScale::parse(
        header
            .get_text("TIMESYS")?
            .expect("example header sets TIMESYS"),
    );

    // The DATE-OBS string itself parses to a `Datetime`, then to Julian Date.
    let t = Datetime::parse(
        header
            .get_text("DATE-OBS")?
            .expect("example header sets DATE-OBS"),
    )?;
    let jd = t.to_jd(timesys)?;
    println!("DATE-OBS -> JD {:.5}, MJD {:.5}", jd, t.to_mjd(timesys)?);

    // Convert that instant from the header's TIMESYS (UTC) to Terrestrial Time.
    let jd_tt = timesys.convert(jd, TimeScale::parse("TT"));
    println!("UTC -> TT differs by {:.3} s", (jd_tt - jd) * 86400.0);

    Ok(())
}
