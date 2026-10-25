use crate::ascii::AsciiColumnData;
use crate::ascii::ascii_text::AsciiText;
use crate::ascii::ascii_tform::AsciiFloatFormat;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tform::Storage;
use crate::block::padded_len;
use crate::block::{BLOCK_SIZE, CARD_SIZE};
use crate::checksum::checksum_report::ChecksumStatus;
use crate::column::MAX_TABLE_FIELDS;
#[cfg(feature = "compression")]
use crate::compression::{Compression, CompressionConfig, Hcompress};
#[cfg(feature = "compression")]
use crate::fits_error::CompressionFeature;
use crate::fits_error::TextOrigin;
use crate::fits_error::{FitsError, SizeUnit};
use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiWriteColumn};
use crate::fits_writer::builder::{Builder, LaidOut, WriteColumn};
use crate::fits_writer::image_writer::ImageWriter;
use crate::fits_writer::{CHUNK_SAMPLES, FitsWriter, WriterState};
use crate::hdu::hdu_kind::HduKind;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::record::RecordView;
use crate::header_model::value::FitsInteger;
use crate::header_model::value::Value;
use crate::header_model::{Header, keywords};
use crate::image_model::U64_OFFSET;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::View;
use crate::image_model::{Image, Metadata};
use crate::internals::cards;
use crate::internals::cards::VALID;
use crate::internals::memory;
use crate::internals::samples;
use crate::internals::samples::{BITPIXES, NONFINITE};
use crate::internals::whole_column::{WholeAsciiColumn, WholeColumn, WholeView};
use crate::ragged::Ragged;
use crate::reader::FitsReader;
use bitvec::bitvec;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use num_complex::Complex;
use std::assert_matches;
use std::io;
use std::io::Write;
use std::io::{Cursor, Seek, SeekFrom};

mod checksums;
mod image;
mod stream;
mod table;
mod template;

#[derive(Debug, Default)]
struct FailMidHdu {
    bytes: Vec<u8>,
    failed: bool,
}

