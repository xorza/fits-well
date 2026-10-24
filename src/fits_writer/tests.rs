use crate::ascii::AsciiColumnData;
use crate::ascii::ascii_text::AsciiText;
use crate::bintable::column_data::ColumnData;
use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::element_kind::ElementKind;
use crate::bintable::tform::Storage;
use crate::bitpix::Bitpix;
use crate::block::padded_len;
use crate::block::{BLOCK_SIZE, CARD_SIZE, SPACE_FILL};
use crate::checksum;
use crate::checksum::checksum_report::{ChecksumReport, ChecksumStatus};
use crate::column::MAX_TABLE_FIELDS;
#[cfg(feature = "compression")]
use crate::compress::{Compression, CompressionConfig, Hcompress};
use crate::error::TextOrigin;
use crate::error::{FitsError, SizeUnit};
use crate::fits_writer::ascii_builder::{AsciiBuilder, AsciiFloatFormat, AsciiWriteColumn};
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::fits_writer::{CHUNK_SAMPLES, FitsWriter, WriterState};
use crate::hdu::HduKind;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::record::RecordView;
use crate::header_model::value::Value;
use crate::header_model::{Header, Signing, Space, keywords};
use crate::image_model::data::Data;
use crate::image_model::internals::physical;
use crate::image_model::scaling::Scaling;
use crate::image_model::unsigned_data::UnsignedData;
use crate::image_model::view::{Borrowed, View};
use crate::image_model::{Image, Metadata};
use crate::image_model::{U16_OFFSET, U64_OFFSET};
use crate::internals::cards;
use crate::internals::cards::{
    VALID, assert_absent, assert_cards, card_index, find_card, from_card_lines as header, header_of,
};
use crate::internals::memory::{
    ascii_table, binary_table, memory_writer, read_whole, rejected_before_output, round_trip,
    round_trip_by, written, written_by,
};
use crate::internals::samples::{BITPIXES, NONFINITE, integer_range, ramp};
use crate::ragged::Ragged;
use crate::reader::FitsReader;
use bitvec::bitvec;
use bitvec::order::Msb0;
use bitvec::vec::BitVec;
use num_complex::Complex;
use std::io;
use std::io::Write;
use std::io::{Cursor, Seek, SeekFrom};

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
    let mut header = header_of(&[
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
    header.push_comment("first preserved comment").unwrap();
    header.push_history("preserved history").unwrap();
    header.push_comment("second preserved comment").unwrap();
    header
}

fn assert_informational_header(header: &Header) {
    assert_cards(
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

fn checksum_report_for_keywords(datasum: Option<&str>, checksum: Option<&str>) -> ChecksumReport {
    let mut header = cards::structural(None, 8, &[]);
    if let Some(value) = datasum {
        header.set_internal(keywords::DATASUM, value);
    }
    if let Some(value) = checksum {
        header.set_internal(keywords::CHECKSUM, value);
    }
    let bytes = written(|w| w.write_raw_hdu(&header, &[]));
    FitsReader::from_bytes(&bytes)
        .unwrap()
        .verify_checksum(0)
        .unwrap()
}

fn valid_checksum_only_report() -> ChecksumReport {
    let mut header = cards::structural(None, 8, &[]);
    header.set_internal(keywords::CHECKSUM, checksum::PLACEHOLDER);
    let mut bytes = written(|w| w.write_raw_hdu(&header, &[]));
    let hdu_sum = checksum::accumulate(&bytes, 0);
    let at = card_index(&bytes, "CHECKSUM") * CARD_SIZE;
    bytes[at + 11..at + 27].copy_from_slice(&checksum::encode(hdu_sum));
    FitsReader::from_bytes(&bytes)
        .unwrap()
        .verify_checksum(0)
        .unwrap()
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
    match ramp(bitpix, 1) {
        Data::U8(values) => ColumnData::U8(values),
        Data::I16(values) => ColumnData::I16(values),
        Data::I32(values) => ColumnData::I32(values),
        Data::I64(values) => ColumnData::I64(values),
        Data::F32(_) | Data::F64(_) => unreachable!("{bitpix:?} is not an integer type"),
    }
}

/// The header a one-column table writes, read back.
fn written_table_header(column: WriteColumn) -> Header {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_table(&binary_table(1, &[column]), None)
        .unwrap();
    let reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    reader.hdus()[1].header.clone()
}

fn assert_table_column_rejected(column: WriteColumn, expected: &'static str) {
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(1, &[column]), None)),
        FitsError::KeywordOutOfRange { keyword } if keyword == expected
    ));
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

fn rendered_header(header: &Header) -> Vec<u8> {
    let mut bytes = Vec::new();
    header
        .render_into(Signing::Unsigned, Space::Spare(0), &mut bytes)
        .unwrap();
    bytes
}

#[test]
fn writer_rejects_invalid_or_overflowing_layouts() {
    // A borrowed image is checked when it is made, so the writer never sees one whose
    // shape does not fix its sample count.
    assert!(matches!(
        Borrowed::new(&[2], View::U8(&[1]), Scaling::IDENTITY),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Samples,
            expected: 2,
            got: 1
        })
    ));
    assert!(matches!(
        Borrowed::new(&[usize::MAX, 2], View::U8(&[]), Scaling::IDENTITY),
        Err(FitsError::DataUnitOverflow)
    ));

    // usize::MAX two-byte elements per row overflow the row width.
    let fixed = WriteColumn::fixed("X", ColumnData::I16(Vec::new()), usize::MAX);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(0, &[fixed]), None)),
        FitsError::DataUnitOverflow
    ));

    let ascii = |name: &str, width| AsciiWriteColumn::text(name, AsciiText::default(), width);
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(
            &ascii_table(0, &[ascii("A", usize::MAX), ascii("B", 1)],),
            None
        )),
        FitsError::DataUnitOverflow
    ));

    let invalid_vla_bits = WriteColumn::vla(
        "FLAGS",
        Ragged::from_rows([ColumnData::Bit(bitvec![u8, Msb0; 1, 0, 1])]).unwrap(),
        DescriptorWidth::P,
    )
    .with_tdim(vec![2, 2]);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(1, &[invalid_vla_bits]), None)),
        FitsError::KeywordOutOfRange { keyword } if keyword == "TDIM1"
    ));

    let invalid_tdim =
        WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4).with_tdim(vec![5]);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(1, &[invalid_tdim]), None)),
        FitsError::KeywordOutOfRange { keyword } if keyword == "TDIM1"
    ));

    // A shape whose product overflows `usize` is out-of-range for the same reason a
    // merely-too-large one is: it cannot describe the cell's element count. The
    // reader reports the overflow identically.
    let overflowing_tdim = WriteColumn::fixed("VEC", ColumnData::I32(vec![1, 2, 3, 4]), 4)
        .with_tdim(vec![usize::MAX, 2]);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(1, &[overflowing_tdim]), None)),
        FitsError::KeywordOutOfRange { keyword } if keyword == "TDIM1"
    ));

    for shape in [vec![], vec![0]] {
        let invalid_empty_vla = WriteColumn::vla(
            "VEC",
            Ragged::from_rows(vec![ColumnData::I32(vec![])]).unwrap(),
            DescriptorWidth::P,
        )
        .with_tdim(shape);
        assert!(matches!(
            rejected_before_output(|w| w.write_table(&binary_table(1, &[invalid_empty_vla]), None)),
            FitsError::KeywordOutOfRange { keyword } if keyword == "TDIM1"
        ));
    }

    // "Vég" is four bytes, so it fills the field and reaches the ASCII check.
    let invalid_text =
        WriteColumn::fixed("NAME", ColumnData::Character("Vég".as_bytes().to_vec()), 4);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(1, &[invalid_text]), None)),
        FitsError::InvalidAscii {
            text: TextOrigin::Cell
        }
    ));

    #[cfg(target_pointer_width = "64")]
    {
        assert!(matches!(
            rejected_before_output(|w| w.write_table(&binary_table(usize::MAX, &[]), None)),
            FitsError::DataUnitOverflow
        ));
    }
}

