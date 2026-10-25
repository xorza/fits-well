//! Write an ASCII table, the text form of a FITS table, and read it back:
//!
//! ```sh
//! cargo run --example ascii_table
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use fits_well::table::{
    AsciiBuilder, AsciiColumnData, AsciiFloatFormat, AsciiText, AsciiWriteColumn,
};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_ascii_table.fits");

    // Each column has a field width; `None` is a null, written as the TNULLn text.
    let names: AsciiText = [Some("Vega"), Some("Sirius"), None].into_iter().collect();
    let mut table = AsciiBuilder::new();
    table
        .push(AsciiWriteColumn::text("NAME", names, 8).with_null("-"))?
        .push(AsciiWriteColumn::integer(
            "ID",
            [Some(1), Some(2), Some(3)],
            4,
        ))?
        .push(
            AsciiWriteColumn::float(
                "MAG",
                [Some(0.03), Some(-1.46), None],
                AsciiFloatFormat::F {
                    width: 7,
                    decimals: 2,
                },
            )
            .with_unit("mag")
            .with_null("NaN"),
        )?;

    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_ascii_table(&table, None)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    let mut reader = FitsReader::open(File::open(&path)?)?;
    let table = reader.read_ascii_table(1)?;
    println!(
        "{} rows of {} bytes",
        table.schema().row_count(),
        table.schema().row_len()
    );
    // Each read fills a buffer the caller keeps, here one for the fields and one for the
    // physical values.
    let mut fields = AsciiColumnData::Integer(Vec::new());
    table.column("NAME")?.raw_into(&mut fields)?;
    if let AsciiColumnData::Text(names) = &fields {
        println!("NAME {:?}", names.iter().collect::<Vec<_>>());
    }
    table.column("ID")?.raw_into(&mut fields)?;
    println!("ID   {fields:?}");
    // A null field reads as NaN on the physical plane.
    let mut magnitudes: Vec<f64> = Vec::new();
    table.column("MAG")?.physical_into(&mut magnitudes)?;
    println!("MAG  {magnitudes:?}");
    Ok(())
}
