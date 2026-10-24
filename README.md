# fits-well

[![crates.io](https://img.shields.io/crates/v/fits-well.svg)](https://crates.io/crates/fits-well)
[![docs.rs](https://img.shields.io/docsrs/fits-well)](https://docs.rs/fits-well)
[![license](https://img.shields.io/crates/l/fits-well.svg)](https://github.com/xorza/fits-well#license)

A reader and writer for **FITS**, the standard file format of astronomy, in pure Rust.

## Highlights

- **Pure Rust.** No C library, no system install and no build script, so
  `cargo build` and cross-compilation work on every platform. Without default
  features, it depends only on `bitvec`, `num-complex` and `thiserror`.
- **The whole FITS 4.0 standard.** Images, ASCII and binary tables with
  variable-length arrays, random groups (read), world coordinates, time coordinates,
  tiled compression of images and tables, and checksums.
- **World coordinates without wcslib.** All 27 projections, the spectral algorithms,
  `-TAB` coordinate arrays, and the SIP and TPV distortions, in both directions. A WCS
  writes back into a header, and follows a crop or a bin of its image.
- **Compression that matches fpack.** All five codecs read and write, for images and
  binary tables, in parallel over tiles. The float quantization gives fpack's
  `ZSCALE` bit for bit, and lossless floats come back bit for bit.
- **Fast.** `open` reads only the headers. A read decodes into a buffer that you
  reuse, and a section read of a compressed image decodes only the tiles it touches.
- **Files larger than memory, and files in memory.** Read one section of an image or
  some rows of a table, and write an image chunk by chunk. `FitsReader::from_bytes`
  reads bytes you already hold, with no copy, and a writer writes into any `Write`,
  a `Vec<u8>` too.
- **Safe on bad files.** See [Safety](#safety).

## Compared with other Rust libraries

| | fits-well | rust-fitsio | fitsio-pure | fitsrs |
|---|:---:|:---:|:---:|:---:|
| Pure Rust | ✅ | ❌ binds cfitsio | ✅ | ✅ |
| `no_std` / `wasm32` | ❌ | ❌ | ✅ | ❌ |
| Image read and write | ✅ | ✅ | ✅ | 🟡 read |
| `BSCALE` / `BZERO`, unsigned, `BLANK` | ✅ | 🟡 no nulls | 🟡 | ❌ |
| Binary tables with variable-length arrays | ✅ | 🟡 no arrays | 🟡 arrays read | 🟡 raw read |
| ASCII tables | ✅ | ✅ | 🟡 | 🟡 raw |
| Random groups | 🟡 read | ❌ | 🟡 header | ❌ |
| Compressed images | ✅ | 🟡 through cfitsio | 🟡 2 codecs | 🟡 read |
| Compressed tables | ✅ | ❌ | ❌ | ❌ |
| Lossless float compression | ✅ | 🟡 through cfitsio | ❓ | ❌ |
| `CHECKSUM` / `DATASUM` | ✅ | ❌ | 🟡 | ❌ |
| `HIERARCH` keywords | ✅ | ❌ | 🟡 | 🟡 read |
| Edit a header in place, append an HDU | ✅ | 🟡 | 🟡 | ❌ |
| World coordinates | ✅ | ❌ | ❌ | 🟡 2-D |
| SIP / TPV distortion | ✅ | ❌ | ❌ | 🟡 SIP |
| Time coordinates (§9) | ✅ | ❌ | ❌ | ❌ |
| `.fits.gz` and stream input | ❌ | ✅ | 🟡 | ✅ |

✅ supported, 🟡 partial, ❌ not supported, ❓ not known. From the source code of
rust-fitsio 0.21, fitsio-pure 0.22 and fitsrs 0.4, October 2026.

Some items are not supported on purpose:

- **`.fits.gz` and stream input.** Inflate the file, and read the bytes with
  `FitsReader::from_bytes`.
- **Time-scale conversion.** It needs leap-second and ephemeris tables from outside the
  file.

## Safety

- **A bad file gives an error, not a panic.** Each size, count and offset from a file
  is checked before use: `NAXIS` up to 999, data sizes in 64 bits, heap arrays inside
  the heap, and an inflated tile of exactly its size.
- **A near-valid header is repaired and reported.** A lowercase keyword or a misplaced
  `=` reads as cfitsio and astropy read it, and `Header::repairs` lists each repair. A
  record with a bad value is kept, and only a read that needs it fails.
- **A write refuses what FITS cannot hold.** Examples are a NaN in a header value,
  non-ASCII text, and a record that would read back as a different record. After a
  failed write, the writer refuses all later writes, so it cannot add to a broken file.
- **An edit cannot break a file.** A header is replaced in place only when the new one
  fits, and `CHECKSUM` stays valid. Otherwise the edited file goes to a second file.
- **Checksums are verified during the read**, and an absent checksum is not reported
  as valid.
- **Errors are typed.** Each `FitsError` variant is one case, and you can match on it.
- **`unsafe` is in one module only:** three slice casts of the sample buffer, for six
  primitive number types behind a sealed trait.

## Supported types

- **Image samples:** `u8`, `i16`, `i32`, `i64`, `f32` and `f64`, and the unsigned
  `u16`, `u32`, `u64` and signed `i8` through `BZERO`.
- **Header values:** logical, integer (exact past 64 bits), real, complex and text,
  with long text on `CONTINUE` records.
- **Table columns:** every `TFORMn` type, `L`, `X`, `B`, `I`, `J`, `K`, `E`, `D`,
  `C`, `M` and `A`, fixed-width or as variable-length arrays, and unsigned integers
  through `TZEROn`.

## Testing

- **Real files.** The codec tests read files that fpack, funpack and astropy wrote,
  and check each sample bit for bit.
- **Reference values.** The WCS tests compare with values from wcslib and astropy, and
  with values worked by hand, in both directions.
- **Round trips.** Each written structure reads back as it was written.
- **Each feature set.** The crate builds with no features, with `compression`, and
  with all features, and the tests pass with the last two.

## Install

```sh
cargo add fits-well
```

| Feature | Default | Adds |
| --- | :---: | --- |
| `compression` | ✅ | Tiled compression: `GZIP_1`, `GZIP_2`, `RICE_1`, `PLIO_1`, `HCOMPRESS_1` and `NOCOMPRESS` (`flate2`) |
| `parallel` | ✅ | Tile-parallel codecs on rayon; turns on `compression` |

Rust 1.89 or later.

## Examples

### Write and read an image

```rust no_run
use std::fs::File;
use fits_well::image::{Data, Image, Scaling};
use fits_well::{FitsError, FitsReader, FitsWriter};

fn main() -> Result<(), FitsError> {
    // The shape is NAXIS1 first: 4 columns, 3 rows.
    let image = Image::new(vec![4, 3], (0..12).collect::<Vec<i16>>(), Scaling::IDENTITY)?;
    let mut writer = FitsWriter::new(File::create("out.fits")?);
    writer.write_image(image.view(), None)?;
    writer.into_inner().sync_all()?;

    let mut reader = FitsReader::open(File::open("out.fits")?)?;
    println!("{:?}", reader.hdus()[0].header.get_text("OBJECT")?); // None: not set
    let mut image = reader.image(0)?;
    println!("{:?}", image.metadata().shape);

    // The stored values...
    if let Data::I16(pixels) = image.read(..)?.into_samples() {
        println!("{pixels:?}");
    }
    // ...or the physical values, with BSCALE and BZERO applied and BLANK as NaN.
    let mut physical: Vec<f32> = Vec::new();
    image.physical_into(.., &mut physical)?;
    Ok(())
}
```

### Compress an image

```rust no_run
use std::fs::File;
use fits_well::compress::{Compression, CompressionConfig};
use fits_well::image::{Image, Scaling};
use fits_well::{FitsError, FitsReader, FitsWriter};

fn main() -> Result<(), FitsError> {
    let image = Image::new(vec![256, 256], vec![0i16; 256 * 256], Scaling::IDENTITY)?;
    let config = CompressionConfig::new(Compression::Rice).with_tiles([256, 16]);
    let mut writer = FitsWriter::new(File::create("compressed.fits")?);
    writer.write_compressed_image(image.view(), &config, None)?;
    writer.into_inner().sync_all()?;

    // A compressed image reads as a plain one does.
    let mut reader = FitsReader::open(File::open("compressed.fits")?)?;
    let first = reader.images().next().expect("the file holds an image").index;
    let pixels = reader.image(first)?.read(..)?;
    println!("{:?}", pixels.shape());
    Ok(())
}
```

### Write and read a table

```rust no_run
use std::fs::File;
use fits_well::table::{Builder, ColumnData, WriteColumn};
use fits_well::{FitsError, FitsReader, FitsWriter};

fn main() -> Result<(), FitsError> {
    let mut table = Builder::new();
    table
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))?
        .push(WriteColumn::scalar("MAG", ColumnData::F64(vec![0.03, -1.46, 0.13])).with_unit("mag"))?;
    let mut writer = FitsWriter::new(File::create("table.fits")?);
    writer.write_table(&table, None)?;
    writer.into_inner().sync_all()?;

    let mut reader = FitsReader::open(File::open("table.fits")?)?;
    let table = reader.read_table(1)?; // HDU 0 is the empty primary
    println!("{:?}", table.column("MAG")?.physical()?);
    Ok(())
}
```

### Convert pixels to sky coordinates

```rust no_run
use std::fs::File;
use fits_well::{FitsError, FitsReader};

fn main() -> Result<(), FitsError> {
    let mut reader = FitsReader::open(File::open("wcs_tan.fits")?)?;
    let wcs = reader.read_wcs(0, None)?;
    let mut sky = [0.0; 2];
    wcs.pixel_to_world(&[256.0, 256.0], &mut sky)?;
    println!("RA {} deg, Dec {} deg", sky[0], sky[1]);
    Ok(())
}
```

More programs are in [`examples/`](examples/), and the API is on
[docs.rs](https://docs.rs/fits-well). To see the HDUs and the main keywords of a file:

```sh
cargo run --example inspect -- path/to/file.fits
```

## License

Licensed under either of [Apache-2.0](LICENSE-APACHE) or [MIT](LICENSE-MIT), at
your option.