#[test]
fn table_writers_enforce_the_exact_tfields_limit_before_output() {
    let binary = empty_binary_columns(MAX_TABLE_FIELDS);
    let reader = round_trip(|w| w.write_table(&binary_table(0, &binary), None));
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::TFIELDS)
            .unwrap(),
        Some(i64::try_from(MAX_TABLE_FIELDS).unwrap())
    );

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_ascii_table(&ascii_table(0, &ascii), None)
        .unwrap();
    let reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::TFIELDS)
            .unwrap(),
        Some(i64::try_from(MAX_TABLE_FIELDS).unwrap())
    );

    let binary = empty_binary_columns(MAX_TABLE_FIELDS + 1);
    assert!(matches!(
        rejected_before_output(|w| w.write_table(&binary_table(0, &binary), None)),
        FitsError::KeywordOutOfRange {
            keyword: keywords::TFIELDS
        }
    ));

    let ascii = empty_ascii_columns(MAX_TABLE_FIELDS + 1);
    assert!(matches!(
        rejected_before_output(|w| w.write_ascii_table(&ascii_table(0, &ascii), None)),
        FitsError::KeywordOutOfRange {
            keyword: keywords::TFIELDS
        }
    ));
}

#[test]
fn partial_hdu_failure_permanently_rejects_subsequent_writes() {
    let image = Image {
        shape: vec![1],
        samples: Data::U8(vec![7]),
        scaling: Scaling::IDENTITY,
    };
    let mut writer = FitsWriter::new(FailMidHdu::default());
    assert!(matches!(
        writer.write_image(image.view(), None),
        Err(FitsError::Io(_))
    ));
    assert_eq!(writer.state, WriterState::Failed);
    assert_eq!(writer.sink.bytes.len(), 127);

    assert!(matches!(
        writer.write_image(image.view(), None),
        Err(FitsError::WriterFailed)
    ));
    assert_eq!(writer.into_inner().bytes.len(), 127);
}

#[test]
fn writes_a_multi_hdu_image_file() {
    let primary = Image {
        shape: vec![2, 2],
        samples: Data::U8(vec![1, 2, 3, 4]),
        scaling: Scaling::IDENTITY,
    };
    let ext = Image {
        shape: vec![3],
        samples: Data::I16(vec![10, 20, 30]),
        scaling: Scaling::IDENTITY,
    };
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_image(primary.view(), None).unwrap();
    let header_ptr = w.header_scratch.as_ptr();
    let header_capacity = w.header_scratch.capacity();
    w.write_image(ext.view(), None).unwrap(); // second image ⇒ IMAGE extension
    assert_eq!(w.header_scratch.as_ptr(), header_ptr);
    assert_eq!(w.header_scratch.capacity(), header_capacity);
    let mut r = FitsReader::open(Cursor::new(w.into_inner().into_inner())).unwrap();

    assert_eq!(r.hdus().len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[1].kind, HduKind::Image);
    assert_eq!(read_whole(&mut r, 0).samples, Data::U8(vec![1, 2, 3, 4]));
    assert_eq!(read_whole(&mut r, 1).samples, Data::I16(vec![10, 20, 30]));
}

#[test]
fn typed_image_header_template_preserves_information_and_regenerates_structure() {
    let image = Image {
        shape: vec![2],
        samples: Data::I16(vec![10, 20]),
        scaling: Scaling::IDENTITY,
    };
    let template = informational_header_template();
    let mut reader = round_trip_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), Some(&template))
    });
    let header = &reader.hdus[0].header;
    assert_informational_header(header);
    assert_eq!(header.axes().unwrap(), [2]);
    // The data unit is the one 32-bit word 0x000A0014 (i16 10 then 20), so its sum is
    // 10·65536 + 20 = 655380.
    assert_cards(
        header,
        &[
            ("SIMPLE", true.into()),
            ("BITPIX", 16.into()),
            ("DATASUM", "655380".into()),
        ],
    );
    assert_absent(header, &["XTENSION", "TFIELDS", "ZIMAGE"]);
    assert_ne!(header.get_text(keywords::CHECKSUM).unwrap(), Some("stale"));
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(read_whole(&mut reader, 0).samples, Data::I16(vec![10, 20]));
}

#[test]
fn typed_table_header_templates_preserve_information_and_regenerate_structure() {
    let template = informational_header_template();
    let binary = [WriteColumn::fixed("VALUE", ColumnData::I32(vec![7, 8]), 1)];
    let ascii = [
        AsciiWriteColumn::text("TEXT", [Some("A"), Some("B")].into_iter().collect(), 3)
            .with_unit("label"),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_table(&binary_table(2, &binary), Some(&template))
        .unwrap();
    writer
        .write_ascii_table(&ascii_table(2, &ascii), Some(&template))
        .unwrap();

    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    for index in [1, 2] {
        let header = &reader.hdus[index].header;
        assert_informational_header(header);
        assert_cards(header, &[("NAXIS2", 2.into()), ("TFIELDS", 1.into())]);
        assert_absent(header, &["CHECKSUM", "DATASUM"]);
    }
    assert_eq!(reader.hdus[1].kind, HduKind::BinTable);
    assert_cards(
        &reader.hdus[1].header,
        &[("TFORM1", "1J".into()), ("TTYPE1", "VALUE".into())],
    );
    assert_absent(&reader.hdus[1].header, &["TBCOL1"]);
    assert_eq!(reader.hdus[2].kind, HduKind::AsciiTable);
    assert_cards(
        &reader.hdus[2].header,
        &[
            ("TFORM1", "A3".into()),
            ("TTYPE1", "TEXT".into()),
            ("TBCOL1", 1.into()),
        ],
    );
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::I32(vec![7, 8])
    );
    assert_eq!(
        reader
            .read_ascii_table(2)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        AsciiColumnData::Text([Some("A  "), Some("B  ")].into_iter().collect())
    );
}

