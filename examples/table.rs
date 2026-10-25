//! Write a binary table with each kind of column, and read it back whole, by rows, by
//! columns and by cell:
//!
//! ```sh
//! cargo run --example table
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use bitvec::bitvec;
use bitvec::order::Msb0;
use fits_well::table::{Builder, ColumnData, DescriptorWidth, Ragged, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_table.fits");

    let mut table = Builder::new();
    table
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
        .push(WriteColumn::characters(
            "NAME",
            ["Vega", "Sirius", "Rigel"],
            8,
        )?)?
        .push(
            WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13])).with_unit("mag"),
        )?
        // Two values a row: a vector column.
        .push(WriteColumn::fixed(
            "POS",
            ColumnData::F64(vec![279.2, 38.8, 101.3, -16.7, 78.6, -8.2]),
            2,
        ))?
        // A different count each row: a variable-length array in the heap.
        .push(WriteColumn::vla(
            "FLUXES",
            Ragged::from_rows([
                ColumnData::F32(vec![1.0]),
                ColumnData::F32(vec![2.0, 2.5]),
                ColumnData::F32(vec![]),
            ])?,
            DescriptorWidth::P,
        ))?
        .push(WriteColumn::bits(
            "FLAGS",
            [
                bitvec![u8, Msb0; 1, 0, 1],
                bitvec![u8, Msb0; 0, 0, 0],
                bitvec![u8, Msb0; 1, 1, 0],
            ],
            3,
        )?)?;

    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_table(&table, None)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // HDU 0 is the empty primary array that every FITS file begins with.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let table = reader.read_table(1)?;
    let schema = table.schema();
    println!(
        "{} rows, {} columns",
        schema.row_count(),
        schema.columns.len()
    );
    for column in &schema.columns {
        println!("  {:?}: {}", column.name()?, column.tform);
    }

    // Each read fills a buffer the caller keeps and reuses from column to column.
    let mut values = ColumnData::U8(Vec::new());
    table.column(0)?.raw_into(&mut values)?;
    println!("ID     {values:?}");
    let name = table.column("NAME")?;
    name.raw_into(&mut values)?;
    if let ColumnData::Character(bytes) = &values {
        let names: Vec<String> = bytes
            .chunks(name.descriptor().tform.repeat())
            .map(|field| String::from_utf8_lossy(field).trim_end().to_owned())
            .collect();
        println!("NAME   {names:?}");
    }
    // `physical_into` applies TSCALn and TZEROn, maps TNULLn to NaN, and appends.
    let mut physical: Vec<f64> = Vec::new();
    table.column("MAG")?.physical_into(&mut physical)?;
    println!("MAG    {physical:?}");
    physical.clear();
    table.column("POS")?.physical_into(&mut physical)?;
    println!("POS    {physical:?}");
    let mut fluxes = Ragged::<Vec<f64>>::new(Vec::new(), Vec::new());
    table.column("FLUXES")?.vla_physical_into(&mut fluxes)?;
    println!("FLUXES {:?}", fluxes.iter().collect::<Vec<_>>());
    let flags = table.column("FLAGS")?.bits()?;
    println!(
        "FLAGS  {:?}",
        flags.iter().map(ToString::to_string).collect::<Vec<_>>()
    );

    // A large table reads in parts: rows 1 and 2, two columns of row 2, one cell.
    let rows = reader.read_table_rows(1, 1..3)?;
    physical.clear();
    rows.column("MAG")?.physical_into(&mut physical)?;
    println!("rows 1..3, MAG {physical:?}");
    let selection = reader.read_table_columns(1, 2..3, ["ID", "MAG"])?;
    for column in &selection.columns {
        println!("row 2, {:?}: {:?}", column.descriptor.name()?, column.data);
    }
    println!("cell (0, ID): {:?}", reader.read_table_cell(1, 0, "ID")?);
    Ok(())
}
