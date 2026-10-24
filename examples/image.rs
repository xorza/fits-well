//! Write two images, and read them back whole, by name, by section, and as physical
//! values:
//!
//! ```sh
//! cargo run --example image
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use fits_well::header::Header;
use fits_well::image::{Data, Image, Scaling, View};
use fits_well::io::HduSelector;
use fits_well::{FitsReader, FitsWriter};

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_image.fits");

    // The shape is NAXIS1 first: 4 columns and 3 rows, stored row after row.
    let counts = Image::new(
        vec![4, 3],
        Data::I16(vec![0, 1, 2, 3, 10, 11, 12, 13, 20, 21, 22, 23]),
        Scaling::IDENTITY,
    )?;
    let flux = Image::new(
        vec![2, 2],
        Data::F32(vec![1.5, -2.5, 3.5, -4.5]),
        Scaling::IDENTITY,
    )?;
    let mut flux_header = Header::new();
    flux_header.set("EXTNAME", "FLUX")?;

    // The first image is the primary array, the second an IMAGE extension.
    let mut writer = FitsWriter::new(File::create(&path)?);
    writer.write_image(counts.view(), None)?;
    writer.write_image(flux.view(), Some(&flux_header))?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    let mut reader = FitsReader::open(File::open(&path)?)?;
    let indices: Vec<usize> = reader.images().map(|hdu| hdu.index).collect();
    println!("image HDUs: {indices:?}");

    let mut image = reader.image(0)?;
    let metadata = image.metadata();
    println!("HDU 0: shape {:?}, {:?}", metadata.shape, metadata.bitpix);
    let whole = image.read(..)?;
    println!("  stored   {:?}", whole.samples());
    let mut physical: Vec<f64> = Vec::new();
    image.physical_into(.., &mut physical)?;
    println!("  physical {physical:?}");

    // One range per axis: columns 1 and 2 of rows 1 and 2.
    let section = image.read(&[1..3, 1..3])?;
    println!("  section  {:?}", section.samples());

    // `view` decodes into `words`, which the caller keeps, so a loop over many reads
    // allocates once.
    let mut words: Vec<u64> = Vec::new();
    for hdu in [HduSelector::from(0), HduSelector::from("FLUX")] {
        let mut image = reader.image(hdu)?;
        match image.view(.., &mut words)?.samples() {
            View::I16(samples) => println!("i16 view {samples:?}"),
            View::F32(samples) => println!("f32 view {samples:?}"),
            other => println!("{:?} view of {} samples", other.bitpix(), other.len()),
        }
    }
    Ok(())
}