#[test]
fn writes_and_reads_back_variable_length_arrays() {
    // A fixed column plus a `P` VLA column with rows of differing length.
    let vla_rows = vec![
        ColumnData::I32(vec![10, 20]),
        ColumnData::I32(vec![]), // empty cell
        ColumnData::I32(vec![1, 2, 3, 4, 5]),
    ];
    let columns = vec![
        WriteColumn::fixed("ID", ColumnData::I32(vec![1, 2, 3]), 1),
        WriteColumn::vla(
            "DATA",
            Ragged::from_rows(vla_rows.clone()).unwrap(),
            DescriptorWidth::P,
        ),
    ];
    let mut r = round_trip(|w| w.write_table(&binary_table(3, &columns), None));
    let table = r.read_table(1).unwrap();
    // A P descriptor sized to the longest row, 5.
    assert_eq!(r.hdus[1].header.get_text("TFORM2").unwrap(), Some("1PJ(5)"));
    let got = table.column(1).unwrap().vla().unwrap();
    assert_eq!(got, Ragged::from_rows(vla_rows).unwrap());

    let empty = [WriteColumn::vla(
        "EMPTY",
        Ragged::<ColumnData>::new(ColumnData::I64(Vec::new()), Vec::new()),
        DescriptorWidth::P,
    )];
    let mut r = round_trip(|w| w.write_table(&binary_table(0, &empty), None));
    let table = r.read_table(1).unwrap();
    assert_eq!(table.schema().columns[0].tform.element(), ElementKind::I64);
    assert!(table.column(0).unwrap().vla().unwrap().is_empty());
}

#[test]
fn vla_constructor_rejects_mixed_element_types() {
    assert!(matches!(
        Ragged::from_rows([ColumnData::I16(vec![1]), ColumnData::I32(vec![2])]),
        Err(FitsError::MixedArrayRows {
            row: 1,
            expected: ElementKind::I16,
            got: ElementKind::I32
        })
    ));
    assert!(matches!(
        Ragged::from_rows(Vec::new()),
        Err(FitsError::EmptyVlaNeedsType)
    ));
}

#[test]
fn writes_tdim_p_q_vla_and_bit_columns() {
    let columns = vec![
        // 2×2 multidimensional column (TDIM '(2,2)'), 4 elements/row.
        WriteColumn::fixed("MAT", ColumnData::I32((1..=8).collect()), 4).with_tdim(vec![2, 2]),
        // 64-bit Q VLA column.
        WriteColumn::vla(
            "QV",
            Ragged::from_rows(vec![
                ColumnData::I16(vec![]),
                ColumnData::I16(vec![7, 8, 9, 10]),
            ])
            .unwrap(),
            DescriptorWidth::Q,
        )
        .with_tdim(vec![2, 2]),
        WriteColumn::vla(
            "PV",
            Ragged::from_rows(vec![
                ColumnData::I16(vec![]),
                ColumnData::I16(vec![1, 2, 3, 4]),
            ])
            .unwrap(),
            DescriptorWidth::P,
        )
        .with_tdim(vec![2, 2]),
        WriteColumn::vla(
            "TXT",
            Ragged::from_rows(vec![
                ColumnData::Character(b"hello".to_vec()),
                ColumnData::Character(b"abc".to_vec()),
            ])
            .unwrap(),
            DescriptorWidth::P,
        ),
        // 12-bit X column: 2 bytes/row, the 4 bits past the width zeroed.
        WriteColumn::bits(
            "FLAGS",
            [
                bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0],
                bitvec![u8, Msb0; 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1],
            ],
            12,
        )
        .unwrap(),
    ];
    let mut r = round_trip(|w| w.write_table(&binary_table(2, &columns), None));
    let t = r.read_table(1).unwrap();
    let metadata = t.schema();

    // TDIM parsed back as a shape.
    assert_eq!(metadata.columns[0].tdim().unwrap(), Some(&[2, 2][..]));
    // P/Q descriptors with TDIM accept an empty row and validate the nonempty row.
    assert_eq!(
        metadata.columns[1].tform.storage(),
        Storage::Array(DescriptorWidth::Q)
    );
    assert_eq!(metadata.columns[1].tdim().unwrap(), Some(&[2, 2][..]));
    assert_eq!(
        t.column(1).unwrap().vla().unwrap(),
        Ragged::from_rows([ColumnData::I16(vec![]), ColumnData::I16(vec![7, 8, 9, 10])]).unwrap()
    );
    assert_eq!(
        metadata.columns[2].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[2].tdim().unwrap(), Some(&[2, 2][..]));
    assert_eq!(
        t.column(2).unwrap().vla().unwrap(),
        Ragged::from_rows([ColumnData::I16(vec![]), ColumnData::I16(vec![1, 2, 3, 4])]).unwrap()
    );
    assert_eq!(r.hdus[1].header.get_text("TFORM4").unwrap(), Some("1PA(5)"));
    assert_eq!(metadata.columns[3].tform.repeat(), 1);
    assert_eq!(
        metadata.columns[3].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[3].tform.element(), ElementKind::Character);
    assert_eq!(
        t.column(3).unwrap().vla().unwrap(),
        Ragged::from_rows([
            ColumnData::Character(b"hello".to_vec()),
            ColumnData::Character(b"abc".to_vec())
        ])
        .unwrap()
    );
    assert_eq!(metadata.columns[4].tform.element(), ElementKind::Bit);
    assert_eq!(metadata.columns[4].tform.repeat(), 12);
    // Each row's 12 bits, MSB-first, with no padding between the rows: 0xAB, 0xC
    // then 0x12, 0x3.
    assert_eq!(
        t.column(4).unwrap().raw().unwrap(),
        ColumnData::Bit(bitvec![
            u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 0, 0, 0, 0, 0, 1, 0, 0, 1, 0, 0, 0, 1, 1
        ])
    );
}

#[test]
fn writes_p_q_variable_length_bit_arrays_with_exact_counts_and_padding() {
    let mut one_bit = bitvec![u8, Msb0; 1; 8];
    one_bit.truncate(1);
    let mut p_nine = bitvec![u8, Msb0; 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1];
    p_nine.truncate(9);
    let mut q_nine = bitvec![u8, Msb0; 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1];
    q_nine.truncate(9);
    assert_eq!(one_bit.as_raw_slice(), &[0xFF]);
    assert_eq!(p_nine.as_raw_slice(), &[0xAA, 0xFF]);
    assert_eq!(q_nine.as_raw_slice(), &[0x55, 0x7F]);
    let p_rows = [BitVec::<u8, Msb0>::new(), one_bit.clone(), p_nine];
    let q_rows = [BitVec::<u8, Msb0>::new(), one_bit, q_nine];
    let columns = [
        WriteColumn::vla(
            "PFLAGS",
            Ragged::from_rows(p_rows.iter().cloned().map(ColumnData::Bit)).unwrap(),
            DescriptorWidth::P,
        ),
        WriteColumn::vla(
            "QFLAGS",
            Ragged::from_rows(q_rows.iter().cloned().map(ColumnData::Bit)).unwrap(),
            DescriptorWidth::Q,
        ),
    ];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_table(&binary_table(3, &columns), None)
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();

    assert_cards(
        &reader.hdus[1].header,
        &[
            ("TFORM1", "1PX(9)".into()),
            ("TFORM2", "1QX(9)".into()),
            ("PCOUNT", 6.into()),
        ],
    );

    let raw = reader.read_data_unit(1).unwrap();
    assert_eq!(i32::from_be_bytes(raw[0..4].try_into().unwrap()), 0);
    assert_eq!(i32::from_be_bytes(raw[4..8].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[8..16].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[16..24].try_into().unwrap()), 0);
    assert_eq!(i32::from_be_bytes(raw[24..28].try_into().unwrap()), 1);
    assert_eq!(i32::from_be_bytes(raw[28..32].try_into().unwrap()), 0);
    assert_eq!(i64::from_be_bytes(raw[32..40].try_into().unwrap()), 1);
    assert_eq!(i64::from_be_bytes(raw[40..48].try_into().unwrap()), 1);
    assert_eq!(i32::from_be_bytes(raw[48..52].try_into().unwrap()), 9);
    assert_eq!(i32::from_be_bytes(raw[52..56].try_into().unwrap()), 2);
    assert_eq!(i64::from_be_bytes(raw[56..64].try_into().unwrap()), 9);
    assert_eq!(i64::from_be_bytes(raw[64..72].try_into().unwrap()), 4);
    assert_eq!(&raw[72..78], &[0x80, 0x80, 0xAA, 0x80, 0x55, 0x00]);

    let table = reader.read_table(1).unwrap();
    let metadata = table.schema();
    assert_eq!(
        metadata.columns[0].tform.storage(),
        Storage::Array(DescriptorWidth::P)
    );
    assert_eq!(metadata.columns[0].tform.element(), ElementKind::Bit);
    assert_eq!(
        metadata.columns[1].tform.storage(),
        Storage::Array(DescriptorWidth::Q)
    );
    assert_eq!(metadata.columns[1].tform.element(), ElementKind::Bit);
    let p = table.column(0).unwrap().vla_bits().unwrap();
    let q = table.column(1).unwrap().vla_bits().unwrap();
    for row in 0..3 {
        assert_eq!(p.row(row), p_rows[row].as_bitslice());
        assert_eq!(q.row(row), q_rows[row].as_bitslice());
    }
    // The decoded rows end at their descriptor counts, 0, 1 and 9 bits: a 1-bit row
    // and a 9-bit one, which share no byte count, stay apart.
    let decoded = table.column(0).unwrap().vla().unwrap();
    assert_eq!(decoded.ends(), [0, 1, 10]);
    assert_eq!(
        decoded,
        Ragged::from_rows(p_rows.iter().cloned().map(ColumnData::Bit)).unwrap()
    );
}

