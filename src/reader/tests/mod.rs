#[cfg(feature = "compression")]
use crate::ascii::ascii_text::AsciiText;
use crate::bintable::descriptor;
use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::internals::table_header;
use crate::bintable::selection::{SelectedColumn, SelectedData};
use crate::block::padded_len;
use crate::checksum::checksum_report::ChecksumStatus;
#[cfg(feature = "compression")]
use crate::compression::table::internals::mixed_columns;
#[cfg(feature = "compression")]
use crate::compression::{Compression, CompressionConfig};
use crate::fits_error::DescriptorField;
use crate::fits_error::Indexed;
use crate::fits_error::Ranked;
use crate::fits_writer::FitsWriter;
#[cfg(feature = "compression")]
use crate::fits_writer::ascii_builder::AsciiWriteColumn;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::header_model::Header;
use crate::header_model::defect::Defect;
use crate::header_model::keywords;
use crate::header_model::record::RecordView;
use crate::header_model::record_name::RecordName;
use crate::header_model::repair::{Repair, RepairKind};
use crate::image_model::Image;
use crate::image_model::bitpix::Bitpix;
use crate::image_model::data::Data;
use crate::image_model::region::Region;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::View;
use crate::internals::cards;
use crate::internals::fixture::Fixture;
use crate::internals::memory;
use crate::internals::samples;
use crate::internals::samples::BITPIXES;
use crate::internals::whole_column::WholeColumn;
use crate::reader::source::internals::{CountedReader, Counting};
use crate::reader::*;
use crate::world_coordinates::internals::{to_pixel, to_world};
use crate::world_coordinates::tabular::internals::{lookup_bytes, lookup_header};
use bitvec::bitvec;
use bitvec::order::Msb0;
use num_complex::Complex;
use std::assert_matches;
use std::io::Cursor;
use std::iter;
use std::ops::Range;
use std::ptr;
use std::slice;

mod checksums;
mod image;
mod table;
mod wcs;

#[test]
fn read_data_unit_is_stable_across_reads() {
    let mut f = Fixture::Uit.stream();
    let a = f.read_data_unit(0).unwrap();
    let b = f.read_data_unit(0).unwrap();
    assert_eq!(a, b, "repeated raw reads yield identical data");
}

/// A whole read of a stream source lands in the image it returns and stages nothing:
/// the staging buffer still holds the one header block the scan read, 2880 bytes.
#[test]
fn whole_reads_stage_nothing() {
    let mut f = Fixture::Uit.stream();
    let first = memory::read_whole(&mut f, 0);
    let second = memory::read_whole(&mut f, 0);
    assert_eq!(first.shape, second.shape);
    assert_eq!(first.samples, second.samples);
    assert!(data_source::internals::scratch(&f.data).is_empty());
}

/// A primary HDU with no data.
fn empty_primary() -> Vec<u8> {
    cards::fits_file(
        &[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
        ],
        &[],
    )
}

/// An image extension of the 3 bytes 7, 8, 9.
fn byte_image() -> Vec<u8> {
    cards::fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  =                    8",
            "NAXIS   =                    1",
            "NAXIS1  =                    3",
            "PCOUNT  =                    0",
            "GCOUNT  =                    1",
        ],
        &[7, 8, 9],
    )
}

/// A near-valid record in one HDU is repaired and reported, a record with a bad value is
/// kept as malformed, and the other HDUs read as before.
#[test]
fn defective_record_is_repaired_or_kept_and_the_other_hdus_read() {
    let defective = [
        "XTENSION= 'IMAGE   '",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
        "PCOUNT  =                    0",
        "GCOUNT  =                    1",
        "exptime =                   30",
        "OBJECT  ='M31'",
        "EXPOS= 5",
        "NOTE    = 'x' / caf",
        "GAIN    = 1.2.3",
    ];
    let mut first = cards::fits_file(&defective, &[]);
    // The `f` of `caf`, past the comment's slash.
    let caf = cards::value_field(&first, "NOTE").start + 9;
    first[caf] = 0xE9;
    let bytes = [empty_primary(), first, byte_image()].concat();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.hdus().len(), 3);
    assert_eq!(
        memory::read_whole(&mut reader, 2).samples,
        Data::U8(vec![7, 8, 9])
    );

    let header = &reader.hdus()[1].header;
    cards::assert_cards(
        header,
        &[
            ("EXPTIME", 30.into()),
            ("OBJECT", "M31".into()),
            ("EXPOS", 5.into()),
        ],
    );
    assert_eq!(header.get_comment("NOTE").unwrap(), Some("caf?"));
    assert_eq!(
        header.repairs(),
        [
            (6, RepairKind::LowercaseName),
            (7, RepairKind::ValueIndicator),
            (8, RepairKind::ValueIndicator),
            (9, RepairKind::NonAsciiText),
        ]
        .map(|(record, kind)| Repair { record, kind })
    );
    assert_matches!(header.get("GAIN"), Err(FitsError::InvalidValue { name: Some(RecordName::Keyword(name)), defect: Defect::Value }) if name == "GAIN"
    );
    assert!(header.iter().any(|record| matches!(
        record,
        RecordView::Malformed { name: Some(RecordName::Keyword(keyword)), defect: Defect::Value, .. } if *keyword == "GAIN"
    )));
    assert!(reader.hdus()[0].header.repairs().is_empty());
}

