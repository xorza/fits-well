//! Build a header with typed keywords, HIERARCH records and commentary, read it record
//! by record, and parse a near-valid header that the reader repairs:
//!
//! ```sh
//! cargo run --example header
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use fits_well::FitsError;
use fits_well::header::{CommentaryName, Header, RecordView, keywords};

fn main() -> fits_well::Result<()> {
    let mut header = Header::new();
    header
        .set(keywords::OBJECT, "M31")?
        .set("EXPTIME", 300.0)?
        .set_comment("EXPTIME", "seconds")?
        .set("HIERARCH ESO DET DIT", 2.5)?
        .push_commentary(CommentaryName::HISTORY, "dark subtracted")?
        .push_commentary(CommentaryName::HISTORY, "flat fielded")?;

    // Each typed getter gives `None` for an absent record and an error for one of
    // another type.
    println!("OBJECT  {:?}", header.get_text("OBJECT")?);
    println!(
        "EXPTIME {:?} ({:?})",
        header.get_real("EXPTIME")?,
        header.get_comment("EXPTIME")?
    );
    println!("ESO DIT {:?}", header.get_real("HIERARCH ESO DET DIT")?);
    println!("FILTER  {:?}", header.get_text("FILTER")?);
    match header.get_integer("OBJECT") {
        Err(FitsError::ValueTypeMismatch { name, expected }) => {
            println!("{name} is not {expected}");
        }
        other => println!("unexpected: {other:?}"),
    }
    let history: Vec<&str> = header.commentary(CommentaryName::HISTORY).collect();
    println!("HISTORY {history:?}");

    for record in &header {
        match record {
            RecordView::Valued { keyword, value, .. } => println!("  {keyword} = {value:?}"),
            RecordView::Hierarch { name, value, .. } => println!("  HIERARCH {name} = {value:?}"),
            RecordView::Commentary { name, text } => println!("  {name} {}", text.unwrap_or("")),
            RecordView::Malformed { raw, .. } => {
                println!("  malformed {:?}", String::from_utf8_lossy(raw));
            }
        }
    }

    // A lowercase keyword and a value indicator in byte 8 read as cfitsio and astropy
    // read them; each repair is listed with its 1-based record.
    let mut bytes = Vec::new();
    for card in [
        "SIMPLE  =                    T",
        "exptime =                 30.0",
        "GAIN   =1.5",
        "END",
    ] {
        bytes.extend_from_slice(format!("{card:<80}").as_bytes());
    }
    bytes.resize(2880, b' ');
    let parsed = Header::parse(&bytes)?;
    println!(
        "EXPTIME {:?}, GAIN {:?}",
        parsed.get_real("EXPTIME")?,
        parsed.get_real("GAIN")?
    );
    for repair in parsed.repairs() {
        println!("  record {}: {:?}", repair.record, repair.kind);
    }
    Ok(())
}
