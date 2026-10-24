//! In-memory writers and readers, for every test and bench that writes a file.

use std::io::Cursor;

use crate::error::{FitsError, Result};
use crate::fits_writer::FitsWriter;
use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiWriteColumn};
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::image_model::Image;
use crate::image_model::region::Region;
use crate::reader::source::Source;
use crate::reader::{FitsReader, StreamReader};

pub(crate) type MemoryWriter = FitsWriter<Cursor<Vec<u8>>>;

/// A writer into a new buffer, with the default form: no checksums, no spare records.
pub(crate) const fn memory_writer() -> MemoryWriter {
    FitsWriter::new(Cursor::new(Vec::new()))
}

pub(crate) fn binary_table(row_count: usize, columns: &[WriteColumn]) -> Builder {
    Builder::with_columns(row_count, columns.iter().cloned()).unwrap()
}

pub(crate) fn ascii_table(row_count: usize, columns: &[AsciiWriteColumn]) -> AsciiBuilder {
    AsciiBuilder::with_columns(row_count, columns.iter().cloned()).unwrap()
}

/// The file `write` produces in [`memory_writer`].
pub(crate) fn written(write: impl FnOnce(&mut MemoryWriter) -> Result<()>) -> Vec<u8> {
    written_by(memory_writer(), write)
}

/// The file `write` produces in `writer`, which sets the form (checksums, spare records).
pub(crate) fn written_by(
    mut writer: MemoryWriter,
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> Vec<u8> {
    write(&mut writer).unwrap();
    writer.into_inner().into_inner()
}

/// The error `write` fails with, after asserting that it wrote nothing.
pub(crate) fn rejected_before_output(
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> FitsError {
    let mut writer = memory_writer();
    let error = write(&mut writer).expect_err("the write must be refused");
    assert!(
        writer.into_inner().into_inner().is_empty(),
        "a refused write must leave no output"
    );
    error
}

/// A reader over the file `write` produces in [`memory_writer`].
pub(crate) fn round_trip(
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> StreamReader<Cursor<Vec<u8>>> {
    round_trip_by(memory_writer(), write)
}

/// A reader over the file `write` produces in `writer`.
pub(crate) fn round_trip_by(
    writer: MemoryWriter,
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> StreamReader<Cursor<Vec<u8>>> {
    FitsReader::open(Cursor::new(written_by(writer, write))).unwrap()
}

/// HDU `index` read whole into an owned image.
pub(crate) fn read_whole<S: Source>(reader: &mut FitsReader<S>, index: usize) -> Image {
    reader.image(index).unwrap().read(Region::All).unwrap()
}