/// A defect in NAXIS1 leaves the extension unsized: it ends the HDU list, the primary
/// reads as it is, and a select of the extension gives the defect.
#[test]
fn defect_in_a_sizing_keyword_ends_the_hdu_list() {
    let mut bad_size = byte_image();
    let field = cards::value_field(&bad_size, "NAXIS1");
    bad_size[field].copy_from_slice(b"               1.2.3");
    let bytes = [empty_primary(), bad_size].concat();
    let ended = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(ended.hdus().len(), 1);
    assert_matches!(
        ended.scan_defect(),
        Some(FitsError::InvalidValue {
            name: Some(RecordName::Keyword(name)),
            defect: Defect::Value
        }) if *name == "NAXIS1"
    );
    assert_matches!(
        ended.hdu(1),
        Err(FitsError::DefectiveHdu { index: 1, source })
            if matches!(&*source, FitsError::InvalidValue { name: Some(RecordName::Keyword(name)), .. } if *name == "NAXIS1")
    );
    assert_matches!(
        ended.hdu(2),
        Err(FitsError::IndexOutOfBounds {
            index: 2,
            len: 1,
            ..
        })
    );
}

#[test]
fn defect_in_a_tform_fails_only_the_table_read() {
    let table = cards::fits_file(
        &[
            "XTENSION= 'BINTABLE'",
            "BITPIX  =                    8",
            "NAXIS   =                    2",
            "NAXIS1  =                    2",
            "NAXIS2  =                    1",
            "PCOUNT  =                    0",
            "GCOUNT  =                    1",
            "TFIELDS =                    2",
            "TFORM1  = '1B      '",
            "TFORM2  = '1B",
        ],
        &[1, 2],
    );
    let bytes = [empty_primary(), table].concat();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.hdus().len(), 2);
    assert_matches!(
        reader.read_table(1),
        Err(FitsError::InvalidValue {
            name: Some(RecordName::Keyword(name)),
            defect: Defect::Value
        }) if name == "TFORM2"
    );
}

#[test]
fn data_unit_larger_than_the_file_is_refused_not_allocated() {
    // A header claiming a 2 TB data unit (10¹² 16-bit samples) in a file that holds one
    // data block: every read refuses it before it sizes a buffer from the claim, on both
    // sources, and a stream's staging buffer stays empty, as the header scan reads its
    // blocks straight into the header.
    let bytes = cards::fits_file(
        &[
            "SIMPLE  = T",
            "BITPIX  = 16",
            "NAXIS   = 1",
            "NAXIS1  = 1000000000000",
        ],
        &[0u8; 16],
    );
    let mut stream = FitsReader::open(Cursor::new(bytes.as_slice())).unwrap();
    let scanned = stream.data.staging_capacity();
    assert_eq!(scanned, 0);
    assert_matches!(stream.image(0), Err(FitsError::UnexpectedEof));
    assert_matches!(stream.read_data_unit(0), Err(FitsError::UnexpectedEof));
    assert_eq!(stream.data.staging_capacity(), scanned);
    let mut slice = FitsReader::from_bytes(&bytes).unwrap();
    assert_matches!(slice.image(0), Err(FitsError::UnexpectedEof));
    assert_matches!(slice.read_data_unit(0), Err(FitsError::UnexpectedEof));
}

/// A data unit is read without its block fill: 512×512 `i16` samples are 524288 bytes,
/// of the 527040 the unit takes on disk.
#[test]
fn read_data_unit_returns_the_data_without_fill() {
    let mut f = Fixture::Uit.stream();
    assert_eq!(f.read_data_unit(0).unwrap().len(), 524_288);
    assert_eq!(f.hdus()[0].padded_data_bytes(), 527_040);
}

#[test]
fn read_data_unit_refuses_an_out_of_bounds_index() {
    let mut f = Fixture::Uit.stream(); // a single-HDU file
    assert_matches!(
        f.read_data_unit(5),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index: 5,
            len: 1
        })
    );
}