#[test]
fn writes_tscal_tzero_tnull_and_reads_back_physical() {
    // Stored [5, 99] with TSCAL=2, TZERO=10, TNULL=99 ⇒ physical [20, NaN].
    let columns = vec![
        WriteColumn::fixed("X", ColumnData::I32(vec![5, 99]), 1)
            .with_scaling(2.0, 10.0)
            .with_null(99),
    ];
    let mut r = round_trip(|w| w.write_table(&binary_table(2, &columns), None));
    assert_cards(
        &r.hdus[1].header,
        &[
            ("TSCAL1", 2.0.into()),
            ("TZERO1", 10.0.into()),
            ("TNULL1", 99.into()),
        ],
    );
    let t = r.read_table(1).unwrap();
    let phys = t.column(0).unwrap().physical().unwrap();
    assert_eq!(phys[0], 20.0);
    assert!(phys[1].is_nan());
}

#[test]
fn binary_metadata_is_validated_for_every_fixed_and_vla_stored_type() {
    for (stored_type, data) in every_column_kind() {
        for column in fixed_and_vla(data) {
            let context = format!("{stored_type} {column:?}");
            if matches!(stored_type, 'A' | 'L' | 'X') {
                assert_table_column_rejected(column.clone().with_scaling(2.0, 3.0), "TSCAL1");
            } else {
                let header = written_table_header(column.clone().with_scaling(2.0, 3.0));
                assert_eq!(header.get_real("TSCAL1").unwrap(), Some(2.0), "{context}");
                assert_eq!(header.get_real("TZERO1").unwrap(), Some(3.0), "{context}");
            }

            if matches!(stored_type, 'B' | 'I' | 'J' | 'K') {
                let header = written_table_header(column.with_null(0));
                assert_eq!(header.get_integer("TNULL1").unwrap(), Some(0), "{context}");
            } else {
                assert_table_column_rejected(column.with_null(0), "TNULL1");
            }
        }
    }
}

/// A null takes exactly the values its stored integer type holds: a column's `TNULLn`,
/// fixed or variable-length, and an image's `BLANK`. A float image has no `BLANK`.
#[test]
fn nulls_are_range_checked_for_every_stored_type() {
    for bitpix in BITPIXES {
        let image = |blank| Image {
            shape: vec![1],
            samples: ramp(bitpix, 1),
            scaling: Scaling {
                blank: Some(blank),
                ..Scaling::IDENTITY
            },
        };
        let blank_refused = |blank| {
            matches!(
                rejected_before_output(|w| w.write_image(image(blank).view(), None)),
                FitsError::KeywordOutOfRange {
                    keyword: keywords::BLANK
                }
            )
        };
        let Some(range) = integer_range(bitpix) else {
            assert!(blank_refused(0), "{bitpix:?}");
            continue;
        };
        for null in [*range.start(), *range.end()] {
            for column in fixed_and_vla(integer_column_data(bitpix)) {
                let header = written_table_header(column.with_null(null));
                assert_cards(&header, &[("TNULL1", null.into())]);
            }
            let mut reader = round_trip(|w| w.write_image(image(null).view(), None));
            assert_cards(&reader.hdus[0].header, &[("BLANK", null.into())]);
            assert_eq!(read_whole(&mut reader, 0).scaling.blank, Some(null));
        }
        for null in [range.start().checked_sub(1), range.end().checked_add(1)]
            .into_iter()
            .flatten()
        {
            for column in fixed_and_vla(integer_column_data(bitpix)) {
                assert_table_column_rejected(column.with_null(null), "TNULL1");
            }
            assert!(blank_refused(null), "{bitpix:?} {null}");
        }
    }
}

/// A scaling that is not finite names the first keyword at fault, in a column, fixed or
/// variable-length, and in an image.
#[test]
fn nonfinite_scaling_is_rejected_before_output() {
    for [scale, zero] in NONFINITE {
        let at_fault = |[scale_keyword, zero_keyword]: [&'static str; 2]| {
            if scale.is_finite() {
                zero_keyword
            } else {
                scale_keyword
            }
        };
        for column in fixed_and_vla(ColumnData::I32(vec![0])) {
            assert_table_column_rejected(
                column.with_scaling(scale, zero),
                at_fault(["TSCAL1", "TZERO1"]),
            );
        }
        let image = Image {
            shape: vec![1],
            samples: Data::U8(vec![0]),
            scaling: Scaling {
                bscale: scale,
                bzero: zero,
                blank: None,
            },
        };
        assert!(matches!(
            rejected_before_output(|w| w.write_image(image.view(), None)),
            FitsError::KeywordOutOfRange { keyword }
                if keyword == at_fault(["BSCALE", "BZERO"])
        ));
    }
}

