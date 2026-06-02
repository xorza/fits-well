//! FITS time coordinates: ISO-8601 ↔ Julian Date, epochs, and time-scale
//! conversion:
//!
//! ```sh
//! cargo run --example time
//! ```

use fits_well::{Datetime, Epoch, TimeScale};

fn main() -> fits_well::Result<()> {
    // Parse an ISO-8601 timestamp, then convert to Julian / Modified Julian Date.
    let t = Datetime::parse("2024-03-14T15:09:26")?;
    println!(
        "2024-03-14T15:09:26  ->  JD {:.5},  MJD {:.5}",
        t.to_jd(),
        t.to_mjd()
    );

    // Standard epochs (`J`ulian / `B`esselian).
    println!(
        "epoch J2000.0  ->  JD {:.1}",
        Epoch::parse("J2000.0")?.to_jd()
    );

    // Convert a Julian Date between time scales — here UTC to Terrestrial Time,
    // which differs by the accumulated leap seconds plus a fixed 32.184 s.
    let jd_utc = t.to_jd();
    let jd_tt = TimeScale::parse("UTC").convert(jd_utc, TimeScale::parse("TT"));
    println!("UTC -> TT  differs by {:.3} s", (jd_tt - jd_utc) * 86400.0);

    Ok(())
}
