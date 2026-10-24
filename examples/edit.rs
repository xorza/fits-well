//! Edit a file that exists: replace a header in place, append an HDU, and verify the
//! checksums after each edit:
//!
//! ```sh
//! cargo run --example edit
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::{File, OpenOptions};

use fits_well::image::{Image, Scaling};
use fits_well::{FitsEditor, FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_edit.fits");

    // Spare records leave room in each header, so a later edit fits in place.
    let image = Image::new(vec![8, 8], vec![1i16; 64], Scaling::IDENTITY)?;
    let mut writer = FitsWriter::new(File::create(&path)?)
        .with_checksums()
        .with_spare_records(16);
    writer.write_image(image.view(), None)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // After a plate solve, for example: new keywords in the header, the data unchanged.
    // The replaced header gets a valid CHECKSUM.
    let mut editor = FitsEditor::open(OpenOptions::new().read(true).write(true).open(&path)?)?;
    let mut header = editor.hdu(0)?.header.clone();
    header.set("OBJECT", "M31")?.set("EXPTIME", 300.0)?;
    editor.replace_header(0, &header)?;

    // `append` gives a writer at the end of the file.
    let mut writer = editor.append()?.with_checksums();
    writer.write_image(image.view(), None)?;
    writer.into_inner().sync_all()?;

    let mut reader = FitsReader::open(File::open(&path)?)?;
    println!(
        "{} HDUs, OBJECT {:?}",
        reader.hdus().len(),
        reader.hdus()[0].header.get_text("OBJECT")?
    );
    for hdu in 0..reader.hdus().len() {
        let report = reader.verify_checksum(hdu)?;
        println!("HDU {hdu}: checksums valid: {}", report.is_valid());
    }
    Ok(())
}
