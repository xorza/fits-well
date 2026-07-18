# fits-well

[![crates.io](https://img.shields.io/crates/v/fits-well.svg)](https://crates.io/crates/fits-well)
[![docs.rs](https://img.shields.io/docsrs/fits-well)](https://docs.rs/fits-well)
[![license](https://img.shields.io/crates/l/fits-well.svg)](https://github.com/xorza/fits#license)

A blazing-fast Rust reader and writer for **FITS** (Flexible Image Transport
System) files — the standard data format of astronomy — targeting the full
**FITS 4.0** standard.

Two goals shape every decision:

1. **Fast** — zero-copy where the format allows, lazy seeking HDU access, reused
   scratch buffers, and tile-parallel (de)compression across a rayon pool.
2. **Whole-standard coverage** — images, ASCII tables, binary tables with a heap
   and variable-length arrays, random groups (read), a typed WCS layer (27
   projections), time coordinates, and tiled compression.

## Install

```toml
[dependencies]
fits-well = "0.1"
```

The default build pulls in tiled compression (`flate2`) and tile parallelism
(`rayon`). For the dependency-light pure-Rust core, use
`default-features = false` (see the Feature flags section below).

## Usage

### Inspect a file

`open` scans every HDU boundary from the headers alone — no pixel data is read.
Use `hdus()` to inspect metadata. Data operations take an HDU index directly;
`hdu_index("EXTNAME", extver)` resolves named extensions without changing reader
state.

```rust,no_run
use std::fs::File;
use fits_well::FitsReader;

let reader = FitsReader::open(File::open("image.fits")?)?;
println!("{} HDU(s)", reader.hdus().len());

for (i, hdu) in reader.hdus().iter().enumerate() {
    println!("HDU {i}: {:?}", hdu.kind);
    // Missing keywords return None; present values of the wrong type return an error.
    if let Some(object) = hdu.header.get_text("OBJECT")? {
        println!("  OBJECT = {object}");
    }
}

let primary = &reader.hdus()[0];
println!("primary kind: {:?}", primary.kind);
# Ok::<(), fits_well::FitsError>(())
```

### Write and read an image

`shape` is fastest-axis-first (`NAXIS1` first); `samples` is the flat buffer.

```rust,no_run
use std::fs::File;
use fits_well::image::{Image, ImageData};
use fits_well::{FitsReader, FitsWriter};

let image = Image::new(
    vec![4, 3],
    vec![0i16, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23],
)?;

let mut writer = FitsWriter::new(File::create("out.fits")?);
writer.write_image(&image)?;
writer.into_inner().sync_all()?;

let mut reader = FitsReader::open(File::open("out.fits")?)?;
// `image_indices` lists the image-bearing HDUs, so you pick one rather than
// hard-coding it. `read_image` borrows a slice/mmap source in place, or a
// seekable source's reused staging buffer.
let image_index = reader.image_indices()[0];
let raw = reader.read_image(image_index)?;
// `bitpix` is the stored width; `sample_type()` is the *effective* type, resolving
// the unsigned / signed-byte BZERO conventions (cfitsio's "equivalent type").
println!("shape {:?}, {:?}", raw.metadata().shape, raw.sample_type());

// `decode()` byte-swaps into an owned host-endian buffer; `physical()` applies
// BSCALE/BZERO and maps any BLANK value to NaN.
if let ImageData::I16(pixels) = raw.decode() {
    println!("pixels {pixels:?}");
}
# Ok::<(), fits_well::FitsError>(())
```

Use `Image::new_scaled` only when the stored and physical planes differ.
`Scaling::IDENTITY` and `Scaling::default()` name the ordinary identity case.

Rectangular sections are zero-based, half-open, and fastest-axis-first. Plain
images coalesce contiguous source reads; compressed images read and decompress
only intersecting tiles:

```rust,no_run
# use std::fs::File;
# use fits_well::FitsReader;
# let mut reader = FitsReader::open(File::open("cube.fits")?)?;
let sci = reader
    .hdu_index("SCI", None)?
    .expect("SCI extension");
let cutout = reader.read_image_section(sci, &[10..110, 20..120, 4..5])?;
println!("cutout shape: {:?}", cutout.metadata().shape);
# Ok::<(), fits_well::FitsError>(())
```

When science metadata belongs with typed data, use
`write_image_with_header`, `write_table_with_header`,
`write_ascii_table_with_header`, or `write_compressed_image_with_header`.
The writer preserves ordered informational cards such as `OBJECT`, `EXTNAME`,
WCS/time keywords, `COMMENT`, and `HISTORY`, while regenerating structural and
checksum cards from the typed payload.

A **tile-compressed** image (FITS §10) reads back through the *same* `read_image`
call — it detects `ZIMAGE` and decompresses transparently. To write one:

```rust,no_run
# #[cfg(feature = "compression")]
# {
# use std::fs::File;
# use fits_well::image::{Compression, CompressionOptions, Image};
# use fits_well::FitsWriter;
# let image = Image::new(vec![16, 16], vec![0i16; 256])?;
let options = CompressionOptions::tiled([8, 8]); // 8×8 tiles
let mut writer = FitsWriter::new(File::create("compressed.fits")?);
writer.write_compressed_image(&image, Compression::Rice, &options)?;
# writer.into_inner().sync_all()?;
# }
# Ok::<(), fits_well::FitsError>(())
```

### Binary tables

Address a column by index or by its `TTYPEn` name; the handle decodes on demand.

```rust,no_run
use std::fs::File;
use fits_well::table::{ColumnData, TableBuilder, WriteColumn};
use fits_well::{FitsReader, FitsWriter};

let table = TableBuilder::new()
    .column(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
    .column(
        WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13]))
            .with_unit("mag"),
    )?;

let mut writer = FitsWriter::new(File::create("table.fits")?);
writer.write_table(&table)?; // row count inferred and cross-checked
writer.into_inner().sync_all()?;

let mut reader = FitsReader::open(File::open("table.fits")?)?;
let table = reader.read_table(1)?; // the table is HDU 1 (HDU 0 is the empty primary)
let metadata = table.metadata();
println!("{} rows, {} columns", metadata.nrows, metadata.columns.len());

// `.raw()` is the stored, typed plane; `.physical()` applies TZEROn/TSCALn and
// maps TNULLn to NaN, widening to f64. `.unsigned()`, `.complex()`, and `.bits()`
// cover fixed special kinds; `.vla()` plus `.vla_physical()`, `.vla_unsigned()`,
// `.vla_complex()`, and `.vla_bits()` cover jagged P/Q heap arrays.
println!("ID  = {:?}", table.column_by_idx(0)?.raw()?);
println!("MAG = {:?}", table.column_by_name("MAG")?.physical()?);
# Ok::<(), fits_well::FitsError>(())
```

`TableBuilder` infers row count and scalar type. `WriteColumn::fixed` remains the
explicit-schema path for vector cells, while `WriteColumn::vla` infers a
nonempty heap type; use `vla_typed` only for an empty/predeclared VLA. The
parallel `AsciiTableBuilder`/`AsciiWriteColumn` API lives under
`fits_well::table`.

For large tables, discover `hdu.table_schema()` without reading data, then use
`read_table_rows`, `read_table_columns`, or `read_table_cell`. Ranged reads fetch
only selected rows and referenced P/Q heap cells; `read_table()` remains the
explicit whole-table materialization path.

Jagged bit arrays use `WriteColumn::vla_bits` with one MSB-first
`BitVec<u8, Msb0>` per row; call `.wide()?` when `QX` descriptors are required.

### World Coordinate System

`FitsReader::read_wcs` parses the `CTYPEn`/`CRPIXn`/`CRVALn`/… keywords into a
transform and resolves any `-TAB` coordinate arrays from their referenced
`BINTABLE`. `Header::wcs` is the header-only form for descriptions that do not
need external table data.

```rust,no_run
use std::fs::File;
use fits_well::FitsReader;

let mut reader = FitsReader::open(File::open("wcs_tan.fits")?)?;
let wcs = reader.read_wcs(0, None)?; // None = the primary WCS

let sky = wcs.pixel_to_world(&[256.0, 256.0])?; // RA/Dec at the reference pixel
let pixel = wcs.world_to_pixel(&sky)?; // and back again
let declared_frame = wcs.view().celestial_frame; // RADESYS/EQUINOX metadata
println!("{sky:?} -> {pixel:?}");
# Ok::<(), fits_well::FitsError>(())
```

Both complete transforms return an error for coordinates outside the projection's
domain, failed iterative inversion, or a nonlinear algorithm this crate does not
yet implement.

The typed **time** layer (`Header::time`, `time::Datetime`, `time::TimeScale`)
handles strict FITS ISO-8601/JD/MJD, FITS time units, epochs, resolved
`TREFPOS`/`TRPOSn`, all image/table PHASE keyword forms, and PC/CD-coupled time
axes through the parsed WCS model. It preserves recognized scale realizations
and arbitrary local scale names. Inter-scale chronometry needs external leap
tables or ephemerides and deliberately remains outside this format library.

## API organization and large-file behavior

`FitsReader`, `FitsWriter`, `FitsError`, and `Result` are at the crate root.
Detailed types have one canonical home under `image`, `table`, `header`, `wcs`,
`time`, or `io`. The `table` module exposes the concrete bit-vector and complex
types used by its public signatures.

Whole-HDU writer methods are transactional: they preflight and stage one data
unit in the writer scratch before touching the sink, so peak memory is
proportional to the largest HDU written. For large seekable image output,
`FitsWriter::stream_image`, `stream_image_scaled`, and `stream_image_with_header`
encode typed chunks directly, then validate the sample count, pad, and patch
checksums at `finish`. Dropping an unfinished stream poisons the writer. Tables
currently use the transactional whole-HDU path.

Readers expose `into_inner` for seekable sources and `into_bytes` for borrowed
sources. FITS files are read lazily and written sequentially; in-place file
editing is deliberately out of scope.

## Capability matrix

| Area | Support |
| --- | --- |
| Normative FITS 4.0 §§3–9 | Complete typed core: HDUs, images, ASCII/binary tables including P/Q, random groups read, WCS, and time |
| Normative tiled compression §10 | Image and fixed/P/Q table read/write; five image codecs plus `NOCOMPRESS`; default `compression` feature |
| Registered conventions | `CONTINUE` and `CHECKSUM`/`DATASUM`; `HIERARCH` is preserved as opaque commentary; convention-only `XPH` is readable but not transformed |
| Partial I/O | N-D image sections including compressed-tile selection; binary-table schema, cells, columns, rows, and ranges |
| Streaming I/O | Lazy reader; scratch-reusing image views; incremental seekable image writer |
| In-place editing | Out of scope |
| Ecosystem interop | Optional `mmap`; typed vectors and shape metadata support downstream array adapters without coupling the core to an array crate |

## Examples

Runnable end-to-end programs live in [`examples/`](examples/):

```sh
cargo run --example image            # write + read an image
cargo run --example table            # binary-table columns
cargo run --example inspect -- f.fits  # describe a file's HDUs and headers
cargo run --example wcs              # pixel ↔ sky coordinates
cargo run --example time             # observation time metadata
cargo run --example compression      # tile-compressed image round-trip
```

## Feature flags

| Feature | Default | What it adds |
|---------|:-------:|--------------|
| `compression` | ✅ | Tiled image + table (de)compression — `GZIP_1/2`, `RICE_1`, `PLIO_1`, signed-64-bit `HCOMPRESS_1`, `NOCOMPRESS`, float quantization, null masks, and P/Q table VLAs (pulls in `flate2`). |
| `parallel` | ✅ | Tile-parallel (de)compression across a rayon pool (implies `compression`). |
| `mmap` | — | `FitsReader::open_mmap` — zero-copy reads straight off memory-mapped pages (`memmap2`). |
`--no-default-features` gives the pure-Rust core (block / header / HDU / reader /
writer / WCS / time); its only unconditional dependencies are `bitvec` (packed
`X` bit-array columns) and `num-complex` (`C`/`M` complex columns). WCS (§8) and
time (§9) are dependency-free pure math and are always compiled.

## License

Licensed under either of

- Apache License, Version 2.0 ([LICENSE-APACHE](LICENSE-APACHE))
- MIT license ([LICENSE-MIT](LICENSE-MIT))

at your option.