#[test]
fn writes_and_reads_back_a_binary_table() {
    let columns = vec![
        WriteColumn::fixed("NOSTA", ColumnData::I32(vec![1, 2, 3]), 1),
        WriteColumn::fixed(
            "XYZ",
            ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0]),
            3, // 3 floats per row
        )
        .with_unit("m"),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Character(b"AB CDEF  ".to_vec()),
            3, // 3-char field
        ),
    ];
    let mut r = round_trip(|w| w.write_table(&binary_table(3, &columns), None));

    // A dataless primary is auto-written before the table extension.
    assert_eq!(r.hdus().len(), 2);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    assert_eq!(r.hdus[0].header.naxis().unwrap(), 0);
    assert_eq!(r.hdus[1].kind, HduKind::BinTable);

    let t = r.read_table(1).unwrap();
    let metadata = t.schema();
    assert_eq!(metadata.row_count(), 3);
    assert_eq!(metadata.columns.len(), 3);
    assert_eq!(metadata.columns[0].name.as_deref(), Some("NOSTA"));
    assert_eq!(metadata.columns[1].unit.as_deref(), Some("m"));
    assert_eq!(
        t.column(0).unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2, 3])
    );
    assert_eq!(
        t.column(1).unwrap().raw().unwrap(),
        ColumnData::F32(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0])
    );
    assert_eq!(
        t.column(2).unwrap().raw().unwrap(),
        ColumnData::Character(b"AB CDEF  ".to_vec())
    );
}

#[test]
fn an_empty_primary_carries_the_template_only_as_the_first_hdu() {
    let template = informational_header_template();
    let table = binary_table(1, &[WriteColumn::scalar("V", ColumnData::I16(vec![5]))]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    writer.write_empty_primary(Some(&template)).unwrap();
    writer.write_table(&table, None).unwrap();
    let written_len = writer.sink.get_ref().len();
    assert!(matches!(
        writer.write_empty_primary(None),
        Err(FitsError::PrimaryAlreadyWritten)
    ));
    assert_eq!(writer.sink.get_ref().len(), written_len);

    // The table follows the caller's primary, not an automatic second one.
    let mut reader = FitsReader::from_bytes(writer.sink.get_ref()).unwrap();
    assert_eq!(reader.hdus().len(), 2);
    let primary = &reader.hdus[0];
    assert_eq!(primary.kind, HduKind::Primary);
    assert_eq!(primary.header.naxis().unwrap(), 0);
    assert_cards(
        &primary.header,
        &[
            ("SIMPLE", true.into()),
            ("EXTEND", true.into()),
            ("BITPIX", 8.into()),
        ],
    );
    assert_informational_header(&primary.header);
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(reader.hdus[1].kind, HduKind::BinTable);

    // An image written first is itself the primary.
    let image = Image::new(vec![1], vec![7u8], Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_image(image.view(), None).unwrap();
    assert!(matches!(
        writer.write_empty_primary(None),
        Err(FitsError::PrimaryAlreadyWritten)
    ));
}

#[test]
fn binary_character_columns_round_trip_exactly_and_reject_over_width() {
    let fields = b"AB  AB\0x\0xyz    ".to_vec();
    let vla_rows =
        Ragged::<ColumnData>::new(ColumnData::Character(fields.clone()), vec![4, 8, 12, 16]);
    let columns = [
        WriteColumn::fixed("FIXED", ColumnData::Character(fields.clone()), 4),
        WriteColumn::vla("PCHAR", vla_rows.clone(), DescriptorWidth::P),
        WriteColumn::vla("QCHAR", vla_rows.clone(), DescriptorWidth::Q),
    ];
    let bytes = written(|w| w.write_table(&binary_table(4, &columns), None));

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_cards(
        &reader.hdus[1].header,
        &[("TFORM2", "1PA(4)".into()), ("TFORM3", "1QA(4)".into())],
    );
    let table = reader.read_table(1).unwrap();
    assert_eq!(
        table.column(0).unwrap().raw().unwrap(),
        ColumnData::Character(fields)
    );
    assert_eq!(table.column(1).unwrap().vla().unwrap(), vla_rows);
    assert_eq!(table.column(2).unwrap().vla().unwrap(), vla_rows);
    let too_wide = WriteColumn::characters("BAD", [&b"ABCD"[..], b"ABCDE"], 4).unwrap_err();
    assert!(matches!(
        &too_wide,
        FitsError::CharacterFieldTooWide {
            column,
            row: 1,
            length: 5,
            width: 4
        } if column == "BAD"
    ));
    assert_eq!(
        too_wide.to_string(),
        "character column \"BAD\" row 1 holds 5 bytes but its field width is 4"
    );
    // A bit row has no padding, so a shorter row is refused as a longer one is.
    for (row, bits) in [
        (0, bitvec![u8, Msb0; 1, 0]),
        (0, bitvec![u8, Msb0; 1, 0, 1, 1]),
    ] {
        assert!(matches!(
            WriteColumn::bits("BITS", [bits.clone()], 3),
            Err(FitsError::BitFieldLengthMismatch { column, row: r, length, width: 3 })
                if column == "BITS" && r == row && length == bits.len()
        ));
    }
    let padded = WriteColumn::characters("PAD", ["AB", "CDE", "F"], 3).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_table(&binary_table(3, &[padded]), None)
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::Character(b"AB CDEF  ".to_vec())
    );
}

#[test]
fn rendered_header_is_block_aligned_and_ends_in_end_then_spaces() {
    let unit = rendered_header(&header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
    ]));
    assert_eq!(unit.len() % BLOCK_SIZE, 0);
    assert_eq!(unit.len(), BLOCK_SIZE); // 4 cards fit in one block

    // The 4th card (index 3) is END, followed by space padding.
    assert_eq!(&unit[3 * CARD_SIZE..3 * CARD_SIZE + 3], b"END");
    assert!(unit[4 * CARD_SIZE..].iter().all(|&b| b == SPACE_FILL));
}

#[test]
fn header_round_trips_through_render_and_parse() {
    let original = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                  -32",
        "NAXIS   =                    2",
        "NAXIS1  =                  100",
        "NAXIS2  =                   50",
        "OBJECT  = 'O''Brien'",
        "COMMENT  a remark",
    ]);
    let reparsed = Header::parse(&rendered_header(&original)).unwrap();
    assert_eq!(reparsed, original);

    let mut invalid = Header::new();
    assert!(matches!(
        invalid.set(keywords::OBJECT, "Véga"),
        Err(FitsError::InvalidAscii { text: TextOrigin::Value(name) }) if name.to_string() == "OBJECT"
    ));
    assert!(invalid.is_empty());
}

#[test]
fn image_round_trips_through_write_image_and_a_whole_read() {
    let image = Image {
        shape: vec![2, 3],
        samples: Data::I16(vec![1, -2, 3, -4, 5, -6]),
        scaling: Scaling::IDENTITY,
    };
    let bytes = written(|w| w.write_image(image.view(), None));
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE); // one header block + one data block

    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(r.hdus().len(), 1);
    assert_eq!(r.hdus[0].kind, HduKind::Primary);
    let back = read_whole(&mut r, 0);
    assert_eq!(back.shape, vec![2, 3]);
    assert_eq!(back.samples, Data::I16(vec![1, -2, 3, -4, 5, -6]));
}

#[test]
fn write_image_emits_scaling_keywords_and_preserves_unsigned_values() {
    // u16 data stored as signed-16 with BZERO = 32768.
    let image = Image {
        shape: vec![3],
        samples: Data::I16(vec![-32768, 0, 32767]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: U16_OFFSET,
            blank: None,
        },
    };
    let mut r =
        FitsReader::open(Cursor::new(written(|w| w.write_image(image.view(), None)))).unwrap();
    assert_cards(
        &r.hdus[0].header,
        &[("BZERO", 32768.0.into()), ("BSCALE", 1.0.into())],
    );
    let back = read_whole(&mut r, 0);
    assert_eq!(physical::<f64>(&back), vec![0.0, 32768.0, 65535.0]);
    assert_eq!(back.samples, Data::I16(vec![-32768, 0, 32767]));
}

