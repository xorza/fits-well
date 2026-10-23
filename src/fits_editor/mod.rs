//! [`FitsEditor`]: edits a FITS file that exists.

use std::io::{Read, Seek, SeekFrom, Write};

use crate::block::{BLOCK_SIZE, CARD_SIZE, padded_len};
use crate::checksum;
use crate::error::FitsError;
use crate::error::Result;
use crate::fits_writer::{FitsWriter, prepare_header_with_data_sum, render_header};
use crate::hdu::{HduPosition, HduRole, data_extent};
use crate::header_model::card::is_end_record;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::reader::hdu::Hdu;
use crate::reader::hdu_selector::HduSelector;
use crate::reader::{FitsReader, selected_hdu};

/// Edits a FITS file that exists, beside [`FitsReader`] and [`FitsWriter`]: it replaces
/// a header in place where the new one fits the old one's blocks, writes the whole
/// edited file to a second sink where it does not, and appends HDUs. It scans the
/// headers as `FitsReader::open` does, and parses each header it writes in place.
#[derive(Debug)]
pub struct FitsEditor<F> {
    file: F,
    hdus: Vec<Hdu>,
}

/// Where an HDU lies in the file, copied out of its [`Hdu`] so the file can be read and
/// written while it is used.
#[derive(Debug, Clone, Copy)]
struct Placement {
    index: usize,
    header_offset: u64,
    header_blocks: usize,
    data_offset: u64,
    /// The data unit's length with its padding.
    data_len: u64,
}

impl<F: Read + Write + Seek> FitsEditor<F> {
    /// Scan the headers of `file`, reading no data.
    pub fn open(mut file: F) -> Result<Self> {
        let hdus = scan(&mut file)?;
        Ok(Self { file, hdus })
    }

    /// The scanned HDU records, in file order.
    pub fn hdus(&self) -> &[Hdu] {
        &self.hdus
    }

    /// Give back the file.
    pub fn into_inner(self) -> F {
        self.file
    }

    /// Replace the header of `hdu` in place with `header`, which must give the same data
    /// unit (`DataSizeMismatch` otherwise). The free records before `END` are blank, so
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
        let place = self.placement(hdu.into(), header)?;
        let edited = EditedHeader {
            header,
            data_sum: self.data_sum(place, header)?,
        };
        let mut out = Vec::new();
        edited.render_filling(place.header_blocks, &mut out)?;
        self.file.seek(SeekFrom::Start(place.header_offset))?;
        self.file.write_all(&out)?;
        self.file.flush()?;
        self.hdus[place.index] = Hdu::parse(
            place.index,
            &out,
            checksum::accumulate(&out, 0),
            place.header_offset,
        )?;
        Ok(())
    }

    /// Write the whole file to `sink` with the header of `hdu` replaced by `header`,
    /// which may take more blocks: each other HDU, and any bytes after the last, are
    /// copied as they are. The caller renames the result over the file, as astropy does
    /// in update mode; this file is left as it was. The checksum rule is
    /// [`FitsEditor::replace_header`]'s.
    pub fn replace_header_into<'h>(
        &mut self,
        hdu: impl Into<HduSelector<'h>>,
        header: &Header,
        mut sink: impl Write,
    ) -> Result<()> {
        let target = self.placement(hdu.into(), header)?;
        let edited = EditedHeader {
            header,
            data_sum: self.data_sum(target, header)?,
        };
        let mut out = Vec::new();
        edited.render(0, &mut out)?;
        let places: Vec<Placement> = self.hdus.iter().map(placement_of).collect::<Result<_>>()?;
        let mut end = 0;
        for place in places {
            if place.index == target.index {
                sink.write_all(&out)?;
                copy_range(&mut self.file, place.data_offset, place.data_len, &mut sink)?;
            } else {
                let len = place.data_offset + place.data_len - place.header_offset;
                copy_range(&mut self.file, place.header_offset, len, &mut sink)?;
            }
            end = place.data_offset + place.data_len;
        }
        let file_len = self.file.seek(SeekFrom::End(0))?;
        copy_range(&mut self.file, end, file_len - end, &mut sink)?;
        sink.flush()?;
        Ok(())
    }

    /// A [`FitsWriter`] that appends HDUs at the end of the file. Errors with
    /// [`FitsError::FileOffBlockGrid`] when the file does not end on a block boundary. It
    /// consumes the editor, so two handles never write one file.
    pub fn append(mut self) -> Result<FitsWriter<F>> {
        let len = self.file.seek(SeekFrom::End(0))?;
        if len % BLOCK_SIZE as u64 != 0 {
            return Err(FitsError::FileOffBlockGrid { len });
        }
        Ok(FitsWriter::continuing(self.file))
    }

    /// The placement of the HDU `selector` names, after checking that `header` gives
    /// the same data unit.
    fn placement(&self, selector: HduSelector<'_>, header: &Header) -> Result<Placement> {
        let hdu = selected_hdu(&self.hdus, selector)?;
        let position = if hdu.index == 0 {
            HduPosition::Primary
        } else {
            HduPosition::Extension
        };
        let extent = data_extent(header, HduRole::from_header(header, position)?)?;
        if extent.data_bytes != hdu.data_bytes {
            return Err(FitsError::DataSizeMismatch {
                expected: usize::try_from(hdu.data_bytes)
                    .map_err(|_| FitsError::DataUnitOverflow)?,
                got: usize::try_from(extent.data_bytes).map_err(|_| FitsError::DataUnitOverflow)?,
            });
        }
        placement_of(hdu)
    }

    /// The data sum the new header's `CHECKSUM` is computed from: its `DATASUM`, else the
    /// sum of the data unit, also for a blank `DATASUM`, which the standard reserves for
    /// an unknown sum (§J); `None` when it holds no `CHECKSUM`.
    fn data_sum(&mut self, place: Placement, header: &Header) -> Result<Option<u32>> {
        if !header.contains(keywords::CHECKSUM)? {
            return Ok(None);
        }
        let declared = match header.get(keywords::DATASUM)? {
            Some(Value::Text(text)) if !text.trim().is_empty() => Some(
                text.trim()
                    .parse::<u32>()
                    .map_err(|_| FitsError::KeywordOutOfRange {
                        keyword: keywords::DATASUM,
                    })?,
            ),
            Some(Value::Text(_)) | None => None,
            Some(_) => {
                return Err(FitsError::KeywordOutOfRange {
                    keyword: keywords::DATASUM,
                });
            }
        };
        if let Some(sum) = declared {
            return Ok(Some(sum));
        }
        let mut data = Vec::new();
        copy_range(&mut self.file, place.data_offset, place.data_len, &mut data)?;
        Ok(Some(checksum::accumulate(&data, 0)))
    }
}

