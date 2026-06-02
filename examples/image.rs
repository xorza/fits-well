//! Create images, write them to a FITS file, and read them back two ways — an owned
//! decode you can keep, and the borrowed `read_image_view` hot path:
//!
//! ```sh
//! cargo run --example image
//! ```

use std::fs::File;

use fits_well::{FitsReader, FitsWriter, Image, ImageData, ImageView, Scaling};

/// Identity scaling: physical value = stored, no blanks — the common case.
const IDENTITY: Scaling = Scaling {
    bscale: 1.0,
    bzero: 0.0,
    blank: None,
};

fn main() -> fits_well::Result<()> {
    let path = std::env::temp_dir().join("fits_well_image.fits");

    // A 4×3 image of signed 16-bit pixels. `shape` is fastest-axis-first
    // (NAXIS1 = 4), and `samples` is the flat row-major buffer.
    let i16_image = Image {
        shape: vec![4, 3],
        #[rustfmt::skip]
        samples: ImageData::I16(vec![
             0,  1,  2,  3,
            10, 11, 12, 13,
            20, 21, 22, 23,
        ]),
        scaling: IDENTITY,
    };
    // A second image of a *different* type (32-bit float) — so the file holds two
    // image HDUs of differing BITPIX, which the view loop below reads into one buffer.
    let f32_image = Image {
        shape: vec![2, 2],
        samples: ImageData::F32(vec![1.5, -2.5, 3.5, -4.5]),
        scaling: IDENTITY,
    };

    // Writing synthesizes the mandatory header (SIMPLE/XTENSION, BITPIX, NAXISn) and
    // the big-endian data unit. The first `write_image` is the primary array; the
    // second becomes an `IMAGE` extension.
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(&i16_image)?;
    writer.write_image(&f32_image)?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    // `image_indices` lists the HDUs that hold images, so you pick indices rather than
    // hard-coding them — here the primary plus the extension, `[0, 1]`.
    let mut reader = FitsReader::open(File::open(&path)?)?;
    let images = reader.image_indices();
    println!("image HDUs: {images:?}");

    // --- Owned path: decode samples you want to keep. ---
    // `read_image` borrows the data unit in place (zero-copy) as a `RawImage`; `decode`
    // byte-swaps the big-endian samples into an *owned* host-endian buffer you can keep
    // and move (a BITPIX=8 image's bytes come back copy-free via `raw.u8()`).
    let raw = reader.read_image(images[0])?;
    println!("hdu 0: shape {:?}, {:?}", raw.shape, raw.bitpix);
    if let ImageData::I16(pixels) = raw.decode() {
        println!("  pixels   {pixels:?}");
    }
    // `physical()` applies BSCALE/BZERO and maps any BLANK value to NaN.
    println!("  physical {:?}", raw.physical());

    // --- Borrowed view path: a hot loop that processes each image and moves on. ---
    // `read_image_view` byte-swaps into a *caller-owned* scratch instead of allocating
    // a fresh buffer per call — so a loop over many images pays the output allocation
    // once and reuses it across reads, even across differing BITPIX (i16 then f32
    // here). The reader retains nothing; you own `scratch` and drop it when done.
    let mut scratch: Vec<u64> = Vec::new();
    for &idx in &images {
        // The view borrows the reader + scratch, so use it before the next read. For
        // samples you need past the loop, use the owned `read_image().decode()` above.
        match reader.read_image_view(idx, &mut scratch)? {
            ImageView::I16(v) => println!("hdu {idx}: i16 view {v:?}"),
            ImageView::F32(v) => println!("hdu {idx}: f32 view {v:?}"),
            other => println!(
                "hdu {idx}: {:?} view, {} samples",
                other.bitpix(),
                other.len()
            ),
        }
    }

    Ok(())
}