#[test]
fn from_u16_round_trips_through_write_and_read() {
    // The `from_u16` constructor + writer emit BZERO=32768 so the exact u16 values
    // come back via the typed `unsigned()` view.
    let built = Image::from_u16(vec![3], &[0, 32768, 65535]).unwrap();
    let mut r =
        FitsReader::open(Cursor::new(written(|w| w.write_image(built.view(), None)))).unwrap();
    assert_eq!(
        r.hdus[0].header.get_real(keywords::BZERO).unwrap(),
        Some(32768.0)
    );
    assert_eq!(
        read_whole(&mut r, 0).view().unsigned(),
        Some(UnsignedData::U16(vec![0, 32768, 65535]))
    );
}

#[test]
fn from_u64_writes_the_exact_offset_and_round_trips_extremes() {
    let built = Image::from_u64(vec![2], &[u64::MIN, u64::MAX]).unwrap();
    let bytes = written(|w| w.write_image(built.view(), None));
    let bzero = find_card(&bytes[..BLOCK_SIZE], "BZERO");
    assert_eq!(&bzero[10..30], b" 9223372036854775808");

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    let Some(Value::Integer(offset)) = reader.hdus[0].header.get(keywords::BZERO).unwrap() else {
        panic!("unsigned-64 BZERO was not stored as an exact integer");
    };
    assert_eq!(offset.to_string(), "9223372036854775808");
    assert_eq!(
        read_whole(&mut reader, 0).view().unsigned(),
        Some(UnsignedData::U64(vec![u64::MIN, u64::MAX]))
    );
}

#[test]
fn i64_table_scaling_writes_the_exact_unsigned_offset() {
    let rows = vec![
        ColumnData::I64(vec![i64::MIN]),
        ColumnData::I64(vec![i64::MAX]),
    ];
    let columns = [
        WriteColumn::fixed("U64", ColumnData::I64(vec![i64::MIN, i64::MAX]), 1)
            .with_scaling(1.0, U64_OFFSET),
        WriteColumn::vla(
            "PU64",
            Ragged::from_rows(rows.clone()).unwrap(),
            DescriptorWidth::P,
        )
        .with_scaling(1.0, U64_OFFSET),
        WriteColumn::vla(
            "QU64",
            Ragged::from_rows(rows.clone()).unwrap(),
            DescriptorWidth::Q,
        )
        .with_scaling(1.0, U64_OFFSET),
    ];
    let bytes = written(|w| w.write_table(&binary_table(2, &columns), None));
    for keyword in ["TZERO1", "TZERO2", "TZERO3"] {
        let tzero = find_card(&bytes, keyword);
        assert_eq!(&tzero[10..30], b" 9223372036854775808");
    }

    let mut reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_cards(
        &reader.hdus[1].header,
        &[("TFORM2", "1PK(1)".into()), ("TFORM3", "1QK(1)".into())],
    );
    let table = reader.read_table(1).unwrap();
    assert_eq!(
        table.column(0).unwrap().unsigned().unwrap(),
        Some(UnsignedData::U64(vec![u64::MIN, u64::MAX]))
    );
    assert_eq!(
        table.column(1).unwrap().vla().unwrap(),
        Ragged::from_rows(rows.clone()).unwrap()
    );
    assert_eq!(
        table.column(2).unwrap().vla().unwrap(),
        Ragged::from_rows(rows).unwrap()
    );
}

/// A checksummed HDU verifies, from a stream and from a slice. A flipped data byte fails
/// both sums, and a flipped byte of the header's blank fill fails only the whole-HDU one.
#[test]
fn checksums_verify_and_tell_which_part_a_flipped_byte_is_in() {
    let image = Image {
        shape: vec![2, 2],
        samples: Data::I16(vec![1, 2, 3, 4]),
        scaling: Scaling::IDENTITY,
    };
    let written = written_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    let fill = (card_index(&written[..BLOCK_SIZE], "END") + 1) * CARD_SIZE;
    let both = ChecksumReport {
        datasum: ChecksumStatus::Invalid,
        checksum: ChecksumStatus::Invalid,
    };
    let whole_hdu = ChecksumReport {
        datasum: ChecksumStatus::Valid,
        checksum: ChecksumStatus::Invalid,
    };
    for (flipped, expected) in [
        (None, VALID),
        (Some(BLOCK_SIZE), both),
        (Some(fill), whole_hdu),
    ] {
        let mut bytes = written.clone();
        if let Some(at) = flipped {
            bytes[at] ^= 1;
        }
        let stream = FitsReader::open(Cursor::new(bytes.clone()))
            .unwrap()
            .verify_checksum(0)
            .unwrap();
        let slice = FitsReader::from_bytes(&bytes)
            .unwrap()
            .verify_checksum(0)
            .unwrap();
        assert_eq!([stream, slice], [expected; 2], "byte {flipped:?}");
    }
}

#[test]
fn checksum_status_distinguishes_absent_unknown_valid_and_invalid() {
    let image = Image {
        shape: vec![2, 2],
        samples: Data::U8(vec![0, 0, 0, 0]),
        scaling: Scaling::IDENTITY,
    };
    let mut r =
        FitsReader::open(Cursor::new(written(|w| w.write_image(image.view(), None)))).unwrap();
    let report = r.verify_checksum(0).unwrap();
    assert_eq!(
        report,
        ChecksumReport {
            datasum: ChecksumStatus::Absent,
            checksum: ChecksumStatus::Absent,
        }
    );

    // An empty or all-blank value is unknown (§4.4.2.7), as cfitsio `ffvcks` reads it.
    for (value, expected) in [
        (" ", ChecksumStatus::Unknown),
        ("", ChecksumStatus::Unknown),
        ("0", ChecksumStatus::Valid),
        ("1", ChecksumStatus::Invalid),
        ("not-a-number", ChecksumStatus::Invalid),
    ] {
        assert_eq!(
            checksum_report_for_keywords(Some(value), None),
            ChecksumReport {
                datasum: expected,
                checksum: ChecksumStatus::Absent,
            },
            "DATASUM={value:?}"
        );
    }
    for (value, expected) in [
        (" ", ChecksumStatus::Unknown),
        ("", ChecksumStatus::Unknown),
        ("not-a-checksum", ChecksumStatus::Invalid),
    ] {
        assert_eq!(
            checksum_report_for_keywords(None, Some(value)),
            ChecksumReport {
                datasum: ChecksumStatus::Absent,
                checksum: expected,
            },
            "CHECKSUM={value:?}"
        );
    }
    assert_eq!(
        valid_checksum_only_report(),
        ChecksumReport {
            datasum: ChecksumStatus::Absent,
            checksum: ChecksumStatus::Valid,
        }
    );

    // Of the 16 pairs, only (Valid, Valid) is valid, and the 7 with an `Invalid` side
    // have one.
    let statuses = [
        ChecksumStatus::Absent,
        ChecksumStatus::Unknown,
        ChecksumStatus::Valid,
        ChecksumStatus::Invalid,
    ];
    let mut valid = Vec::new();
    let mut invalid = Vec::new();
    for datasum in statuses {
        for checksum in statuses {
            let report = ChecksumReport { datasum, checksum };
            if report.is_valid() {
                valid.push((datasum, checksum));
            }
            if report.has_invalid() {
                invalid.push((datasum, checksum));
            }
        }
    }
    assert_eq!(valid, [(ChecksumStatus::Valid, ChecksumStatus::Valid)]);
    assert_eq!(
        invalid,
        [
            (ChecksumStatus::Absent, ChecksumStatus::Invalid),
            (ChecksumStatus::Unknown, ChecksumStatus::Invalid),
            (ChecksumStatus::Valid, ChecksumStatus::Invalid),
            (ChecksumStatus::Invalid, ChecksumStatus::Absent),
            (ChecksumStatus::Invalid, ChecksumStatus::Unknown),
            (ChecksumStatus::Invalid, ChecksumStatus::Valid),
            (ChecksumStatus::Invalid, ChecksumStatus::Invalid),
        ]
    );
}

