//! Read the time of an observation (FITS §9) from a header: its time frame, its start,
//! end and exposure, and a datetime's MJD:
//!
//! ```sh
//! cargo run --example time
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use fits_well::header::Header;
use fits_well::time::{Bounds, Coordinate, Datetime, Frame};

fn main() -> fits_well::Result<()> {
    let mut header = Header::new();
    header
        .set("TIMESYS", "TT")?
        .set("MJDREF", 51_544.5)?
        .set("DATE-OBS", "2024-03-14T15:09:26.5")?
        .set("DATE-BEG", "2024-03-14T15:09:26.5")?
        .set("DATE-END", "2024-03-14T15:14:26.5")?
        .set("XPOSURE", 300.0)?;

    // The frame: the scale, the reference epoch and the unit of the relative times.
    let frame = Frame::from_header(&header)?;
    println!(
        "scale {}, unit {}, MJDREF {:?}",
        frame.scale, frame.unit, frame.reference_mjd
    );

    // Each bound reads its own keywords, so a bad one fails alone.
    let bounds = Bounds::new(&header, &frame);
    if let Some(start) = bounds.start()? {
        println!(
            "start: MJD {} + {:.9} {}",
            start.mjd.day(),
            start.mjd.fraction(),
            start.scale
        );
    }
    if let Some(end) = bounds.end()? {
        println!(
            "end:   MJD {} + {:.9} {}",
            end.mjd.day(),
            end.mjd.fraction(),
            end.scale
        );
    }
    println!("exposure: {:?} s", bounds.exposure()?);
    println!(
        "observation: {:?}",
        Coordinate::observation(&header)?.map(|time| time.mjd)
    );

    // A datetime parses from its ISO-8601 text and prints back as the same text.
    let datetime: Datetime = "2024-03-14T15:09:26.5".parse()?;
    println!("{datetime} is MJD {:?}", datetime.to_mjd()?);
    Ok(())
}
