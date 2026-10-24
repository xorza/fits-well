//! Write an image larger than the buffer that holds it, chunk by chunk, and read it back
//! section by section, with the data checksum summed during the read:
//!
//! ```sh
//! cargo run --example stream
//! ```

#![expect(clippy::print_stdout, reason = "an example reports to the terminal")]

use std::env;
use std::fs::File;

use fits_well::image::{Bitpix, Metadata, Scaling, View};
use fits_well::{FitsReader, FitsWriter};

const WIDTH: usize = 2048;
const HEIGHT: usize = 2048;
const ROWS_PER_CHUNK: usize = 64;

fn main() -> fits_well::Result<()> {
    let path = env::temp_dir().join("fits_well_stream.fits");

    // The header goes first and the samples follow in order, so one chunk of rows is in
    // memory at a time. A stream seeks back once at the end, to write its checksums.
    let layout = Metadata {
        shape: &[WIDTH, HEIGHT],
        bitpix: Bitpix::F32,
        scaling: Scaling::IDENTITY,
    };
    let mut writer = FitsWriter::new(File::create(&path)?).with_checksums();
    let mut stream = writer.stream_image(layout, None)?;
    let mut chunk = vec![0.0f32; WIDTH * ROWS_PER_CHUNK];
    for top in (0..HEIGHT).step_by(ROWS_PER_CHUNK) {
        for (i, sample) in chunk.iter_mut().enumerate() {
            *sample = (top + i / WIDTH) as f32;
        }
        stream.write_chunk(View::from(chunk.as_slice()))?;
    }
    stream.finish()?;
    writer.into_inner().sync_all()?;
    println!("wrote {}", path.display());

    let mut reader = FitsReader::open(File::open(&path)?)?;
    let mut image = reader.image(0)?.with_checksums();
    let mut words: Vec<u64> = Vec::new();
    let mut sum = 0.0f64;
    for top in (0..HEIGHT).step_by(ROWS_PER_CHUNK) {
        let section = image.view(&[0..WIDTH, top..top + ROWS_PER_CHUNK], &mut words)?;
        if let View::F32(samples) = section.samples() {
            sum += samples.iter().map(|&sample| f64::from(sample)).sum::<f64>();
        }
    }
    // The verdict covers the whole data unit, so it comes after the last section.
    let report = image.finish()?;
    println!(
        "sum of all samples: {sum}, checksums valid: {}",
        report.is_valid()
    );
    Ok(())
}