#[test]
fn written_file_reads_back_with_matching_boundaries() {
    let primary = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    1",
        "NAXIS1  =                   10",
    ]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[0u8; 10]).unwrap();
    let bytes = writer.into_inner().into_inner();

    // Header block + one padded data block.
    assert_eq!(bytes.len(), 2 * BLOCK_SIZE);

    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus().len(), 1);
    assert_eq!(f.hdus[0].data_offset, BLOCK_SIZE as u64);
    assert_eq!(padded_len(f.hdus[0].data_bytes).unwrap(), BLOCK_SIZE as u64);
    assert_eq!(f.hdus[0].header.axes().unwrap(), vec![10]);

    // An extension written first follows an empty primary: one header block, then the
    // extension's header block and its one data block.
    let extension = header(&[
        "XTENSION= 'IMAGE   '",
        "BITPIX  =                    8",
        "NAXIS   =                    1",
        "NAXIS1  =                   10",
        "PCOUNT  =                    0",
        "GCOUNT  =                    1",
    ]);
    let bytes = written(|w| w.write_raw_hdu(&extension, &[7u8; 10]));
    assert_eq!(bytes.len(), 3 * BLOCK_SIZE);
    let mut f = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(f.hdus().len(), 2);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);
    assert_eq!(f.hdus[0].data_bytes, 0);
    assert_eq!(
        f.hdus[0].header.get_logical(keywords::EXTEND).unwrap(),
        Some(true)
    );
    assert_eq!(f.hdus[1].data_offset, 2 * BLOCK_SIZE as u64);
    assert_eq!(read_whole(&mut f, 1).samples, Data::U8(vec![7; 10]));
}

/// An image of more samples than one encode chunk is summed in a first pass and written
/// in a second, chunk by chunk: `CHUNK_SAMPLES + 3` samples take two chunks, the second of
/// 3 samples, and the signed file verifies and reads back each sample.
#[test]
fn checksummed_image_of_several_chunks_verifies() {
    let len = CHUNK_SAMPLES + 3;
    let samples: Vec<i16> = (0..len).map(|i| (i % 65_536) as u16 as i16).collect();
    let image = Image::new(vec![len], samples.clone(), Scaling::IDENTITY).unwrap();
    let mut reader = round_trip_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(read_whole(&mut reader, 0).samples, Data::I16(samples));
}

#[test]
fn raw_hdu_validates_the_complete_unit_before_output() {
    let header = header(&[
        "SIMPLE  =                    T",
        "BITPIX  =                   16",
        "NAXIS   =                    1",
        "NAXIS1  =                    2",
    ]);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    assert!(matches!(
        writer.write_raw_hdu(&header, &[0, 1]),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 4,
            got: 2
        })
    ));
    assert_eq!(writer.state, WriterState::Empty);
    assert!(writer.into_inner().into_inner().is_empty());
}

#[cfg(feature = "compression")]
#[test]
fn compressed_image_metadata_is_validated_before_automatic_primary() {
    let image = Image {
        shape: vec![1],
        samples: Data::F32(vec![0.0]),
        scaling: Scaling {
            blank: Some(0),
            ..Scaling::IDENTITY
        },
    };
    assert!(matches!(
        rejected_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            None
        )),
        FitsError::KeywordOutOfRange {
            keyword: keywords::BLANK
        }
    ));

    let image = Image {
        shape: vec![1, 1],
        samples: Data::I64(vec![i64::MIN]),
        scaling: Scaling::IDENTITY,
    };
    let bytes = written(|w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Rice),
            None,
        )
    });
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        read_whole(&mut reader, 1).samples,
        Data::I64(vec![i64::MIN])
    );

    // Only a 1×1 tile carries `i64::MIN` through HCOMPRESS, as any larger one takes its
    // difference with a neighbour; HCOMPRESS takes no tile under 4 pixels, so it is refused.
    assert!(matches!(
        rejected_before_output(|w| w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::Hcompress(Hcompress::default()))
                .with_tiles([1, 1]),
            None
        )),
        FitsError::HcompressTileTooSmall { axis: 0, length: 1 }
    ));
}

#[cfg(feature = "compression")]
#[test]
fn compressed_header_templates_preserve_information_and_regenerate_structure() {
    let template = informational_header_template();
    let image = Image {
        shape: vec![2],
        samples: Data::I16(vec![10, 20]),
        scaling: Scaling::IDENTITY,
    };
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP),
            Some(&template),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    let header = &reader.hdus[1].header;
    assert_informational_header(header);
    assert_cards(
        header,
        &[
            ("ZIMAGE", true.into()),
            ("ZBITPIX", 16.into()),
            ("ZNAXIS", 1.into()),
            ("ZNAXIS1", 2.into()),
        ],
    );
    // The default tiling is one row: the whole 2-sample axis.
    assert_eq!(header.get_integer("ZTILE1").unwrap(), Some(2));
    assert_absent(header, &["ZSIMPLE", "CHECKSUM"]);
    assert_eq!(read_whole(&mut reader, 1).samples, Data::I16(vec![10, 20]));

    let columns = [WriteColumn::fixed(
        "VALUE",
        ColumnData::I32(vec![7, 8, 9]),
        1,
    )];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer
        .write_compressed_table(
            &binary_table(3, &columns),
            &CompressionConfig::new(Compression::GZIP).with_tiles([2]),
            Some(&template),
        )
        .unwrap();
    let mut reader = FitsReader::open(Cursor::new(writer.into_inner().into_inner())).unwrap();
    let header = &reader.hdus[1].header;
    assert_informational_header(header);
    assert_cards(
        header,
        &[
            ("ZTABLE", true.into()),
            ("ZNAXIS2", 3.into()),
            ("ZFORM1", "1J".into()),
            ("TFORM1", "1QB".into()),
        ],
    );
    assert_eq!(
        reader
            .read_table(1)
            .unwrap()
            .column(0)
            .unwrap()
            .raw()
            .unwrap(),
        ColumnData::I32(vec![7, 8, 9])
    );
}

#[test]
fn logical_column_round_trips_with_null_state() {
    // §7.3.3: `L` is three-state — `T`/`F`/`0x00` (null); the null must survive.
    let columns = vec![WriteColumn::fixed(
        "FLAG",
        ColumnData::Logical(vec![Some(true), None, Some(false)]),
        1,
    )];
    let mut r = round_trip(|w| w.write_table(&binary_table(3, &columns), None));
    assert_eq!(
        r.read_table(1).unwrap().column(0).unwrap().raw().unwrap(),
        ColumnData::Logical(vec![Some(true), None, Some(false)])
    );
}

