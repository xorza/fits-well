//! Write a binary table, then read its columns back:
//!
//! ```sh
//! cargo run --example table
//! ```

use std::fs::File;

use fits_well::{ColumnData, FitsReader, FitsWriter, WriteColumn};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_table.fits");

    // Each column holds typed data; the last argument is the per-row element count
    // (the character width for a text column, 1 for a plain scalar column).
    let columns = [
        WriteColumn::fixed("ID", ColumnData::I32(vec![1, 2, 3]), 1),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Text(vec!["Vega".into(), "Sirius".into(), "Rigel".into()]),
            8,
        ),
        WriteColumn::fixed("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13]), 1).with_unit("mag"),
    ];

    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_table(3, &columns)?; // 3 rows
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // The table is HDU 1 — HDU 0 is the automatic empty primary array that every
    // FITS file begins with.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let table = reader.read_table(1)?;

    println!("{} rows, {} columns", table.nrows, table.columns.len());
    println!("ID   = {:?}", table.read_column(0)?);
    println!("NAME = {:?}", table.read_column(1)?);
    println!("MAG  = {:?}", table.read_column(2)?);

    Ok(())
}
