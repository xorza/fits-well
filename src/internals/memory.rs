//! In-memory writers and readers, for every test and bench that writes a file.

use std::io::Cursor;
use std::ops::{Deref, DerefMut};

use crate::fits_error::{FitsError, Result};
use crate::fits_writer::FitsWriter;
use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiWriteColumn};
use crate::fits_writer::builder::{Builder, WriteColumn};
#[cfg(feature = "compression")]
use crate::header_model::Header;
use crate::image_model::Image;
use crate::image_model::region::Region;
use crate::reader::source::Source;
use crate::reader::{FitsReader, SliceReader, StreamReader};

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

/// A file written into memory: its bytes, and the readers a test opens over them, as
/// `Fixture`'s are opened. It derefs to its bytes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Written(Vec<u8>);

impl Written {
    pub(crate) fn bytes(&self) -> &[u8] {
        &self.0
    }

    pub(crate) fn into_bytes(self) -> Vec<u8> {
        self.0
    }

    /// A reader over the bytes in memory.
    pub(crate) fn slice(&self) -> SliceReader<'_> {
        FitsReader::from_bytes(&self.0).unwrap()
    }

    /// A streaming reader over the bytes, as a caller with a `File` reads it.
    pub(crate) fn stream(&self) -> StreamReader<Cursor<&[u8]>> {
        FitsReader::open(Cursor::new(self.0.as_slice())).unwrap()
    }
}

impl Deref for Written {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.0
    }
}

impl DerefMut for Written {
    fn deref_mut(&mut self) -> &mut [u8] {
        &mut self.0
    }
}

/// The file `write` produces in [`memory_writer`].
pub(crate) fn written(write: impl FnOnce(&mut MemoryWriter) -> Result<()>) -> Written {
    written_by(memory_writer(), write)
}

/// The file `write` produces in `writer`, which sets the form (checksums, spare records).
pub(crate) fn written_by(
    mut writer: MemoryWriter,
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> Written {
    write(&mut writer).unwrap();
    Written(writer.into_inner().into_inner())
}

/// A file of an empty primary and one HDU of `header` over the data unit `data`, as it is
/// given: a header no builder makes, such as one with a defect.
#[cfg(feature = "compression")]
pub(crate) fn raw_hdu_file(header: &Header, data: &[u8]) -> Written {
    written(|w| {
        w.write_empty_primary(None)?;
        w.write_raw_hdu(header, data)
    })
}

/// The error `write` fails with, after asserting that it wrote nothing and left the
/// writer as it found it, so the next write makes the primary.
pub(crate) fn refused_before_output(
    write: impl FnOnce(&mut MemoryWriter) -> Result<()>,
) -> FitsError {
    let mut writer = memory_writer();
    let error = write(&mut writer).expect_err("the write must be refused");
    assert!(
        writer.is_empty(),
        "a refused write must leave the writer empty"
    );
    assert!(
        writer.into_inner().into_inner().is_empty(),
        "a refused write must leave no output"
    );
    error
}

/// HDU `index` read whole into an owned image.
pub(crate) fn read_whole<S: Source>(reader: &mut FitsReader<S>, index: usize) -> Image {
    reader.image(index).unwrap().read(Region::All).unwrap()
}
