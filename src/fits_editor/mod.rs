//! [`FitsEditor`]: edits a FITS file that exists.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::block::RECORDS_PER_BLOCK;
use crate::checksum;
use crate::checksum::declared::Declared;
use crate::fits_error::FitsError;
use crate::fits_error::Result;
use crate::fits_error::SizeUnit;
use crate::fits_writer::FitsWriter;
use crate::hdu::hdu_kind::{HduKind, HduPosition};
use crate::hdu::hdu_selector::HduSelector;
use crate::hdu::hdus::Hdus;
use crate::hdu::{Hdu, HeaderUnit};
use crate::header_model::{Header, Signing, Space};
use crate::reader::data_checksum::DataChecksum;
use crate::reader::data_source::DataSource;
use crate::reader::source::StreamSource;

#[cfg(feature = "internals")]
pub(crate) mod bench;

/// Edits a FITS file that exists, beside [`FitsReader`](crate::FitsReader) and [`FitsWriter`]: it replaces
/// a header in place where the new one fits the old one's blocks, writes the whole
/// edited file to a second sink where it does not, and appends HDUs. It scans the
/// headers as `FitsReader::open` does, and parses each header it writes in place.
#[derive(Debug)]
pub struct FitsEditor<F> {
    file: F,
    hdus: Hdus,
}

impl<F: Read + Write + Seek> FitsEditor<F> {
    /// Scan the headers of `file`, reading no data.
    pub fn open(mut file: F) -> Result<Self> {
        let hdus = Hdus::scan(&mut StreamSource::new(&mut file)?)?;
        Ok(Self { file, hdus })
    }

    /// The scanned HDU records, in file order.
    pub fn hdus(&self) -> &[Hdu] {
        self.hdus.as_slice()
    }

    /// The HDU `hdu` names, as [`FitsReader::hdu`](crate::FitsReader::hdu) finds it.
    pub fn hdu<'h>(&self, hdu: impl Into<HduSelector<'h>>) -> Result<&Hdu> {
        self.hdus.select(hdu.into())
    }

    /// Every HDU that holds an image, as [`FitsReader::images`](crate::FitsReader::images)
    /// lists them.
    pub fn images(&self) -> impl Iterator<Item = &Hdu> + '_ {
        self.hdus.images()
    }

    /// Give back the file.
    pub fn into_inner(self) -> F {
        self.file
    }

    /// Replace the header of `hdu` in place with `header`, which must give the same data
    /// unit (`DataSizeMismatch` otherwise) of the same kind (`HduKindChange` otherwise),
    /// as a `TABLE` and a `BINTABLE` fill their last block differently. The free records before `END` are blank, so
    /// the header unit keeps its blocks and no other byte of the file moves. A header
    /// that needs more blocks is [`FitsError::HeaderDoesNotFit`]: write it with
    /// [`FitsEditor::replace_header_into`], or leave room with
    /// [`FitsWriter::with_spare_records`].
    ///
    /// A header that holds `CHECKSUM` gets a valid one: from its `DATASUM` when it holds
    /// one, which is trusted as it was before the edit, else from the data unit, read
    /// once.
    pub fn replace_header<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        header: &Header,
    ) -> Result<()> {
        let target = self.hdus.select(hdu.into())?;
        check_unit(target, header)?;
        let edited = EditedHeader {
            header,
            data_sum: data_sum(&mut self.file, target, header)?,
        };
        let mut out = Vec::new();
        edited.render(Space::Blocks(target.header_blocks()), &mut out)?;
        // Parsed before it is written, so a header that does not parse leaves the file
        // and the HDU list as they were.
        let unit = HeaderUnit {
            bytes: &out,
            sum: checksum::accumulate(&out, 0),
            offset: target.header_offset,
        };
        let hdu = Hdu::parse(target.index, unit)?;
        self.file.seek(SeekFrom::Start(hdu.header_offset))?;
        self.file.write_all(&out)?;
        self.file.flush()?;
        self.hdus.replace(hdu);
        Ok(())
    }

    /// Write the whole file to `sink` with the header of `hdu` replaced by `header`,
    /// which may take more blocks: each other HDU, and any bytes after the last, are
    /// copied as they are. The new header keeps the free records the old one had in its
    /// blocks, as space for a later edit in place. The caller renames the result over the file, as astropy does
    /// in update mode; this file is left as it was. The checksum rule is
    /// [`FitsEditor::replace_header`]'s.
    pub fn replace_header_into<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        header: &Header,
        mut sink: impl Write,
    ) -> Result<()> {
        let target = self.hdus.select(hdu.into())?;
        check_unit(target, header)?;
        let edited = EditedHeader {
            header,
            data_sum: data_sum(&mut self.file, target, header)?,
        };
        // A header that a parse repaired can render to more records than its blocks
        // held, and then has no free record to keep.
        let free = (target.header_blocks() * RECORDS_PER_BLOCK - 1)
            .saturating_sub(target.header.rendered_records()?);
        let mut out = Vec::new();
        edited.render(Space::Spare(free), &mut out)?;
        let mut end = 0;
        for hdu in &self.hdus {
            let data_end = hdu.data_offset + hdu.padded_data_bytes();
            if hdu.index == target.index {
                sink.write_all(&out)?;
                copy_range(
                    &mut self.file,
                    hdu.data_offset,
                    hdu.padded_data_bytes(),
                    &mut sink,
                )?;
            } else {
                let len = data_end - hdu.header_offset;
                copy_range(&mut self.file, hdu.header_offset, len, &mut sink)?;
            }
            end = data_end;
        }
        let file_len = self.file.seek(SeekFrom::End(0))?;
        copy_range(&mut self.file, end, file_len - end, &mut sink)?;
        sink.flush()?;
        Ok(())
    }

    /// A [`FitsWriter`] that appends HDUs at the end of the file, which must be the end
    /// of its last HDU. Errors with [`FitsError::TrailingContent`] when bytes follow it,
    /// which a reader stops at, and with [`FitsError::UnexpectedEof`] when the file ends
    /// inside it; cfitsio writes over such bytes instead. It consumes the editor, so two
    /// handles never write one file.
    pub fn append(mut self) -> Result<FitsWriter<F>> {
        let file_len = self.file.seek(SeekFrom::End(0))?;
        let last = self.hdus.last();
        let hdu_end = last.data_offset + last.padded_data_bytes();
        if file_len > hdu_end {
            return Err(FitsError::TrailingContent { hdu_end, file_len });
        }
        if file_len < hdu_end {
            return Err(FitsError::UnexpectedEof);
        }
        Ok(FitsWriter::continuing(self.file))
    }
}

