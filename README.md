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
  random groups (read), WCS with all 27 projections, `-TAB` and the SIP and TPV
  distortions, time coordinates, tiled compression of images and tables, and
  `CHECKSUM`/`DATASUM` (write and verify).
- **Safe on hostile input.** Sizes, counts and offsets from a file are checked
  before use, so a malformed file is an error rather than a panic.

## Install

```toml
[dependencies]
fits-well = "0.3"
```

| Feature | Default | Adds |
| --- | :---: | --- |
| `compression` | ✅ | Tiled compression: `GZIP_1`/`GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1`, `NOCOMPRESS`, float quantization or lossless floats (`flate2`) |
| `parallel` | ✅ | Tile-parallel codecs on rayon; implies `compression` |

With `default-features = false`, the only dependencies are `bitvec`,
`num-complex` and `thiserror`. Rust 1.89 or later.

## Usage

### Images

```rust no_run
use std::fs::File;
use fits_well::image::{Data, Image, Scaling};
use fits_well::{FitsError, FitsReader, FitsWriter};

fn main() -> Result<(), FitsError> {
    let image = Image::new(vec![4, 3], (0..12).collect::<Vec<i16>>(), Scaling::IDENTITY)?; // NAXIS1 first
    let mut writer = FitsWriter::new(File::create("out.fits")?);
    writer.write_image(image.view(), None)?;
    writer.into_inner().sync_all()?;

    // `open` reads only the headers. A tile-compressed image reads the same way.
    let mut reader = FitsReader::open(File::open("out.fits")?)?;
    let first = reader.images().next().ok_or(FitsError::NotAnImage)?.index;
    let mut image = reader.image(first)?;
    println!("{:?}", image.metadata().shape);
    let mut physical: Vec<f32> = Vec::new();
    image.physical_into(.., &mut physical)?; // BSCALE/BZERO applied, BLANK as NaN
    if let Data::I16(pixels) = image.read(..)?.into_samples() {
        println!("{pixels:?}"); // stored values, host-endian
    }
    Ok(())
}
```

An HDU is named by its index, by its `EXTNAME` (`"SCI"`), or by its name and
`EXTVER` (`("SCI", 2)`). `image` gives a reader of one HDU: `view` decodes a region into a buffer you
reuse across reads, so a loop allocates once, `read` copies a region into an
owned `Image`, and `physical_into` converts it in one pass. A region is the whole
image or one range per axis. `stream_image` writes a large image chunk by chunk.

`write_compressed_image` writes a tiled-compressed image with a `CompressionConfig`:
the codec, the tiles and, for a float image, its `Quantization`: `relative` or
`absolute` levels with a `Dither`, as fpack's `-q`, or `Quantization::NONE` for
lossless floats.

### Binary tables

```rust no_run
use std::fs::File;
use fits_well::table::{ColumnData, Builder, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

fn main() -> Result<(), fits_well::FitsError> {
    let mut table = Builder::new();
    table
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
        .push(
            WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13]))
                .with_unit("mag"),
        )?;
    let mut writer = FitsWriter::new(File::create("table.fits")?);
    writer.write_table(&table, None)?;
    writer.into_inner().sync_all()?;

    let mut reader = FitsReader::open(File::open("table.fits")?)?;
    let table = reader.read_table(1)?; // HDU 0 is the empty primary
    println!("{:?}", table.column("MAG")?.physical()?); // TSCALn/TZEROn applied
    Ok(())
}
```

`read_table_rows`, `read_table_columns` and `read_table_cell` read only part of
a large table.

### Editing a file

`FitsEditor` replaces a header in place when the new one fits the old one's blocks,
keeping `CHECKSUM` valid; `replace_header_into` writes the whole edited file to a
second sink when it does not, and `append` gives a writer at the end of the file.
`FitsWriter::with_spare_records` leaves room in each header for later edits.

### World coordinates and time

```rust no_run
use std::fs::File;
use fits_well::FitsReader;

fn main() -> Result<(), fits_well::FitsError> {
    let mut reader = FitsReader::open(File::open("wcs_tan.fits")?)?;
    let wcs = reader.read_wcs(0, None)?; // resolves `-TAB` arrays from their tables
    let mut sky = [0.0; 2];
    wcs.pixel_to_world(&[256.0, 256.0], &mut sky)?; // into the caller's buffer
    let mut pixel = [0.0; 2];
    wcs.world_to_pixel(&sky, &mut pixel)?;
    Ok(())
}
```

`time::Frame::from_header` reads the time keywords, and gives each time as a
`time::Mjd`: a whole day and a fraction of it, with a spacing of 1e-11 s. More
programs are in [`examples/`](examples/).

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