impl Write for FailMidHdu {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.bytes.is_empty() {
            let written = buf.len().min(127);
            self.bytes.extend_from_slice(&buf[..written]);
            return Ok(written);
        }
        if !self.failed {
            self.failed = true;
            return Err(io::Error::other("injected write failure"));
        }
        self.bytes.extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn informational_header_template() -> Header {
    let mut header = cards::header_of(&[
        ("SIMPLE", false.into()),
        ("XTENSION", "TABLE".into()),
        ("BITPIX", 64.into()),
        ("NAXIS", 3.into()),
        ("NAXIS1", 999.into()),
        ("NAXIS2", 999.into()),
        ("NAXIS3", 999.into()),
        ("PCOUNT", 999.into()),
        ("GCOUNT", 999.into()),
        ("TFIELDS", 999.into()),
        ("TFORM1", "99D".into()),
        ("TTYPE1", "STALE".into()),
        ("TUNIT1", "stale".into()),
        ("TDIM1", "(99)".into()),
        ("TSCAL1", 99.0.into()),
        ("TZERO1", 99.0.into()),
        ("TNULL1", 99.into()),
        ("TBCOL1", 99.into()),
        ("NAXIS01", "preserved lookalike".into()),
        ("TFORM01", "preserved lookalike".into()),
        ("ZFORM01", "preserved lookalike".into()),
        ("BSCALE", 99.0.into()),
        ("BZERO", 99.0.into()),
        ("BLANK", 99.into()),
        ("CHECKSUM", "stale".into()),
        ("DATASUM", "999".into()),
        ("ZIMAGE", false.into()),
        ("ZCMPTYPE", "STALE".into()),
        ("ZBITPIX", 64.into()),
        ("ZNAXIS", 3.into()),
        ("ZNAXIS1", 999.into()),
        ("ZTILE1", 999.into()),
        ("ZSIMPLE", false.into()),
        ("OBJECT", "M42".into()),
        ("EXTNAME", "SCI".into()),
        ("CTYPE1", "RA---TAN".into()),
        ("CUNIT1", "deg".into()),
        ("CRPIX1", 1.0.into()),
        ("CRVAL1", 83.822.into()),
        ("CDELT1", 0.1.into()),
        ("TIMESYS", "UTC".into()),
        ("MJDREF", 60_000.0.into()),
    ]);
    header
        .push_commentary(CommentaryName::COMMENT, "first preserved comment")
        .unwrap();
    header
        .push_commentary(CommentaryName::HISTORY, "preserved history")
        .unwrap();
    header
        .push_commentary(CommentaryName::COMMENT, "second preserved comment")
        .unwrap();
    header
}

fn assert_informational_header(header: &Header) {
    cards::assert_cards(
        header,
        &[
            ("OBJECT", "M42".into()),
            ("EXTNAME", "SCI".into()),
            ("CTYPE1", "RA---TAN".into()),
            ("CUNIT1", "deg".into()),
            ("CRPIX1", 1.0.into()),
            ("CRVAL1", 83.822.into()),
            ("CDELT1", 0.1.into()),
            ("TIMESYS", "UTC".into()),
            ("MJDREF", 60_000.0.into()),
            ("NAXIS01", "preserved lookalike".into()),
            ("TFORM01", "preserved lookalike".into()),
            ("ZFORM01", "preserved lookalike".into()),
        ],
    );
    let commentary: Vec<_> = header
        .iter()
        .filter_map(|record| match record {
            RecordView::Commentary { name, text }
                if matches!(name, CommentaryName::COMMENT | CommentaryName::HISTORY) =>
            {
                Some((name, text.unwrap_or("")))
            }
            _ => None,
        })
        .collect();
    assert_eq!(
        commentary,
        [
            (CommentaryName::COMMENT, "first preserved comment"),
            (CommentaryName::HISTORY, "preserved history"),
            (CommentaryName::COMMENT, "second preserved comment"),
        ]
    );
}

/// One sample of each binary-table stored type, with its `TFORMn` code.
fn every_column_kind() -> [(char, ColumnData); 11] {
    [
        ('L', ColumnData::Logical(vec![Some(true)])),
        ('B', ColumnData::U8(vec![1])),
        ('I', ColumnData::I16(vec![1])),
        ('J', ColumnData::I32(vec![1])),
        ('K', ColumnData::I64(vec![1])),
        ('E', ColumnData::F32(vec![1.0])),
        ('D', ColumnData::F64(vec![1.0])),
        ('C', ColumnData::ComplexF32(vec![Complex::new(1.0, 2.0)])),
        ('M', ColumnData::ComplexF64(vec![Complex::new(1.0, 2.0)])),
        ('A', ColumnData::Character(b"a".to_vec())),
        ('X', ColumnData::Bit(bitvec![u8, Msb0; 1])),
    ]
}

/// A one-row column of `data`: fixed, then variable-length.
fn fixed_and_vla(data: ColumnData) -> [WriteColumn; 2] {
    [
        WriteColumn::fixed("FIXED", data.clone(), 1),
        WriteColumn::vla(
            "VLA",
            Ragged::<ColumnData>::new(data, vec![1]),
            DescriptorWidth::P,
        ),
    ]
}

/// One stored sample of an integer `bitpix`.
fn integer_column_data(bitpix: Bitpix) -> ColumnData {
    match samples::ramp(bitpix, 1) {
        Data::U8(values) => ColumnData::U8(values),
        Data::I16(values) => ColumnData::I16(values),
        Data::I32(values) => ColumnData::I32(values),
        Data::I64(values) => ColumnData::I64(values),
        Data::F32(_) | Data::F64(_) => unreachable!("{bitpix:?} is not an integer type"),
    }
}

/// The header a one-column table writes, read back.
fn written_table_header(column: WriteColumn) -> Header {
    let file = memory::written(|w| w.write_table(&memory::binary_table(1, &[column]), None));
    let reader = file.stream();
    reader.hdus()[1].header.clone()
}

fn assert_table_column_refused(column: WriteColumn, expected: &'static str) {
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(1, &[column]), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == expected
    );
}

fn empty_binary_columns(count: usize) -> Vec<WriteColumn> {
    (1..=count)
        .map(|n| WriteColumn::fixed(format!("C{n}"), ColumnData::U8(Vec::new()), 1))
        .collect()
}

fn empty_ascii_columns(count: usize) -> Vec<AsciiWriteColumn> {
    (1..=count)
        .map(|n| AsciiWriteColumn::text(format!("C{n}"), AsciiText::default(), 1))
        .collect()
}

