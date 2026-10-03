# fits-well

[![crates.io](https://img.shields.io/crates/v/fits-well.svg)](https://crates.io/crates/fits-well)
[![docs.rs](https://img.shields.io/docsrs/fits-well)](https://docs.rs/fits-well)
[![license](https://img.shields.io/crates/l/fits-well.svg)](https://github.com/xorza/fits-well#license)

A fast Rust reader and writer for **FITS**, the standard file format of astronomy,
covering the whole **FITS 4.0** standard.

- **Fast.** Lazy HDU access from the headers alone, zero-copy reads where the
  format allows, decode into a reused caller-owned buffer, and tile-parallel
  (de)compression.
- **Complete.** Images, ASCII and binary tables with variable-length arrays,
  random groups (read), WCS with all 27 projections and `-TAB`, time
  coordinates, tiled compression of images and tables, and `CHECKSUM`/`DATASUM`
  (write and verify).
- **Safe on hostile input.** Sizes, counts and offsets from a file are checked
  before use, so a malformed file is an error rather than a panic.

## Install

```toml
[dependencies]
fits-well = "0.3"
```

| Feature | Default | Adds |
| --- | :---: | --- |
| `compression` | ✅ | Tiled compression: `GZIP_1`/`GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1`, `NOCOMPRESS`, float quantization (`flate2`) |
| `parallel` | ✅ | Tile-parallel codecs on rayon; implies `compression` |
| `mmap` | | `FitsReader::open_mmap`: reads straight from mapped pages (`memmap2`) |

With `default-features = false`, the only dependencies are `bitvec`,
`num-complex` and `thiserror`. Rust 1.89 or later.

## Usage

### Images

```rust no_run
use std::fs::File;
use fits_well::image::{Image, ImageData};
use fits_well::{FitsReader, FitsWriter};

fn main() -> Result<(), fits_well::FitsError> {
    let image = Image::new(vec![4, 3], (0..12).collect::<Vec<i16>>())?; // NAXIS1 first
    let mut writer = FitsWriter::new(File::create("out.fits")?);
    writer.write_image(&image, None)?;
    writer.into_inner().sync_all()?;

    // `open` reads only the headers. A tile-compressed image reads the same way.
    let mut reader = FitsReader::open(File::open("out.fits")?)?;
    let image = reader.read_image(reader.image_indices()[0])?;
    println!("{:?} {:?}", image.metadata().shape, image.sample_type());
    let physical = image.physical(); // BSCALE/BZERO applied, BLANK as NaN
    if let ImageData::I16(pixels) = image.decode() {
        println!("{pixels:?}"); // stored values, host-endian
    }
    Ok(())
}
```

`read_image_view` decodes into a buffer you reuse across reads, so a loop over
many images allocates once. `read_image_section` reads a sub-region, and
`stream_image` writes a large image chunk by chunk.

### Binary tables

```rust no_run
use std::fs::File;
use fits_well::table::{ColumnData, TableBuilder, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

fn main() -> Result<(), fits_well::FitsError> {
    let table = TableBuilder::new()
        .column(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
        .column(
            WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13]))
                .with_unit("mag"),
        )?;
    let mut writer = FitsWriter::new(File::create("table.fits")?);
    writer.write_table(&table, None)?;
    writer.into_inner().sync_all()?;

    let mut reader = FitsReader::open(File::open("table.fits")?)?;
    let table = reader.read_table(1)?; // HDU 0 is the empty primary
    println!("{:?}", table.column_by_name("MAG")?.physical()?); // TSCALn/TZEROn applied
    Ok(())
}
```

`read_table_rows`, `read_table_columns` and `read_table_cell` read only part of
a large table.

### World coordinates and time

```rust no_run
use std::fs::File;
use fits_well::FitsReader;

fn main() -> Result<(), fits_well::FitsError> {
    let mut reader = FitsReader::open(File::open("wcs_tan.fits")?)?;
    let wcs = reader.read_wcs(0, None)?; // resolves `-TAB` arrays from their tables
    let sky = wcs.pixel_to_world(&[256.0, 256.0])?;
    let pixel = wcs.world_to_pixel(&sky)?;
    Ok(())
}
```

`time::FitsTime::from_header` reads the time keywords. More programs are in
[`examples/`](examples/).

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
