# fits-well

[![crates.io](https://img.shields.io/crates/v/fits-well.svg)](https://crates.io/crates/fits-well)
[![docs.rs](https://img.shields.io/docsrs/fits-well)](https://docs.rs/fits-well)
[![license](https://img.shields.io/crates/l/fits-well.svg)](#license)

A blazing-fast Rust reader and writer for **FITS** (Flexible Image Transport
System) files — the standard data format of astronomy — targeting the full
**FITS 4.0** standard.

Two goals shape every decision:

1. **Fast** — zero-copy where the format allows, lazy seeking HDU access, reused
   scratch buffers, and tile-parallel (de)compression across a rayon pool.
2. **Whole-standard coverage** — images, ASCII tables, binary tables with a heap
   and variable-length arrays, random groups (read), a typed WCS layer (23
   projections), time coordinates, and tiled compression.

## Install

```toml
[dependencies]
fits-well = "0.1"
```

The default build pulls in tiled compression (`flate2`) and tile parallelism
(`rayon`). For the dependency-light pure-Rust core, use
`default-features = false` (see [Feature flags](#feature-flags)).

## Usage

### Inspect a file

`open` scans every HDU boundary from the headers alone — no pixel data is read.

```rust
use std::fs::File;
use fits_well::FitsReader;

let reader = FitsReader::open(File::open("image.fits")?)?;
println!("{} HDU(s)", reader.hdus().len());

for (i, hdu) in reader.hdus().iter().enumerate() {
    println!("HDU {i}: {:?}", hdu.kind);
    if let Some(object) = hdu.header.get_text("OBJECT") {
        println!("  OBJECT = {object}");
    }
}
# Ok::<(), fits_well::FitsError>(())
```

### Write and read an image

`shape` is fastest-axis-first (`NAXIS1` first); `samples` is the flat buffer.

```rust
use std::fs::File;
use fits_well::{FitsReader, FitsWriter, Image, ImageData, Scaling};

let image = Image {
    shape: vec![4, 3],
    samples: ImageData::I16(vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23]),
    scaling: Scaling { bscale: 1.0, bzero: 0.0, blank: None },
};

let mut writer = FitsWriter::new(File::create("out.fits")?);
writer.write_image(&image)?;
writer.into_inner().sync_all()?;

let mut reader = FitsReader::open(File::open("out.fits")?)?;
// `image_indices` lists the image-bearing HDUs, so you pick one rather than
// hard-coding it. `read_image` borrows the data unit in place (zero-copy).
let raw = reader.read_image(reader.image_indices()[0])?;
// `bitpix` is the stored width; `sample_type()` is the *effective* type, resolving
// the unsigned / signed-byte BZERO conventions (cfitsio's "equivalent type").
println!("shape {:?}, {:?}", raw.shape, raw.sample_type());

// `decode()` byte-swaps into an owned host-endian buffer; `physical()` applies
// BSCALE/BZERO and maps any BLANK value to NaN.
if let ImageData::I16(pixels) = raw.decode() {
    println!("pixels {pixels:?}");
}
# Ok::<(), fits_well::FitsError>(())
```

A **tile-compressed** image (FITS §10) reads back through the *same* `read_image`
call — it detects `ZIMAGE` and decompresses transparently. To write one:

```rust
# use std::fs::File;
# use fits_well::{FitsWriter, CompressOptions, Image, ImageData, Scaling};
# let image = Image { shape: vec![16, 16], samples: ImageData::I16(vec![0; 256]), scaling: Scaling { bscale: 1.0, bzero: 0.0, blank: None } };
let options = CompressOptions::tiled([8, 8]); // 8×8 tiles
let mut writer = FitsWriter::new(File::create("compressed.fits")?);
writer.write_compressed_image(&image, "RICE_1", &options)?;
# writer.into_inner().sync_all()?;
# Ok::<(), fits_well::FitsError>(())
```

### Binary tables

Address a column by index or by its `TTYPEn` name; the handle decodes on demand.

```rust
use std::fs::File;
use fits_well::{ColumnData, FitsReader, FitsWriter, WriteColumn};

let columns = [
    WriteColumn::fixed("ID", ColumnData::I32(vec![1, 2, 3]), 1),
    WriteColumn::fixed("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13]), 1).with_unit("mag"),
];

let mut writer = FitsWriter::new(File::create("table.fits")?);
writer.write_table(3, &columns)?; // 3 rows
writer.into_inner().sync_all()?;

let mut reader = FitsReader::open(File::open("table.fits")?)?;
let table = reader.read_table(1)?; // the table is HDU 1 (HDU 0 is the empty primary)
println!("{} rows, {} columns", table.nrows, table.columns.len());

// `.raw()` is the stored, typed plane; `.physical()` applies TZEROn/TSCALn and
// maps TNULLn to NaN, widening to f64. `.unsigned()`, `.complex()`, `.bits()`,
// and `.vla()` cover the other column kinds the same way.
println!("ID  = {:?}", table.column_by_idx(0)?.raw()?);
println!("MAG = {:?}", table.column_by_name("MAG")?.physical()?);
# Ok::<(), fits_well::FitsError>(())
```

### World Coordinate System

`Header::wcs` parses the `CTYPEn`/`CRPIXn`/`CRVALn`/… keywords into a transform
that converts between pixel and sky coordinates in the file's declared frame.

```rust
use std::fs::File;
use fits_well::FitsReader;

let reader = FitsReader::open(File::open("wcs_tan.fits")?)?;
let wcs = reader.hdus()[0].header.wcs(None)?; // None = the primary WCS

let sky = wcs.pixel_to_world(&[256.0, 256.0]); // RA/Dec at the reference pixel
let pixel = wcs.world_to_pixel(&sky);           // and back again
println!("{sky:?} -> {pixel:?}");
# Ok::<(), fits_well::FitsError>(())
```

The typed **time** layer (`Header::time`, `Datetime`, `TimeScale`) handles
ISO-8601/JD/MJD, epochs, and `UTC`…`TCB`/`GPS`/UT1 scale conversions.

## Examples

Runnable end-to-end programs live in [`examples/`](examples/):

```sh
cargo run --example image            # write + read an image
cargo run --example table            # binary-table columns
cargo run --example inspect -- f.fits  # describe a file's HDUs and headers
cargo run --example wcs              # pixel ↔ sky coordinates
cargo run --example time             # observation times and scale conversions
cargo run --example compression      # tile-compressed image round-trip
cargo run --example ndarray --features ndarray   # read an image as an n-D array
```

## Feature flags

| Feature | Default | What it adds |
|---------|:-------:|--------------|
| `compression` | ✅ | Tiled image + table (de)compression — `GZIP_1/2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` (pulls in `flate2`). |
| `parallel` | ✅ | Tile-parallel (de)compression across a rayon pool (implies `compression`). |
| `mmap` | — | `FitsReader::open_mmap` — zero-copy reads straight off memory-mapped pages (`memmap2`). |
| `ndarray` | — | `RawImage`/`Image` → typed `ImageArray` or a physical `ArrayD<f64>` in FITS axis order. |

`--no-default-features` gives the pure-Rust core (block / header / HDU / reader /
writer / WCS / time); its only unconditional dependencies are `bitvec` (packed
`X` bit-array columns) and `num-complex` (`C`/`M` complex columns). WCS (§8) and
time (§9) are dependency-free pure math and are always compiled.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