#[test]
fn table_builders_infer_rows_and_reject_cross_column_mismatches() {
    let mut binary = Builder::new();
    binary
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))
        .unwrap();
    binary
        .push(WriteColumn::fixed(
            "PAIR",
            ColumnData::I16(vec![10, 11, 20, 21, 30, 31]),
            2,
        ))
        .unwrap();
    assert_eq!(binary.row_count, Some(3));
    assert!(matches!(
        binary.push(WriteColumn::scalar(
            "SHORT",
            ColumnData::F64(vec![1.0, 2.0]),
        )),
        Err(FitsError::TableRowCountMismatch {
            column,
            expected: 3,
            got: 2,
        }) if column == "SHORT"
    ));

    let inferred_vla = WriteColumn::vla(
        "SAMPLES",
        Ragged::from_rows(vec![
            ColumnData::I16(vec![1, 2]),
            ColumnData::I16(Vec::new()),
            ColumnData::I16(vec![3]),
        ])
        .unwrap(),
        DescriptorWidth::P,
    );
    binary.push(inferred_vla).unwrap();

    // 7 I16 values in rows of 3 leave a row of 1; 3 bytes in 12-bit (2-byte) rows
    // leave a row of 1 byte; values in 0-element rows fit no row count; an empty
    // 0-element column fits any.
    for (name, column, count, row_width) in [
        (
            "F",
            WriteColumn::fixed("F", ColumnData::I16(vec![0; 7]), 3),
            7,
            3,
        ),
        (
            "Z",
            WriteColumn::fixed("Z", ColumnData::I16(vec![1, 2]), 0),
            2,
            0,
        ),
    ] {
        assert!(matches!(
            Builder::with_rows(2).push(column).map(|_| ()),
            Err(FitsError::PartialTableRow { column, count: c, row_width: w })
                if column == name && c == count && w == row_width
        ));
    }
    let mut empty = Builder::with_rows(2);
    empty
        .push(WriteColumn::fixed("Z", ColumnData::I16(Vec::new()), 0))
        .unwrap();
    assert_eq!(empty.row_count, Some(2));
    Builder::with_rows(0)
        .push(WriteColumn::vla(
            "EMPTY",
            Ragged::<ColumnData>::new(ColumnData::I64(Vec::new()), Vec::new()),
            DescriptorWidth::P,
        ))
        .unwrap();

    let mut ascii = AsciiBuilder::new();
    ascii
        .push(AsciiWriteColumn::text(
            "NAME",
            [Some("A"), Some("B")].into_iter().collect(),
            2,
        ))
        .unwrap()
        .push(
            AsciiWriteColumn::float(
                "VALUE",
                vec![Some(1.25), None],
                AsciiFloatFormat::F {
                    width: 8,
                    decimals: 2,
                },
            )
            .with_null("NULL"),
        )
        .unwrap();
    let reader = round_trip(|w| w.write_ascii_table(&ascii, None));
    assert_eq!(
        reader.hdus[1]
            .header
            .get_integer(keywords::NAXIS.indexed(2).unwrap())
            .unwrap(),
        Some(2)
    );
}

#[test]
fn streaming_image_matches_transactional_output_and_checksums() {
    let samples = vec![1i16, -2, 300, i16::MIN, i16::MAX, 42];
    let scaling = Scaling {
        bscale: 2.5,
        bzero: -10.0,
        blank: Some(i64::from(i16::MIN)),
    };
    let image = Image::new(vec![3, 2], samples.clone(), scaling).unwrap();
    let mut template = Header::new();
    template.set(keywords::OBJECT, "streamed").unwrap();

    let expected = written_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), Some(&template))
    });

    let mut streamed = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    {
        let mut image_stream = streamed
            .stream_image(image.view().metadata(), Some(&template))
            .unwrap();
        image_stream.write_chunk(View::I16(&samples[..2])).unwrap();
        image_stream.write_chunk(View::I16(&samples[2..5])).unwrap();
        image_stream.write_chunk(View::I16(&samples[5..])).unwrap();
        image_stream.finish().unwrap();
    }
    let actual = streamed.into_inner().into_inner();
    assert_eq!(actual, expected);

    let expected_scaled = written_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    let mut streamed_scaled = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    {
        let mut image_stream = streamed_scaled
            .stream_image(image.view().metadata(), None)
            .unwrap();
        image_stream.write_chunk(View::I16(&samples[..3])).unwrap();
        image_stream.write_chunk(View::I16(&samples[3..])).unwrap();
        image_stream.finish().unwrap();
    }
    assert_eq!(streamed_scaled.into_inner().into_inner(), expected_scaled);

    let mut reader = FitsReader::from_bytes(&actual).unwrap();
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(read_whole(&mut reader, 0).samples, Data::I16(samples));
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

/// After a chunk's write fails, the stream writes nothing more: a retried chunk and the
/// finish are refused, and so is the next HDU, so no signed but corrupt HDU is written.
#[test]
fn a_failed_stream_write_refuses_every_later_write() {
    let sink = FailingSink {
        inner: Cursor::new(Vec::new()),
        writes: 0,
        fail_at: 2,
    };
    let mut writer = FitsWriter::new(sink).with_checksums();
    let mut stream = writer
        .stream_image(
            Metadata {
                shape: &[2],
                bitpix: Bitpix::I16,
                scaling: Scaling::IDENTITY,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        stream.write_chunk(View::I16(&[1])),
        Err(FitsError::Io(_))
    ));
    assert!(matches!(
        stream.write_chunk(View::I16(&[1])),
        Err(FitsError::WriterFailed)
    ));
    assert!(matches!(stream.finish(), Err(FitsError::WriterFailed)));
    let image = Image::new(vec![1], vec![7u8], Scaling::IDENTITY).unwrap();
    assert!(matches!(
        writer.write_image(image.view(), None),
        Err(FitsError::WriterFailed)
    ));
}

#[test]
fn unfinished_or_invalid_image_stream_poisons_the_writer() {
    let fallback = Image::new(vec![1], vec![7u8], Scaling::IDENTITY).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    {
        let mut stream = writer
            .stream_image(
                Metadata {
                    shape: &[2],
                    bitpix: Bitpix::I16,
                    scaling: Scaling::IDENTITY,
                },
                None,
            )
            .unwrap();
        assert!(matches!(
            stream.write_chunk(View::U8(&[1])),
            Err(FitsError::ChunkTypeMismatch {
                expected: Bitpix::I16,
                got: Bitpix::U8
            })
        ));
    }
    assert!(matches!(
        writer.write_image(fallback.view(), None),
        Err(FitsError::WriterFailed)
    ));

    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    let stream = writer
        .stream_image(
            Metadata {
                shape: &[2],
                bitpix: Bitpix::I32,
                scaling: Scaling::IDENTITY,
            },
            None,
        )
        .unwrap();
    assert!(matches!(
        stream.finish(),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Samples,
            expected: 2,
            got: 0,
        })
    ));
    assert!(matches!(
        writer.write_image(fallback.view(), None),
        Err(FitsError::WriterFailed)
    ));
}
