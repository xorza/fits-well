//! Write a binary table, then read its columns back:
//!
//! ```sh
//! cargo run --example table
//! ```

use std::fs::File;

use fits_well::table::{ColumnData, TableBuilder, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_table.fits");

    // Each column holds typed data; the last argument is the per-row element count
    // (the character width for a text column, 1 for a plain scalar column).
    let table = TableBuilder::new()
        .column(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
        .column(WriteColumn::fixed(
            "NAME",
            ColumnData::Character(vec!["Vega".into(), "Sirius".into(), "Rigel".into()]),
            8,
        ))?
        .column(
            WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13])).with_unit("mag"),
        )?;

    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_table(&table)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // The table is HDU 1 — HDU 0 is the automatic empty primary array that every
    // FITS file begins with.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let table = reader.read_table(1)?;
    let metadata = table.metadata();

    println!(
        "{} rows, {} columns",
        metadata.nrows,
        metadata.columns.len()
    );
    // Address a column by index or by `TTYPEn` name; the handle decodes on demand.
    println!("ID   = {:?}", table.column_by_idx(0)?.raw()?);
    println!("NAME = {:?}", table.column_by_name("NAME")?.raw()?);
    println!("MAG  = {:?}", table.column_by_name("MAG")?.raw()?);

    // `.raw()` is the stored, typed plane. The handle also interprets: `.physical()`
    // applies `TZEROn + TSCALn ×` and maps `TNULLn` to NaN, widening any numeric
    // column to `f64` (MAG is unscaled here, so these are just the stored values).
    // `.unsigned()`, `.complex()`, and `.bits()` cover the unsigned, complex, and
    // bit-array columns the same way — no need to pass the column descriptor back.
    println!(
        "MAG (physical) = {:?}",
        table.column_by_name("MAG")?.physical()?
    );

    Ok(())
}