#[test]
fn writer_refuses_invalid_or_overflowing_layouts() {
    // usize::MAX two-byte elements per row overflow the row width.
    let fixed = WriteColumn::fixed("X", ColumnData::I16(Vec::new()), usize::MAX);
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(0, &[fixed]), None)),
        FitsError::DataUnitOverflow
    );

    let ascii = |name: &str, width| AsciiWriteColumn::text(name, AsciiText::default(), width);
    assert_matches!(
        memory::refused_before_output(|w| w.write_ascii_table(
            &memory::ascii_table(0, &[ascii("A", usize::MAX), ascii("B", 1)],),
            None
        )),
        FitsError::DataUnitOverflow
    );

    let invalid_vla_bits = WriteColumn::vla(
        "FLAGS",
        Ragged::from_rows([ColumnData::Bit(bitvec![u8, Msb0; 1, 0, 1])]).unwrap(),
        DescriptorWidth::P,
    )
    .with_tdim(vec![2, 2]);
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(1, &[invalid_vla_bits]), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TDIM1"
    );

    let invalid_tdim =
        WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4).with_tdim(vec![5]);
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(1, &[invalid_tdim]), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TDIM1"
    );

    // A shape whose product overflows `usize` is out-of-range for the same reason a
    // merely-too-large one is: it cannot describe the cell's element count. The
    // reader reports the overflow identically.
    let overflowing_tdim = WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4)
        .with_tdim(vec![usize::MAX, 2]);
    assert_matches!(
        memory::refused_before_output(|w| w.write_table(&memory::binary_table(1, &[overflowing_tdim]), None)),
        FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TDIM1"
    );

    for shape in [vec![], vec![0]] {
        let invalid_empty_vla = WriteColumn::vla(
            "VEC",
            Ragged::from_rows(vec![ColumnData::I32(vec![])]).unwrap(),
            DescriptorWidth::P,
        )
        .with_tdim(shape);
        assert_matches!(
            memory::refused_before_output(|w| w.write_table(&memory::binary_table(1, &[invalid_empty_vla]), None)),
            FitsError::InvalidKeywordValue { keyword, .. } if keyword == "TDIM1"
        );
    }

    // "Vég" is four bytes, so it fills the field and reaches the ASCII check.
    let invalid_text =
        WriteColumn::fixed("NAME", ColumnData::Character("Vég".as_bytes().to_vec()), 4);
    assert_matches!(
        memory::refused_before_output(
            |w| w.write_table(&memory::binary_table(1, &[invalid_text]), None)
        ),
        FitsError::InvalidAscii {
            text: TextOrigin::Cell
        }
    );

    #[cfg(target_pointer_width = "64")]
    {
        assert_matches!(
            memory::refused_before_output(
                |w| w.write_table(&memory::binary_table(usize::MAX, &[]), None)
            ),
            FitsError::DataUnitOverflow
        );
    }
}

#[test]
fn partial_hdu_failure_refuses_every_later_write() {
    let image = Image::new(vec![1], Data::U8(vec![7]), Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(FailMidHdu::default());
    assert_matches!(
        writer.write_image(image.view(), None),
        Err(FitsError::Io(_))
    );
    assert_eq!(writer.state, WriterState::Failed);
    assert_eq!(writer.sink.bytes.len(), 127);

    assert_matches!(
        writer.write_image(image.view(), None),
        Err(FitsError::WriterFailed)
    );
    assert_eq!(writer.into_inner().bytes.len(), 127);
}

#[test]
fn raw_hdu_validates_the_complete_unit_before_output() {
    let header = cards::from_card_lines(&[
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    1",
        "NAXIS1  =                    2",
    ]);
    let mut writer = memory::memory_writer();
    assert_matches!(
        writer.write_raw_hdu(&header, &[0, 1]),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 4,
            got: 2
        })
    );
    assert_eq!(writer.state, WriterState::Empty);
    assert!(writer.into_inner().into_inner().is_empty());
}

/// A sink whose write number `fail_at` fails, and every one after it succeeds.
#[derive(Debug)]
struct FailingSink {
    inner: Cursor<Vec<u8>>,
    writes: usize,
    fail_at: usize,
}

impl Write for FailingSink {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.writes += 1;
        if self.writes == self.fail_at {
            return Err(io::Error::other("the sink fails once"));
        }
        self.inner.write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner.flush()
    }
}

impl Seek for FailingSink {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}