#[test]
fn readers_recover_their_original_sources() {
    let image = Image::new(vec![2], vec![1u8, 2], Scaling::IDENTITY).unwrap();
    let bytes = memory::written(|w| w.write_image(image.view(), None));
    let slice = bytes.slice().into_bytes();
    assert!(ptr::eq(slice.as_ptr(), bytes.as_ptr()));
    assert_eq!(slice.len(), bytes.len());

    let cursor = FitsReader::open(Cursor::new(bytes.to_vec()))
        .unwrap()
        .into_inner();
    assert_eq!(cursor.into_inner(), *bytes);
}

/// A read of one kind refuses an HDU of each other kind, naming both, and takes its own:
/// an image the primary, `IMAGE` and tiled-compressed ones, a table a `BINTABLE` and a
/// tiled-compressed one. HDU 0 of `Ddts` is a random-groups primary.
#[cfg(feature = "compression")]
#[test]
fn each_read_refuses_the_hdus_of_other_kinds() {
    type Read = fn(&mut SliceReader<'_>, usize) -> Result<()>;
    let image = Image::new(vec![2], Data::U8(vec![1, 2]), Scaling::IDENTITY).unwrap();
    let table =
        Builder::with_columns(1, vec![WriteColumn::scalar("A", ColumnData::I16(vec![1]))]).unwrap();
    let text: AsciiText = [Some("A")].into_iter().collect();
    let ascii = memory::ascii_table(1, &[AsciiWriteColumn::text("A", text, 1)]);
    let config = CompressionConfig::new(Compression::GZIP);
    let mut bytes = memory::written(|w| {
        w.write_image(image.view(), None)?;
        w.write_image(image.view(), None)?;
        w.write_ascii_table(&ascii, None)?;
        w.write_table(&table, None)?;
        w.write_compressed_image(image.view(), &config, None)?;
        w.write_compressed_table(&table, &config, None)
    })
    .into_bytes();
    bytes.extend_from_slice(&cards::fits_file(
        &[
            "XTENSION= 'FOO     '",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
            "PCOUNT  =                    0",
            "GCOUNT  =                    1",
        ],
        &[],
    ));
    let groups = Fixture::Ddts.bytes();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut groups = FitsReader::from_bytes(groups).unwrap();
    let kinds = [
        HduKind::Primary,
        HduKind::Image,
        HduKind::AsciiTable,
        HduKind::BinTable,
        HduKind::CompressedImage,
        HduKind::CompressedTable,
        HduKind::Other,
    ];
    let reads: [(&str, ExpectedHdu, &[HduKind], Read); 7] = [
        (
            "image",
            ExpectedHdu::Image,
            &[HduKind::Primary, HduKind::Image, HduKind::CompressedImage],
            |r, hdu| r.image(hdu).map(drop),
        ),
        (
            "image_metadata",
            ExpectedHdu::Image,
            &[HduKind::Primary, HduKind::Image, HduKind::CompressedImage],
            |r, hdu| r.hdu(hdu)?.image_metadata().map(drop),
        ),
        (
            "read_table",
            ExpectedHdu::BinTable,
            &[HduKind::BinTable, HduKind::CompressedTable],
            |r, hdu| r.read_table(hdu).map(drop),
        ),
        (
            "table_schema",
            ExpectedHdu::BinTable,
            &[HduKind::BinTable, HduKind::CompressedTable],
            |r, hdu| r.hdu(hdu)?.table_schema().map(drop),
        ),
        (
            "read_ascii_table",
            ExpectedHdu::AsciiTable,
            &[HduKind::AsciiTable],
            |r, hdu| r.read_ascii_table(hdu).map(drop),
        ),
        (
            "ascii_schema",
            ExpectedHdu::AsciiTable,
            &[HduKind::AsciiTable],
            |r, hdu| r.hdu(hdu)?.ascii_schema().map(drop),
        ),
        (
            "read_groups",
            ExpectedHdu::RandomGroups,
            &[HduKind::RandomGroups],
            |r, hdu| r.read_groups(hdu).map(drop),
        ),
    ];
    for (name, expected, takes, read) in reads {
        for index in 0..=kinds.len() {
            let (reader, index, kind) = match kinds.get(index) {
                Some(&kind) => (&mut reader, index, kind),
                None => (&mut groups, 0, HduKind::RandomGroups),
            };
            assert_eq!(reader.hdus()[index].kind, kind);
            let result = read(reader, index);
            if takes.contains(&kind) {
                result.unwrap_or_else(|error| panic!("{name} of {kind:?}: {error}"));
            } else {
                assert_matches!(
                    result,
                    Err(FitsError::HduKindMismatch { expected: e, got }) if e == expected && got == kind,
                    "{name} of {kind:?}"
                );
            }
        }
    }
}