/// The records of a header unit.
const RECORDS_PER_BLOCK: usize = BLOCK_SIZE / CARD_SIZE;

/// A header the editor writes, with the data sum its `CHECKSUM` comes from; `None`
/// when it holds no `CHECKSUM`.
#[derive(Debug, Clone, Copy)]
struct EditedHeader<'h> {
    header: &'h Header,
    data_sum: Option<u32>,
}

impl EditedHeader<'_> {
    /// Render the header into `out` with `spare` blank records before `END`.
    fn render(self, spare: usize, out: &mut Vec<u8>) -> Result<()> {
        match self.data_sum {
            Some(sum) => prepare_header_with_data_sum(self.header.clone(), sum, spare, out),
            None => render_header(self.header, spare, out),
        }
    }

    /// Render the header into `out` to fill exactly `blocks` blocks, with blank records
    /// before `END`; `HeaderDoesNotFit` when it needs more.
    fn render_filling(self, blocks: usize, out: &mut Vec<u8>) -> Result<()> {
        self.render(0, out)?;
        let needed = out.len() / BLOCK_SIZE;
        if needed > blocks {
            return Err(FitsError::HeaderDoesNotFit {
                needed,
                available: blocks,
            });
        }
        let records = out
            .as_chunks::<CARD_SIZE>()
            .0
            .iter()
            .position(|record| is_end_record(record))
            .expect("a rendered header ends with END");
        self.render(blocks * RECORDS_PER_BLOCK - records - 1, out)?;
        debug_assert_eq!(out.len(), blocks * BLOCK_SIZE);
        Ok(())
    }
}

fn placement_of(hdu: &Hdu) -> Result<Placement> {
    Ok(Placement {
        index: hdu.index,
        header_offset: hdu.header_offset,
        header_blocks: usize::try_from(hdu.header_blocks())
            .map_err(|_| FitsError::DataUnitOverflow)?,
        data_offset: hdu.data_offset,
        data_len: padded_len(hdu.data_bytes).ok_or(FitsError::DataUnitOverflow)?,
    })
}

/// The HDUs of `file`, its headers scanned as `FitsReader::open` does.
fn scan<F: Read + Seek>(file: &mut F) -> Result<Vec<Hdu>> {
    Ok(FitsReader::open(file)?.into_hdus())
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