/// Check that `header` gives the data unit of `hdu`, of the same kind.
fn check_unit(hdu: &Hdu, header: &Header) -> Result<()> {
    let kind = HduKind::from_header(header, HduPosition::of_index(hdu.index))?;
    if kind != hdu.kind {
        return Err(FitsError::HduKindChange {
            from: hdu.kind,
            to: kind,
        });
    }
    let data_bytes = kind.data_bytes(header)?;
    if data_bytes != hdu.data_bytes {
        return Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: hdu.data_bytes,
            got: data_bytes,
        });
    }
    Ok(())
}

/// The data sum the new header of `hdu` in `file` computes its `CHECKSUM` from: its
/// `DATASUM` when it declares a valid one, else the sum of the data unit, read in bounded
/// chunks; `None` when it holds no `CHECKSUM`.
fn data_sum<F: Read + Seek>(file: &mut F, hdu: &Hdu, header: &Header) -> Result<Option<u32>> {
    let declared = Declared::read(header);
    if !declared.has_checksum() {
        return Ok(None);
    }
    if let Some(sum) = declared.data_sum() {
        return Ok(Some(sum));
    }
    let mut data = DataSource::new(StreamSource::new(file)?);
    DataChecksum::of(hdu).sum(&mut data).map(Some)
}

/// A header the editor writes, with the data sum its `CHECKSUM` comes from; `None`
/// when it holds no `CHECKSUM`.
#[derive(Debug, Clone, Copy)]
struct EditedHeader<'h> {
    header: &'h Header,
    data_sum: Option<u32>,
}

impl EditedHeader<'_> {
    /// Render the header into `out` with the blank records `space` asks before `END`.
    fn render(self, space: Space, out: &mut Vec<u8>) -> Result<()> {
        let signing = self
            .data_sum
            .map_or(Signing::Unsigned, Signing::WithDataSum);
        self.header.render_into(signing, space, out)
    }
}

/// Copy `len` bytes of `file` from `start` to `sink`. A file shorter than that is
/// `UnexpectedEof`.
fn copy_range<F: Read + Seek>(
    file: &mut F,
    start: u64,
    len: u64,
    sink: &mut impl Write,
) -> Result<()> {
    file.seek(SeekFrom::Start(start))?;
    let copied = std::io::copy(&mut file.by_ref().take(len), sink)?;
    if copied != len {
        return Err(FitsError::UnexpectedEof);
    }
    Ok(())
}

#[cfg(test)]
mod tests;
