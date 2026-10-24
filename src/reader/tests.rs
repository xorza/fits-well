use crate::checksum::checksum_report::ChecksumStatus;
use bitvec::bitvec;
use bitvec::order::Msb0;

use crate::bintable::descriptor;
use crate::bintable::descriptor::DescriptorWidth;
use crate::bintable::internals::table_header;
use crate::bintable::selection::{SelectedColumn, SelectedData};
use crate::bitpix::Bitpix;
use crate::block::BLOCK_SIZE;
use crate::block::padded_len;
use crate::checksum;
#[cfg(feature = "compression")]
use crate::compress::table::internals::mixed_columns;
#[cfg(feature = "compression")]
use crate::compress::{Compression, CompressionConfig};
use crate::error::Indexed;
use crate::error::Ranked;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::header_model::defect::Defect;
use crate::header_model::keywords;
use crate::header_model::record::RecordView;
use crate::header_model::record_name::RecordName;
use crate::header_model::repair::{Repair, RepairKind};
use crate::header_model::{Header, Signing, Space};
use crate::image_model::data::Data;
use crate::image_model::region::Region;
use crate::image_model::scaling::Scaling;
use crate::image_model::view::View;
use crate::image_model::{Image, Parts};
use crate::internals::cards;
use crate::internals::cards::{assert_cards, fits_file, from_card_lines, records};
use crate::internals::fixture::Fixture;
use crate::internals::memory::{memory_writer, read_whole, written, written_by};
use crate::internals::samples::{BITPIXES, assert_same_bits, gather, ramp, section_indices};
use crate::ragged::Ragged;
use crate::reader::source::internals::CountedReader;
use crate::reader::*;
use crate::world_coordinates::internals::{to_pixel, to_world};
use crate::world_coordinates::tabular::internals::{lookup_bytes, lookup_header};
use num_complex::Complex;
use std::cell::RefCell;
use std::io::{self, Cursor, SeekFrom};
use std::iter;
use std::ptr;
use std::rc::Rc;
use std::slice;

/// A cursor that records the byte range of every read it serves.
#[derive(Debug)]
struct CountingCursor {
    inner: Cursor<Vec<u8>>,
    read_ranges: Rc<RefCell<Vec<Range<usize>>>>,
}

impl Read for CountingCursor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let start = self.inner.position() as usize;
        let count = self.inner.read(buffer)?;
        if count != 0 {
            self.read_ranges.borrow_mut().push(start..start + count);
        }
        Ok(count)
    }
}

impl Seek for CountingCursor {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.inner.seek(position)
    }
}

fn write_tab_lookup(
    writer: &mut FitsWriter<Cursor<Vec<u8>>>,
    version: i64,
    level: i64,
    coordinates: &[f64],
) {
    let shape = format!("(1,{})", coordinates.len());
    write_tab_lookup_columns(
        writer,
        version,
        level,
        &[("COORD", coordinates, Some(shape.as_str()))],
    );
}

fn write_tab_lookup_columns(
    writer: &mut FitsWriter<Cursor<Vec<u8>>>,
    version: i64,
    level: i64,
    columns: &[(&str, &[f64], Option<&str>)],
) {
    let mut header = lookup_header(columns);
    header
        .set_internal(keywords::EXTNAME, "WCS-TABLE")
        .set_internal(keywords::EXTVER, version)
        .set_internal(keywords::EXTLEVEL, level);
    writer
        .write_raw_hdu(&header, &lookup_bytes(columns))
        .unwrap();
}

#[test]
fn read_wcs_resolves_the_exact_tabular_extension() {
    let mut primary = cards::structural(None, 8, &[1]);
    primary
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 1.0)
        .set_internal("CDELT1", 1.0)
        .set_internal("PS1_0", "WCS-TABLE")
        .set_internal("PV1_1", 2)
        .set_internal("PV1_2", 3)
        .set_internal("PS1_1", "COORD")
        .set_internal("PV1_3", 1);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[0]).unwrap();
    // A decoy BINTABLE whose EXTNAME does not match. Its EXTVER is not an integer, so
    // resolving the reference must never interpret it — an unrelated corrupt version
    // card cannot fail a lookup that does not concern it.
    let mut decoy = table_header(0, 0, &[]);
    decoy
        .set_internal(keywords::EXTNAME, "OTHER")
        .set_internal(keywords::EXTVER, "not-an-integer");
    writer.write_raw_hdu(&decoy, &[]).unwrap();
    write_tab_lookup(&mut writer, 2, 1, &[100.0, 200.0, 400.0]);
    write_tab_lookup(&mut writer, 2, 3, &[10.0, 20.0, 40.0]);

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        to_world(&Wcs::from_header(&reader.hdus[0].header, None).unwrap(), &[2.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0]
    ));
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
    assert_eq!(to_pixel(&wcs, &[30.0]).unwrap(), [2.5]);

    // An extension the file does not hold is the lookup's own error.
    let alone = written(|w| w.write_raw_hdu(&primary, &[0]));
    assert!(matches!(
        FitsReader::from_bytes(&alone).unwrap().read_wcs(0, None),
        Err(FitsError::HduNotFound { name, version: Some(2), level: Some(3) })
            if name == "WCS-TABLE"
    ));

    // A tiled-compressed table serves its arrays as the plain one does.
    #[cfg(feature = "compression")]
    {
        let lookup = Builder::with_columns(
            1,
            [
                WriteColumn::fixed("COORD", ColumnData::F64(vec![10.0, 20.0, 40.0]), 3)
                    .with_tdim([1, 3]),
            ],
        )
        .unwrap();
        let mut names = Header::new();
        names
            .set_internal(keywords::EXTNAME, "WCS-TABLE")
            .set_internal(keywords::EXTVER, 2)
            .set_internal(keywords::EXTLEVEL, 3);
        let compressed = written(|w| {
            w.write_raw_hdu(&primary, &[0])?;
            w.write_compressed_table(
                &lookup,
                &CompressionConfig::new(Compression::GZIP),
                Some(&names),
            )
        });
        let mut reader = FitsReader::from_bytes(&compressed).unwrap();
        let wcs = reader.read_wcs(0, None).unwrap();
        assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
    }
}

#[test]
fn read_wcs_resolves_shared_arrays_and_extensions_once_per_reference_group() {
    let mut primary = cards::structural(None, 8, &[]);
    primary.set_internal(keywords::WCSAXES, 5);
    for (axis, column, version, table_axis) in [
        (1, "FIRST", 1, 1),
        (2, "SECOND", 1, 1),
        (3, "COUPLED", 1, 1),
        (4, "COUPLED", 1, 2),
        (5, "DISTINCT", 2, 1),
    ] {
        primary.set_tab_axis(axis, column, table_axis);
        primary.set_internal(format!("PV{axis}_1").as_str(), version);
    }
    let coupled = [30.0, 40.0, 31.0, 40.0, 30.0, 42.0, 31.0, 42.0];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[]).unwrap();
    write_tab_lookup_columns(
        &mut writer,
        1,
        1,
        &[
            ("FIRST", &[10.0, 20.0], Some("(1,2)")),
            ("SECOND", &[100.0, 200.0], Some("(1,2)")),
            ("COUPLED", &coupled, Some("(2,2,2)")),
        ],
    );
    write_tab_lookup_columns(
        &mut writer,
        2,
        1,
        &[("DISTINCT", &[1000.0, 2000.0], Some("(1,2)"))],
    );

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(
        to_world(&wcs, &[1.0, 1.0, 1.0, 1.0, 1.0]).unwrap(),
        [10.0, 100.0, 30.0, 40.0, 1000.0]
    );
}

#[test]
fn read_wcs_fetches_only_the_referenced_first_row_heap_cells() {
    let mut primary = cards::structural(None, 8, &[1]);
    primary
        .set_internal("CTYPE1", "WAVE-TAB")
        .set_internal("CUNIT1", "m")
        .set_internal("CRPIX1", 1.0)
        .set_internal("CRVAL1", 10.0)
        .set_internal("CDELT1", 10.0)
        .set_internal("PS1_0", "WCS-TABLE")
        .set_internal("PS1_1", "COORD")
        .set_internal("PS1_2", "INDEX")
        .set_internal("PV1_3", 1);
    let row_count = 64;
    let coordinates = (0..row_count).map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]));
    let indices = (0..row_count).map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]));
    let unused = (0..row_count).map(|_| ColumnData::U8(vec![7; 1024]));
    let table = Builder::with_columns(
        row_count,
        vec![
            WriteColumn::vla(
                "COORD",
                Ragged::from_rows(coordinates).unwrap(),
                DescriptorWidth::P,
            )
            .with_tdim(vec![1, 3]),
            WriteColumn::vla(
                "INDEX",
                Ragged::from_rows(indices).unwrap(),
                DescriptorWidth::P,
            ),
            WriteColumn::vla(
                "UNUSED",
                Ragged::from_rows(unused).unwrap(),
                DescriptorWidth::P,
            ),
        ],
    )
    .unwrap();
    let mut lookup_header = Header::new();
    lookup_header.set_internal(keywords::EXTNAME, "WCS-TABLE");
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[0]).unwrap();
    writer.write_table(&table, Some(&lookup_header)).unwrap();

    let read_ranges = Rc::new(RefCell::new(Vec::new()));
    let source = CountingCursor {
        inner: Cursor::new(writer.into_inner().into_inner()),
        read_ranges: Rc::clone(&read_ranges),
    };
    let mut reader = FitsReader::open(source).unwrap();
    read_ranges.borrow_mut().clear();
    let wcs = reader.read_wcs(0, None).unwrap();
    let bytes_read: usize = read_ranges.borrow().iter().map(Range::len).sum();
    assert_eq!(bytes_read, 24 + 6 * size_of::<f64>());
    assert_eq!(to_world(&wcs, &[2.0]).unwrap(), [20.0]);
}

#[test]
fn reads_a_single_hdu_image_with_exact_boundaries() {
    let f = Fixture::Uit.stream();
    assert_eq!(f.hdus().len(), 1);
    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.bitpix().unwrap(), Bitpix::I16);
    assert_eq!(p.header.axes().unwrap(), vec![512, 512]);
    assert_eq!(p.data_offset, 11_520);
    assert_eq!(padded_len(p.data_bytes).unwrap(), 527_040);
    let bytes = Fixture::Uit.bytes();
    assert_eq!(
        p.header_sum,
        checksum::accumulate(&bytes[..p.data_offset as usize], 0)
    );
}

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
    let first = read_whole(&mut f, 0);
    let second = read_whole(&mut f, 0);
    assert_eq!(first.shape, second.shape);
    assert_eq!(first.samples, second.samples);
    assert_eq!(data_source::internals::scratch(&f.data).len(), BLOCK_SIZE);
}

#[test]
fn reads_random_groups_primary_plus_bintable_extension() {
    let f = Fixture::Ddts.stream();
    assert_eq!(f.hdus().len(), 2);

    let g = &f.hdus[0];
    assert_eq!(g.kind, HduKind::RandomGroups);
    assert_eq!(g.header.bitpix().unwrap(), Bitpix::F32);
    assert_eq!(g.header.axes().unwrap(), vec![0, 3, 4, 1, 1, 1]);
    assert_eq!(g.data_offset, 14_400);
    assert_eq!(padded_len(g.data_bytes).unwrap(), 573_120);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 593_280);
    assert_eq!(padded_len(t.data_bytes).unwrap(), 2_880);

    let bytes = Fixture::Ddts.bytes();
    assert_eq!(
        g.header_sum,
        checksum::accumulate(&bytes[..g.data_offset as usize], 0)
    );
    let second_header = g.data_offset + padded_len(g.data_bytes).unwrap();
    assert_eq!(
        t.header_sum,
        checksum::accumulate(&bytes[second_header as usize..t.data_offset as usize], 0,)
    );
}

#[test]
fn reads_dataless_primary_then_bintable() {
    let f = Fixture::Iue.stream();
    assert_eq!(f.hdus().len(), 2);

    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.naxis().unwrap(), 0);
    assert_eq!(p.data_offset, 28_800);
    assert_eq!(padded_len(p.data_bytes).unwrap(), 0);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 34_560);
    assert_eq!(padded_len(t.data_bytes).unwrap(), 14_400);
}

#[test]
fn trailing_special_records_and_partial_blocks_are_ignored() {
    // A valid single-HDU file, then §3.5 special records / §3.6 trailing fill and
    // partial blocks appended — none carrying an `END`. The reader must still find
    // exactly the one real HDU and not error on the trailing bytes.
    let mut bytes = Fixture::Uit.bytes().to_vec();
    bytes.extend(iter::repeat_n(0u8, BLOCK_SIZE)); // trailing all-zero fill block
    bytes.extend(iter::repeat_n(b'x', BLOCK_SIZE)); // a special record (no END)
    bytes.extend_from_slice(b"a truncated tail"); // sub-block partial remnant
    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus().len(), 1);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);

    // A special block may legally contain a canonical END-shaped card later in
    // the block. Its first card is not XTENSION, so none of it is an HDU header.
    let mut bytes = Fixture::Uit.bytes().to_vec();
    bytes.extend_from_slice(&fits_file(
        &["COMMENT special records", "BITPIX  = 8", "NAXIS   = 0"],
        &[],
    ));
    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus().len(), 1);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);
}

/// An `END` record with stray bytes after its name still ends the header: the file
/// opens, and the header reports the repair.
#[test]
fn an_end_record_with_stray_bytes_ends_the_header() {
    let mut file = records(&[
        "SIMPLE  =                    T",
        "BITPIX  =                    8",
        "NAXIS   =                    0",
        "END     junk",
    ]);
    file.resize(BLOCK_SIZE, b' ');
    let reader = FitsReader::from_bytes(&file).unwrap();
    assert_eq!(reader.hdus().len(), 1);
    assert_eq!(
        reader.hdus()[0].header.repairs(),
        [Repair {
            record: 4,
            kind: RepairKind::EndText
        }]
    );
}

/// A near-valid record in one HDU is repaired and reported, a record with a bad value
/// is kept as malformed, and the other HDUs read as before; only a defect in a keyword
/// that sizes an HDU stops the open.
#[test]
fn defective_records_fail_only_the_reads_that_need_them() {
    let primary = fits_file(
        &[
            "SIMPLE  =                    T",
            "BITPIX  =                    8",
            "NAXIS   =                    0",
        ],
        &[],
    );
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
    let mut first = fits_file(&defective, &[]);
    let note = 8 * 80;
    first[note + 19] = 0xE9;
    let last = fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  =                    8",
            "NAXIS   =                    1",
            "NAXIS1  =                    3",
            "PCOUNT  =                    0",
            "GCOUNT  =                    1",
        ],
        &[7, 8, 9],
    );
    let bytes = [primary.clone(), first, last.clone()].concat();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.hdus().len(), 3);
    assert_eq!(read_whole(&mut reader, 2).samples, Data::U8(vec![7, 8, 9]));

    let header = &reader.hdus()[1].header;
    assert_cards(
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
    assert!(
        matches!(header.get("GAIN"), Err(FitsError::InvalidValue { name: Some(RecordName::Keyword(name)), defect: Defect::Value }) if name == "GAIN")
    );
    assert!(header.iter().any(|record| matches!(
        record,
        RecordView::Malformed { name: Some(RecordName::Keyword(keyword)), defect: Defect::Value, .. } if *keyword == "GAIN"
    )));
    let mut rendered = Vec::new();
    let mut edited = header.clone();
    assert!(matches!(
        edited.render_into(Signing::Unsigned, Space::Spare(0), &mut rendered),
        Err(FitsError::InvalidValue { .. })
    ));
    edited.set("GAIN", 1.25).unwrap();
    edited
        .render_into(Signing::Unsigned, Space::Spare(0), &mut rendered)
        .unwrap();
    let text = String::from_utf8(rendered).unwrap();
    for record in [
        "EXPTIME =                   30",
        "OBJECT  = 'M31     '",
        "EXPOS   =                    5",
        "GAIN    =                 1.25",
    ] {
        assert!(text.contains(record), "{record}");
    }
    assert!(reader.hdus()[0].header.repairs().is_empty());

    // A defect in NAXIS1 leaves the next HDU unfound; one in TFORM2 fails only the
    // table read.
    let mut bad_size = last;
    bad_size[3 * 80 + 10..3 * 80 + 30].copy_from_slice(b"               1.2.3");
    assert!(matches!(
        FitsReader::from_bytes(&[primary.clone(), bad_size].concat()),
        Err(FitsError::InvalidValue { .. })
    ));
    let table = fits_file(
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
    let bytes = [primary, table].concat();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        reader.read_table(1),
        Err(FitsError::InvalidValue { .. })
    ));
}

#[test]
fn malformed_image_pcount_is_rejected_not_panicked() {
    // A primary array with PCOUNT=5 is non-conforming (§4.3): an error, not a
    // panic in the data-extent arithmetic.
    let bytes = fits_file(
        &[
            "SIMPLE  = T",
            "BITPIX  = 8",
            "NAXIS   = 1",
            "NAXIS1  = 10",
            "PCOUNT  = 5",
            "GCOUNT  = 1",
        ],
        &[0u8; 15],
    );
    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert!(matches!(r.image(0), Err(FitsError::ImageHasGroups)));
    // The scan could not resolve the image, and reports why on request.
    assert!(matches!(
        r.hdus()[0].image_metadata(),
        Err(FitsError::ImageHasGroups)
    ));
    assert!(matches!(
        r.hdus()[0].table_schema(),
        Err(FitsError::NotABinTable)
    ));
}

#[test]
fn data_unit_larger_than_the_file_is_rejected_not_allocated() {
    // A header claiming a ~1 MB data unit in a file that holds only a single data
    // block must error up front, not attempt the header-sized allocation.
    let bytes = fits_file(
        &[
            "SIMPLE  = T",
            "BITPIX  = 8",
            "NAXIS   = 1",
            "NAXIS1  = 1000000",
            "PCOUNT  = 0",
            "GCOUNT  = 1",
        ],
        &[0u8; 16],
    );
    let mut r = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert!(matches!(
        r.image(0).unwrap().read(Region::All),
        Err(FitsError::UnexpectedEof)
    ));
    assert!(matches!(r.read_table(0), Err(FitsError::NotABinTable)));
    assert!(matches!(
        r.read_ascii_table(0),
        Err(FitsError::NotAnAsciiTable)
    ));
}

#[test]
fn content_before_any_valid_hdu_is_rejected() {
    // Garbage with no `END` and no preceding HDU is not a FITS file.
    let bytes = vec![b'x'; BLOCK_SIZE + 17];
    assert!(matches!(
        FitsReader::open(Cursor::new(bytes)),
        Err(FitsError::MissingKeyword {
            keyword: keywords::SIMPLE
        })
    ));
}

#[test]
fn role_aware_scanning_rejects_invalid_file_starts_and_group_signatures() {
    assert!(matches!(
        FitsReader::open(Cursor::new(Vec::<u8>::new())),
        Err(FitsError::UnexpectedEof)
    ));

    let extension_only = fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  = 8",
            "NAXIS   = 0",
            "PCOUNT  = 0",
            "GCOUNT  = 1",
        ],
        &[],
    );
    assert!(matches!(
        FitsReader::open(Cursor::new(extension_only)),
        Err(FitsError::MissingKeyword {
            keyword: keywords::SIMPLE
        })
    ));

    let invalid_groups = fits_file(
        &[
            "SIMPLE  = T",
            "BITPIX  = 8",
            "NAXIS   = 1",
            "NAXIS1  = 2",
            "GROUPS  = T",
            "PCOUNT  = 0",
            "GCOUNT  = 1",
        ],
        &[],
    );
    assert!(matches!(
        FitsReader::open(Cursor::new(invalid_groups)),
        Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "NAXIS1"
    ));
}

#[test]
fn role_aware_extents_find_the_exact_next_hdu_boundary() {
    let mut bytes = fits_file(
        &["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 1", "NAXIS1  = 3"],
        &[1, 2, 3],
    );
    bytes.extend_from_slice(&fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  = 8",
            "NAXIS   = 1",
            "NAXIS1  = 5",
            "PCOUNT  = 0",
            "GCOUNT  = 1",
        ],
        &[4, 5, 6, 7, 8],
    ));
    let reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(reader.hdus().len(), 2);
    assert_eq!(reader.hdus[0].kind, HduKind::Primary);
    assert_eq!(reader.hdus[0].data_offset, BLOCK_SIZE as u64);
    assert_eq!(reader.hdus[0].data_bytes, 3);
    assert_eq!(reader.hdus[1].kind, HduKind::Image);
    assert_eq!(reader.hdus[1].data_offset, (3 * BLOCK_SIZE) as u64);
    assert_eq!(reader.hdus[1].data_bytes, 5);
}

#[test]
fn extensions_require_pcount_and_gcount_for_boundary_sizing() {
    let mut bytes = fits_file(&["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 0"], &[]);
    bytes.extend_from_slice(&fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  = 8",
            "NAXIS   = 0",
            "GCOUNT  = 1",
        ],
        &[],
    ));
    assert!(matches!(
        FitsReader::open(Cursor::new(bytes)),
        Err(FitsError::MissingKeyword {
            keyword: keywords::PCOUNT
        })
    ));
}

#[test]
fn last_data_unit_ends_exactly_at_end_of_file() {
    for fixture in [Fixture::Uit, Fixture::Ddts, Fixture::Iue] {
        let f = fixture.stream();
        let last = f.hdus.last();
        assert_eq!(
            last.data_offset + padded_len(last.data_bytes).unwrap(),
            u64::try_from(fixture.bytes().len()).unwrap(),
            "{fixture:?}"
        );
    }
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
fn read_data_unit_rejects_out_of_bounds_index() {
    let mut f = Fixture::Uit.stream(); // a single-HDU file
    assert!(matches!(
        f.read_data_unit(5),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index: 5,
            len: 1
        })
    ));
}

/// Every image path reads `BLANK` one way: a float image ignores it and reads, an
/// integer `BLANK` its type cannot hold fails the metadata, the view and the read
/// alike. 40000 is no `i16`.
#[test]
fn every_image_path_judges_blank_alike() {
    let float = written(|w| {
        w.write_raw_hdu(
            &from_card_lines(&[
                "SIMPLE  = T",
                "BITPIX  = -32",
                "NAXIS   = 1",
                "NAXIS1  = 1",
                "BLANK   = 7",
            ]),
            &1.5f32.to_be_bytes(),
        )
    });
    let mut r = FitsReader::from_bytes(&float).unwrap();
    assert_eq!(r.hdus()[0].image_metadata().unwrap().scaling.blank, None);
    let mut words = Vec::new();
    assert_eq!(
        r.image(0)
            .unwrap()
            .view(Region::All, &mut words)
            .unwrap()
            .samples(),
        View::F32(&[1.5])
    );
    assert_eq!(
        r.image(0).unwrap().read(Region::All).unwrap().samples,
        Data::F32(vec![1.5])
    );

    let integer = written(|w| {
        w.write_raw_hdu(
            &from_card_lines(&[
                "SIMPLE  = T",
                "BITPIX  = 16",
                "NAXIS   = 1",
                "NAXIS1  = 1",
                "BLANK   = 40000",
            ]),
            &3i16.to_be_bytes(),
        )
    });
    let mut r = FitsReader::from_bytes(&integer).unwrap();
    let blank = |result: Result<()>| matches!(result, Err(FitsError::KeywordOutOfRange { keyword }) if keyword == "BLANK");
    assert!(blank(r.hdus()[0].image_metadata().map(|_| ())));
    assert!(blank(r.image(0).map(|_| ())));
}

#[test]
fn image_reads_the_primary_array_shape_and_type() {
    let mut f = Fixture::Uit.stream();
    let mut image = f.image(0).unwrap();
    assert_eq!(image.metadata().shape, [512, 512]);
    assert_eq!(image.metadata().bitpix, Bitpix::I16);
    // The sum, extremes and centre sample of the 262144 stored values; BSCALE is
    // 2.0587209E-16 and BZERO 0.
    let mut physical: Vec<f64> = Vec::new();
    image.physical_into(Region::All, &mut physical).unwrap();
    assert_eq!(physical.len(), 512 * 512);
    let Data::I16(samples) = image.read(Region::All).unwrap().samples else {
        panic!("expected I16 samples");
    };
    assert_eq!(
        samples.iter().map(|&v| i64::from(v)).sum::<i64>(),
        1_050_151
    );
    assert_eq!(samples.iter().min(), Some(&-9));
    assert_eq!(samples.iter().max(), Some(&2049));
    let centre = 256 * 512 + 256;
    assert_eq!(samples[centre], 2);
    assert_eq!(physical[centre], 2.0 * 2.058_720_9e-16);
}

#[test]
fn image_samples_match_a_manual_big_endian_decode() {
    let mut f = Fixture::Uit.stream();
    // Independently decode every pixel straight from the data bytes.
    let unit = f.read_data_unit(0).unwrap();
    let manual: Vec<i16> = unit
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_be_bytes(*c))
        .collect();
    assert_eq!(read_whole(&mut f, 0).samples, Data::I16(manual));
}

#[test]
fn image_rejects_non_image_hdus() {
    // hdu[0] is random groups, hdu[1] is a binary table — neither is an image.
    let mut f = Fixture::Ddts.stream();
    assert!(matches!(f.image(0), Err(FitsError::NotAnImage)));
    assert!(matches!(f.image(1), Err(FitsError::NotAnImage)));
}

#[test]
fn hdu_selectors_find_hdus_by_index_name_version_and_level() {
    let f = Fixture::Ddts.stream();
    // hdu 1 is the AIPS antenna table, EXTNAME = 'AIPS AN' (trailing spaces trimmed).
    assert_eq!(f.hdu("AIPS AN").unwrap().index, 1);
    assert_eq!(f.hdu("aips an").unwrap().index, 1); // case-insensitive
    assert_eq!(f.hdu(1).unwrap().index, 1);
    // The primary has no EXTNAME, so it is named PRIMARY, in any case.
    assert_eq!(f.hdu("PRIMARY").unwrap().index, 0);
    assert_eq!(f.hdu("primary").unwrap().index, 0);
    assert!(f.hdus()[0].matches("PRIMARY").unwrap());
    assert!(!f.hdus()[1].matches("PRIMARY").unwrap());
    assert!(matches!(
        f.hdu(("AIPS AN", 99)), // no such EXTVER
        Err(FitsError::HduNotFound { name, version: Some(99), level: None }) if name == "AIPS AN"
    ));
    // EXTLEVEL is 1 where the card is absent.
    let level = |level| HduSelector::Extension {
        name: "AIPS AN",
        version: None,
        level: Some(level),
    };
    assert_eq!(f.hdu(level(1)).unwrap().index, 1);
    assert!(matches!(
        f.hdu(level(2)),
        Err(FitsError::HduNotFound { level: Some(2), .. })
    ));
    assert!(matches!(
        f.hdu("MISSING"),
        Err(FitsError::HduNotFound { name, version: None, level: None }) if name == "MISSING"
    ));
    assert!(matches!(
        f.hdu(9),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index: 9,
            len: 2
        })
    ));
    // A tiled-compressed image extension is found by its EXTNAME too.
    assert_eq!(
        Fixture::CompGzipI16
            .stream()
            .hdu("COMPRESSED_IMAGE")
            .unwrap()
            .index,
        1
    );

    let mut header = cards::structural(None, 8, &[]);
    header.set_internal(keywords::EXTNAME, 7);
    let bytes = written(|w| w.write_raw_hdu(&header, &[]));
    // An `EXTNAME` that is no text names nothing, not even the primary's default, and
    // fails no lookup.
    let malformed = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        malformed.hdus()[0].extname(),
        Err(FitsError::ValueTypeMismatch { .. })
    ));
    assert!(matches!(
        malformed.hdu("SCI"),
        Err(FitsError::HduNotFound { .. })
    ));
    assert!(!malformed.hdus()[0].matches("PRIMARY").unwrap());

    // A written extension is found by its name in any case and its EXTVER.
    let primary = Image::new(vec![1], vec![0u8], Scaling::IDENTITY).unwrap();
    let extension = Image::new(vec![3], vec![10i16, 20, 30], Scaling::IDENTITY).unwrap();
    let mut extension_header = Header::new();
    extension_header.set(keywords::EXTNAME, "SCI").unwrap();
    extension_header.set(keywords::EXTVER, 2).unwrap();
    extension_header.set(keywords::OBJECT, "target").unwrap();

    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_image(primary.view(), None).unwrap();
    writer
        .write_image(extension.view(), Some(&extension_header))
        .unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();

    let index = reader.hdu(("sci", 2)).unwrap().index;
    assert_eq!(index, 1);
    let hdu = &reader.hdus()[index];
    assert_eq!(
        (
            hdu.extname().unwrap(),
            hdu.extver().unwrap(),
            hdu.extlevel().unwrap()
        ),
        (Some("SCI"), 2, 1)
    );
    assert_eq!(reader.hdus()[index].kind, HduKind::Image);
    assert_eq!(
        reader.hdus()[index]
            .header
            .get_text(keywords::OBJECT)
            .unwrap(),
        Some("target")
    );
    assert_eq!(
        read_whole(&mut reader, index).samples,
        Data::I16(vec![10, 20, 30])
    );
    // `..` reads the whole image, and its parts come apart as the accessors give them.
    let image = reader.image(("SCI", 2)).unwrap().read(..).unwrap();
    assert_eq!(
        (image.shape(), image.samples(), image.scaling()),
        (&[3][..], &Data::I16(vec![10, 20, 30]), Scaling::IDENTITY)
    );
    let Parts {
        shape,
        samples,
        scaling,
    } = image.into_parts();
    assert_eq!(
        (shape, samples, scaling),
        (vec![3], Data::I16(vec![10, 20, 30]), Scaling::IDENTITY)
    );
    assert_eq!(reader.hdus()[0].kind, HduKind::Primary);
    assert!(matches!(
        reader.hdu(("SCI", 3)),
        Err(FitsError::HduNotFound {
            version: Some(3),
            ..
        })
    ));
}

#[test]
fn images_lists_readable_images_including_compressed() {
    let images = |fixture: Fixture| {
        fixture
            .stream()
            .images()
            .map(|hdu| hdu.index)
            .collect::<Vec<_>>()
    };
    // A single primary array image.
    assert_eq!(images(Fixture::Uit), [0]);
    // Empty primary + a tiled-compressed image extension (classified by ZIMAGE),
    // so only HDU 1 is an image — the `NAXIS = 0` primary is skipped.
    assert_eq!(images(Fixture::CompGzipI16), [1]);
    // Random-groups primary + plain bintable: no images at all.
    assert!(images(Fixture::Ddts).is_empty());

    // `NAXIS = 0` holds no array in a primary and in an image extension alike.
    let mut bytes = fits_file(
        &["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 0", "EXTEND  = T"],
        &[],
    );
    let extensions: [(&[&str], &[u8]); 2] = [
        (&["NAXIS   = 0"], &[]),
        (&["NAXIS   = 1", "NAXIS1  = 2"], &[1, 2]),
    ];
    for (naxis, data) in extensions {
        let cards = [
            &["XTENSION= 'IMAGE   '", "BITPIX  = 8"][..],
            naxis,
            &["PCOUNT  = 0", "GCOUNT  = 1"],
        ]
        .concat();
        bytes.extend(fits_file(&cards, data));
    }
    let reader = FitsReader::open(Cursor::new(bytes)).unwrap();
    let images: Vec<bool> = reader.hdus().iter().map(Hdu::is_image).collect();
    assert_eq!(images, [false, false, true]);
    assert_eq!(
        reader.images().map(|hdu| hdu.index).collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn a_u8_view_borrows_the_source_with_zero_copy() {
    let image = Image {
        shape: vec![4],
        samples: Data::U8(vec![10, 20, 30, 40]),
        scaling: Scaling::IDENTITY,
    };
    let buf = written(|w| w.write_image(image.view(), None));

    let mut reader = FitsReader::from_bytes(&buf).unwrap();
    let mut image = reader.image(0).unwrap();
    assert_eq!(image.metadata().shape, [4]);
    assert_eq!(image.metadata().bitpix, Bitpix::U8);
    // U8 needs no byte-swap, so the view borrows the stored bytes in the source: its
    // address lies within `buf`, and the caller's words stay untouched.
    let mut scratch = Vec::new();
    let view = image.view(Region::All, &mut scratch).unwrap();
    assert_eq!(view.shape(), [4]);
    let v = view.samples().as_u8().expect("a U8 image views as U8");
    assert_eq!(v, &[10, 20, 30, 40]);
    let base = buf.as_ptr() as usize;
    assert!(
        (base..base + buf.len()).contains(&(v.as_ptr() as usize)),
        "the u8 view must point inside the source buffer (zero-copy)"
    );
    assert!(scratch.is_empty(), "a U8 view must not touch the scratch");
}

#[test]
fn a_multibyte_view_swaps_into_the_callers_words() {
    let image = Image {
        shape: vec![3],
        samples: Data::I16(vec![1, -2, 300]),
        scaling: Scaling::IDENTITY,
    };
    let buf = written(|w| w.write_image(image.view(), None));

    // The stored bytes are big-endian: 1 → 0x0001, -2 → 0xFFFE, 300 → 0x012C.
    let mut reader = FitsReader::from_bytes(&buf).unwrap();
    assert_eq!(reader.read_data_unit(0).unwrap(), [0, 1, 255, 254, 1, 44]);
    // A type that needs byte-swapping views from the caller's words, not the source.
    let mut image = reader.image(0).unwrap();
    let mut words = Vec::new();
    let view = image.view(Region::All, &mut words).unwrap();
    let v = view.samples().as_i16().expect("an I16 image views as I16");
    assert_eq!(v, [1, -2, 300]);
    assert_eq!(v.as_ptr(), words.as_ptr().cast::<i16>());
}

#[test]
fn a_whole_view_matches_the_owned_read_of_a_plain_image() {
    let mut f = Fixture::Uit.stream();
    // The owned decode is the reference; the borrowed view (into a caller scratch)
    // must equal it.
    let expected_scaling = f.hdus()[0].image_metadata().unwrap().scaling;
    let owned = read_whole(&mut f, 0).samples;
    let mut scratch = Vec::new();
    let mut image = f.image(0).unwrap();
    let view = image.view(Region::All, &mut scratch).unwrap();
    assert_eq!(view.shape(), [512, 512]);
    assert_eq!(view.scaling(), expected_scaling);
    match (view.samples(), &owned) {
        (View::I16(v), Data::I16(o)) => assert_eq!(v, o.as_slice()),
        (v, o) => panic!("expected matching I16 view/decode, got {v:?} / {o:?}"),
    }
}

#[test]
#[cfg(feature = "compression")]
fn a_whole_view_matches_the_owned_read_of_a_compressed_image() {
    let mut f = Fixture::CompGzipI16.stream();
    let owned = read_whole(&mut f, 1).samples;
    let mut scratch: Vec<u64> = Vec::with_capacity(owned.len().div_ceil(4));
    let scratch_ptr = scratch.as_ptr().cast::<i16>();
    let mut image = f.image(1).unwrap();
    let view = image.view(Region::All, &mut scratch).unwrap();
    assert_eq!(view.shape(), [24, 16]);
    match (view.samples(), &owned) {
        (View::I16(v), Data::I16(o)) => {
            assert_eq!(v, o.as_slice());
            assert_eq!(v.as_ptr(), scratch_ptr);
        }
        (v, o) => panic!("expected matching I16 view/decode, got {v:?} / {o:?}"),
    }
}

#[test]
fn image_sections_match_hand_computed_values_for_every_bitpix() {
    // x + 5y + 20z over x in 1..4, y in 1..3, z in 1..3: 26, 27, 28, 31, … 53.
    let idxs = section_indices(&[5, 4, 3], &[1..4, 1..3, 1..3]);
    assert_eq!(idxs, [26, 27, 28, 31, 32, 33, 46, 47, 48, 51, 52, 53]);
    for bitpix in BITPIXES {
        let samples = ramp(bitpix, 60);
        let expected = gather(&samples, &idxs);
        let image = Image::new(vec![5, 4, 3], samples, Scaling::IDENTITY).unwrap();
        let bytes = written(|w| w.write_image(image.view(), None));
        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        let mut scratch = Vec::new();
        let mut image = reader.image(0).unwrap();
        let section = image.view(&[1..4, 1..3, 1..3], &mut scratch).unwrap();
        assert_eq!(section.shape, [3, 2, 2], "{bitpix:?}");
        assert_eq!(section.scaling, Scaling::IDENTITY, "{bitpix:?}");
        assert_same_bits(
            &section.samples.to_data(),
            &expected,
            &format!("{bitpix:?}"),
        );
    }
}

#[test]
fn image_sections_preserve_scaling_and_validate_empty_and_invalid_regions() {
    let image = Image::new(
        vec![4, 3],
        vec![1i16, 2, -99, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank: Some(-99),
        },
    )
    .unwrap();
    let bytes = written(|w| w.write_image(image.view(), None));
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut handle = reader.image(0).unwrap();
    let section = handle.read(&[1..4, 0..2]).unwrap();
    assert_eq!(section.shape, [3, 2]);
    assert_eq!(section.scaling, image.scaling);
    // Stored 2, -99 (BLANK), 4, 6, 7, 8: 10 + 2·2 = 14, NaN, 18, 22, 24, 26 — the owned
    // read's view and the handle's one pass from the stored bytes agree, appended
    // after what `out` already held.
    let expected = [14.0, f64::NAN, 18.0, 22.0, 24.0, 26.0];
    let mut from_view: Vec<f64> = Vec::new();
    section.view().physical_into(&mut from_view);
    let mut from_bytes = vec![-1.0];
    handle
        .physical_into(&[1..4, 0..2], &mut from_bytes)
        .unwrap();
    for physical in [&*from_view, &from_bytes[1..]] {
        assert_eq!(physical.len(), expected.len());
        for (got, want) in physical.iter().zip(expected) {
            assert!(
                got == &want || got.is_nan() && want.is_nan(),
                "{physical:?}"
            );
        }
    }
    assert_eq!(from_bytes[0], -1.0);

    let empty = handle.read(&[2..2, 1..3]).unwrap();
    assert_eq!(empty.shape, [0, 2]);
    assert!(empty.samples.is_empty());
    let wrong_rank = 0..1;
    assert!(matches!(
        handle.read(slice::from_ref(&wrong_rank)),
        Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: 2,
            got: 1,
        })
    ));
    assert!(matches!(
        handle.read(&[0..5, 0..1]),
        Err(FitsError::ImageRegionOutOfBounds {
            axis: 0,
            start: 0,
            end: 5,
            len: 4,
        })
    ));
}

#[test]
fn plain_image_section_streams_exact_strided_runs() {
    let shape = [6, 5, 4, 3];
    let samples: Vec<i16> = (0..shape.iter().product::<usize>())
        .map(|index| i16::try_from(index).unwrap() - 100)
        .collect();
    let image = Image::new(shape.to_vec(), samples.clone(), Scaling::IDENTITY).unwrap();
    let bytes = written(|w| w.write_image(image.view(), None));
    let read_ranges = Rc::new(RefCell::new(Vec::new()));
    let source = CountingCursor {
        inner: Cursor::new(bytes.clone()),
        read_ranges: Rc::clone(&read_ranges),
    };
    let mut stream = FitsReader::open(source).unwrap();
    let data_offset = stream.hdus[0].data_offset as usize;
    read_ranges.borrow_mut().clear();
    let ranges = [1..3, 1..4, 0..4, 1..3];
    let section = stream.image(0).unwrap().read(&ranges).unwrap();

    let mut expected_values = Vec::new();
    let mut expected_reads = Vec::new();
    for axis3 in 1..3 {
        for axis2 in 0..4 {
            for axis1 in 1..4 {
                let element = 1 + 6 * axis1 + 30 * axis2 + 120 * axis3;
                expected_values.extend_from_slice(&samples[element..element + 2]);
                let start = data_offset + element * size_of::<i16>();
                expected_reads.push(start..start + 2 * size_of::<i16>());
            }
        }
    }
    assert_eq!(section.shape, [2, 3, 4, 2]);
    assert_eq!(section.samples, Data::I16(expected_values.clone()));
    assert_eq!(&*read_ranges.borrow(), &expected_reads);

    let mut slice = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        slice.image(0).unwrap().read(&ranges).unwrap().samples,
        Data::I16(expected_values)
    );
}

#[cfg(feature = "compression")]
#[test]
fn compressed_image_sections_cross_tile_boundaries_and_match_the_whole_image() {
    for bitpix in BITPIXES {
        // Sample 13 is the integer types' `BLANK`, and a NaN with a payload in the floats.
        let mut samples = ramp(bitpix, 9 * 7);
        let blank = match &mut samples {
            Data::U8(values) => Some(i64::from(values[13])),
            Data::I16(values) => Some(i64::from(values[13])),
            Data::I32(values) => Some(i64::from(values[13])),
            Data::I64(values) => Some(values[13]),
            Data::F32(values) => {
                values[13] = f32::from_bits(0x7FC0_1234);
                None
            }
            Data::F64(values) => {
                values[13] = f64::from_bits(0x7FF8_0000_0000_1234);
                None
            }
        };
        let scaling = Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank,
        };
        let image = Image::new(vec![9, 7], samples, scaling).unwrap();
        let bytes = written(|w| {
            w.write_compressed_image(
                image.view(),
                &CompressionConfig::new(Compression::GZIP).with_tiles([4, 3]),
                None,
            )
        });
        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        // The encoded image's geometry, not the container's two-axis BINTABLE.
        let metadata = reader.hdus()[1].image_metadata().unwrap();
        assert_eq!(
            (metadata.shape, metadata.bitpix, metadata.scaling),
            (&[9, 7][..], bitpix, scaling)
        );
        assert_eq!(
            reader.hdus()[1].container_schema().unwrap().row_count(),
            3 * 3
        );
        let whole = read_whole(&mut reader, 1).samples;
        let expected = gather(&whole, &section_indices(&[9, 7], &[2..9, 1..7]));
        let mut image = reader.image(1).unwrap();
        let owned = image.read(&[2..9, 1..7]).unwrap();
        assert_eq!(owned.shape, [7, 6], "{bitpix:?}");
        assert_eq!(owned.scaling, scaling, "{bitpix:?}");
        assert_same_bits(&owned.samples, &expected, &format!("{bitpix:?} owned"));
        // The blank sits at source (4, 1), which is (2, 0) in the section; no other
        // sample of any case holds the blank value. The handle's physical read decodes
        // the same tiles, so it gives the owned read's values bit for bit.
        let mut physical: Vec<f64> = Vec::new();
        owned.view().physical_into(&mut physical);
        assert!(physical[2].is_nan(), "{bitpix:?}");
        assert_eq!(physical.iter().filter(|value| value.is_nan()).count(), 1);
        let mut from_handle: Vec<f64> = Vec::new();
        image
            .physical_into(&[2..9, 1..7], &mut from_handle)
            .unwrap();
        let bits = |values: &[f64]| values.iter().map(|v| v.to_bits()).collect::<Vec<_>>();
        assert_eq!(bits(&from_handle), bits(&physical), "{bitpix:?}");

        let mut words = Vec::new();
        let view = image.view(&[2..9, 1..7], &mut words).unwrap();
        assert_eq!(view.shape, [7, 6], "{bitpix:?}");
        assert_eq!(view.scaling, scaling, "{bitpix:?}");
        assert_same_bits(
            &view.samples.to_data(),
            &expected,
            &format!("{bitpix:?} view"),
        );

        let empty = image.read(&[2..2, 1..7]).unwrap();
        assert_eq!(empty.shape, [0, 6], "{bitpix:?}");
        assert!(empty.samples.is_empty(), "{bitpix:?}");

        if bitpix == Bitpix::U8 {
            // A one-tile corner fetches its tile's row and the heap bytes that row
            // addresses — exactly what a one-row table selection holds. The whole image
            // fetches every row and the whole heap, which the writer packs without gaps:
            // the data unit, once.
            let fetched = [[0..2, 0..2], [0..9, 0..7]].map(|ranges| {
                let mut counted = CountedReader::open(&bytes);
                let mut scratch = Vec::new();
                counted
                    .reader
                    .image(1)
                    .unwrap()
                    .view(&ranges, &mut scratch)
                    .unwrap();
                counted.fetched()
            });
            let first_tile = reader
                .read_container_rows(1, 0..1)
                .unwrap()
                .schema
                .extent
                .heap_end;
            let data_bytes = reader.hdus()[1].data_bytes as usize;
            assert_eq!(fetched, [first_tile, data_bytes]);
        }
    }
}

#[test]
fn ranged_table_access_matches_whole_table_for_special_column_kinds() {
    let rows = 4;
    let columns = vec![
        WriteColumn::scalar("ID", ColumnData::I32(vec![10, 20, 30, 40])),
        WriteColumn::scalar(
            "LOGICAL",
            ColumnData::Logical(vec![Some(true), Some(false), None, Some(true)]),
        ),
        WriteColumn::scalar("BYTE", ColumnData::U8(vec![1, 2, 3, 4])),
        WriteColumn::bits(
            "FLAGS",
            [
                bitvec![u8, Msb0; 1, 0, 1, 0, 0],
                bitvec![u8, Msb0; 0, 1, 0, 1, 0],
                bitvec![u8, Msb0; 1, 1, 1, 0, 0],
                bitvec![u8, Msb0; 0, 0, 0, 1, 0],
            ],
            5,
        )
        .unwrap(),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Character(b"one two tri four".to_vec()),
            4,
        ),
        WriteColumn::scalar(
            "COMPLEX",
            ColumnData::ComplexF32(vec![
                Complex::new(1.0, -1.0),
                Complex::new(2.0, -2.0),
                Complex::new(3.0, -3.0),
                Complex::new(4.0, -4.0),
            ]),
        ),
        WriteColumn::scalar(
            "COMPLEX64",
            ColumnData::ComplexF64(vec![
                Complex::new(10.0, -10.0),
                Complex::new(20.0, -20.0),
                Complex::new(30.0, -30.0),
                Complex::new(40.0, -40.0),
            ]),
        ),
        WriteColumn::scalar("SCALED", ColumnData::I16(vec![1, -99, 2, 3]))
            .with_scaling(2.0, 10.0)
            .with_null(-99),
        WriteColumn::scalar("LONG", ColumnData::I64(vec![100, 200, 300, 400])),
        WriteColumn::scalar("FLOAT", ColumnData::F32(vec![1.5, 2.5, 3.5, 4.5])),
        WriteColumn::scalar("DOUBLE", ColumnData::F64(vec![5.5, 6.5, 7.5, 8.5])),
        WriteColumn::fixed("ZERO", ColumnData::I16(Vec::new()), 0),
        WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![
                ColumnData::I32(vec![1]),
                ColumnData::I32(vec![2, 3]),
                ColumnData::I32(Vec::new()),
                ColumnData::I32(vec![4, 5, 6]),
            ])
            .unwrap(),
            DescriptorWidth::P,
        ),
        WriteColumn::vla(
            "QVLA",
            Ragged::from_rows(vec![
                ColumnData::F64(vec![0.5]),
                ColumnData::F64(Vec::new()),
                ColumnData::F64(vec![2.5, 3.5]),
                ColumnData::F64(vec![4.5]),
            ])
            .unwrap(),
            DescriptorWidth::Q,
        ),
    ];
    let table = Builder::with_columns(rows, columns).unwrap();
    let bytes = written(|w| w.write_table(&table, None));
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();

    let schema = reader.hdu(1).unwrap().table_schema().unwrap().clone();
    assert_eq!(schema.row_count(), 4);
    assert_eq!(schema.columns.len(), 14);
    let empty = reader.read_table_rows(1, 2..2).unwrap();
    assert_eq!(empty.schema().row_count(), 0);
    assert_eq!(
        empty.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(Vec::new()), Vec::new())
    );
    let full = reader.read_table_rows(1, 0..4).unwrap();
    assert_eq!(
        full.column("QVLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]), vec![1, 1, 3, 4])
    );
    let ranged = reader.read_table_rows(1, 1..3).unwrap();
    assert_eq!(
        ranged.column("ID").unwrap().raw().unwrap(),
        ColumnData::I32(vec![20, 30])
    );
    let flags = ranged.column("FLAGS").unwrap().bits().unwrap();
    assert_eq!(
        (0..5)
            .map(|column| flags.get(0, column).unwrap())
            .collect::<Vec<_>>(),
        [false, true, false, true, false]
    );
    assert_eq!(
        ranged.column("NAME").unwrap().raw().unwrap(),
        ColumnData::Character(b"two tri ".to_vec())
    );
    assert_eq!(
        ranged.column("COMPLEX").unwrap().complex().unwrap(),
        [Complex::new(2.0, -2.0), Complex::new(3.0, -3.0)]
    );
    let physical = ranged.column("SCALED").unwrap().physical().unwrap();
    assert!(physical[0].is_nan());
    assert_eq!(physical[1], 14.0);
    assert_eq!(
        ranged.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2, 2])
    );
    assert_eq!(
        ranged.column("QVLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::F64(vec![2.5, 3.5]), vec![0, 2])
    );

    let selection = reader.read_table_columns(1, 1..3, ["ID", "VLA"]).unwrap();
    assert_eq!(selection.rows, 1..3);
    assert!(matches!(
        &selection.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values == &[20, 30]
    ));
    assert!(matches!(
        &selection.columns[1].data,
        SelectedData::Variable(values)
            if values == &Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2, 2])
    ));
    // `ID` is column 0 and `VLA` column 12 of the table; `QVLA`, column 13, is not selected.
    let number = |column: Option<&SelectedColumn>| column.map(|column| column.descriptor.number);
    assert_eq!(number(selection.column("id")), Some(1));
    assert_eq!(number(selection.column(0)), Some(1));
    assert_eq!(number(selection.column(12)), Some(13));
    assert!(selection.column(13).is_none());
    assert!(selection.column("QVLA").is_none());

    let mut reused = selection.clone();
    let id_buffer = match &reused.columns[0].data {
        SelectedData::Fixed(ColumnData::I32(values)) => values.as_ptr(),
        other => panic!("{other:?}"),
    };
    reader
        .read_table_columns_into(1, 1..3, ["ID", "VLA"], &mut reused)
        .unwrap();
    assert_eq!(reused.rows, selection.rows);
    assert_eq!(reused.columns[0].data, selection.columns[0].data);
    assert_eq!(reused.columns[1].data, selection.columns[1].data);
    assert!(matches!(
        &reused.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values.as_ptr() == id_buffer
    ));
    // The fixed `ID` column becomes the variable `QVLA` column, and `VLA` is dropped.
    reader
        .read_table_columns_into(1, 0..4, ["QVLA"], &mut reused)
        .unwrap();
    assert_eq!(reused.rows, 0..4);
    assert_eq!(reused.columns.len(), 1);
    assert_eq!(reused.columns[0].descriptor.name.as_deref(), Some("QVLA"));
    assert_eq!(
        reused.columns[0].data,
        SelectedData::Variable(Ragged::<ColumnData>::new(
            ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]),
            vec![1, 1, 3, 4]
        ))
    );
    assert!(matches!(
        reader.read_table_columns_into(1, 0..4, ["ID", "MISSING"], &mut reused),
        Err(FitsError::ColumnNotFound { name }) if name == "MISSING"
    ));
    assert_eq!(reused.rows, 0..0);
    assert!(reused.columns.is_empty());
    assert_eq!(
        reader.read_table_cell(1, 3, "QVLA").unwrap(),
        ColumnData::F64(vec![4.5])
    );
    for (column, expected) in [
        ("ID", ColumnData::I32(vec![30])),
        ("LOGICAL", ColumnData::Logical(vec![None])),
        ("BYTE", ColumnData::U8(vec![3])),
        ("FLAGS", ColumnData::Bit(bitvec![u8, Msb0; 1, 1, 1, 0, 0])),
        ("NAME", ColumnData::Character(b"tri ".to_vec())),
        (
            "COMPLEX",
            ColumnData::ComplexF32(vec![Complex::new(3.0, -3.0)]),
        ),
        (
            "COMPLEX64",
            ColumnData::ComplexF64(vec![Complex::new(30.0, -30.0)]),
        ),
        ("SCALED", ColumnData::I16(vec![2])),
        ("LONG", ColumnData::I64(vec![300])),
        ("FLOAT", ColumnData::F32(vec![3.5])),
        ("DOUBLE", ColumnData::F64(vec![7.5])),
        ("ZERO", ColumnData::I16(Vec::new())),
        ("VLA", ColumnData::I32(Vec::new())),
        ("QVLA", ColumnData::F64(vec![2.5, 3.5])),
    ] {
        assert_eq!(
            reader.read_table_cell(1, 2, column).unwrap(),
            expected,
            "{column}"
        );
    }
    assert!(matches!(
        reader.read_table_rows(1, 3..5),
        Err(FitsError::RowRangeOutOfBounds {
            start: 3,
            end: 5,
            len: 4,
        })
    ));
    assert!(matches!(
        reader.read_table_cell(1, 4, "ID"),
        Err(FitsError::RowRangeOutOfBounds {
            start: 4,
            end: 5,
            len: 4,
        })
    ));

    let mut corrupted = bytes.clone();
    let qvla = &schema.columns[schema
        .columns
        .iter()
        .position(|column| column.name.as_deref() == Some("QVLA"))
        .unwrap()];
    let qvla_slot =
        usize::try_from(reader.hdus[1].data_offset).unwrap() + schema.row_len() + qvla.byte_offset;
    corrupted[qvla_slot..qvla_slot + qvla.tform.byte_width()].fill(0xff);
    let mut selective = FitsReader::from_bytes(&corrupted).unwrap();
    let selection = selective
        .read_table_columns(1, 1..2, ["ID", "VLA"])
        .unwrap();
    assert!(matches!(
        &selection.columns[0].data,
        SelectedData::Fixed(ColumnData::I32(values)) if values == &[20]
    ));
    assert!(matches!(
        &selection.columns[1].data,
        SelectedData::Variable(values)
            if values == &Ragged::<ColumnData>::new(ColumnData::I32(vec![2, 3]), vec![2])
    ));
    assert_eq!(
        selective.read_table_cell(1, 1, "ID").unwrap(),
        ColumnData::I32(vec![20])
    );
    assert!(matches!(
        selective.read_table_rows(1, 1..2),
        Err(FitsError::InvalidPqDescriptor {
            field: "count",
            value: -1
        })
    ));

    let mut stream = FitsReader::open(Cursor::new(bytes)).unwrap();
    let one = stream.read_table_rows(1, 3..4).unwrap();
    assert_eq!(
        one.column("VLA").unwrap().vla().unwrap(),
        Ragged::<ColumnData>::new(ColumnData::I32(vec![4, 5, 6]), vec![3])
    );
    assert_eq!(
        stream
            .read_table_columns(1, 0..4, ["QVLA"])
            .unwrap()
            .columns[0]
            .data,
        SelectedData::Variable(Ragged::<ColumnData>::new(
            ColumnData::F64(vec![0.5, 2.5, 3.5, 4.5]),
            vec![1, 1, 3, 4]
        ))
    );
}

#[test]
fn malformed_pq_descriptors_match_across_table_read_paths() {
    for width in [DescriptorWidth::P, DescriptorWidth::Q] {
        let column = WriteColumn::vla(
            "VLA",
            Ragged::from_rows(vec![ColumnData::U8(vec![7])]).unwrap(),
            width,
        );
        let table = Builder::with_columns(1, vec![column]).unwrap();
        let bytes = written(|w| w.write_table(&table, None));
        let base_reader = FitsReader::from_bytes(&bytes).unwrap();
        let schema = base_reader.hdu(1).unwrap().table_schema().unwrap();
        let slot = usize::try_from(base_reader.hdus[1].data_offset).unwrap()
            + schema.columns[0].byte_offset;

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.width == width)
        {
            let mut corrupted = bytes.clone();
            corrupted[slot..slot + case.bytes.len()].copy_from_slice(&case.bytes);
            let mut reader = FitsReader::from_bytes(&corrupted).unwrap();

            let whole = reader.read_table(1).unwrap();
            case.assert_error(&whole.column("VLA").unwrap().vla().unwrap_err());
            case.assert_error(&reader.read_table_cell(1, 0, "VLA").unwrap_err());
            case.assert_error(&reader.read_table_rows(1, 0..1).unwrap_err());
            case.assert_error(&reader.read_table_columns(1, 0..1, ["VLA"]).unwrap_err());
        }
    }
}

#[test]
fn readers_recover_their_original_sources() {
    let image = Image::new(vec![2], vec![1u8, 2], Scaling::IDENTITY).unwrap();
    let bytes = written(|w| w.write_image(image.view(), None));
    let slice = FitsReader::from_bytes(&bytes).unwrap().into_bytes();
    assert!(ptr::eq(slice.as_ptr(), bytes.as_ptr()));
    assert_eq!(slice.len(), bytes.len());

    let cursor = FitsReader::open(Cursor::new(bytes.clone()))
        .unwrap()
        .into_inner();
    assert_eq!(cursor.into_inner(), bytes);
}

/// A data checksum fed by a decode reading a BITPIX=16 image front to back, in sections whose
/// runs split words, gives the report `verify_checksum` gives — valid, then invalid once a stored
/// byte changes — and the decode with its finish fetch the data unit once, block fill included,
/// where the decode and a separate verification fetch it twice. Sections read out of order feed
/// only what continues the sum, and the finish reads the rest. The owned and the borrowed section
/// reads feed the sum alike.
#[test]
fn a_data_checksum_summed_during_the_decode_reads_the_unit_once() {
    let image = Image {
        shape: vec![7, 5],
        samples: Data::I16((0..35).map(|value| value * 731 - 9000).collect()),
        scaling: Scaling::IDENTITY,
    };
    let bytes = written_by(memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    let padded = padded_len(35 * 2).unwrap() as usize;

    let read = |bytes: &[u8], rows: &[Range<usize>], view: bool| {
        let mut counted = CountedReader::open(bytes);
        let mut image = counted.reader.image(0).unwrap().with_checksums();
        let mut words = Vec::new();
        for row in rows {
            let ranges = [0..7, row.clone()];
            if view {
                image.view(&ranges, &mut words).unwrap();
            } else {
                image.read(&ranges).unwrap();
            }
        }
        let report = image.finish().unwrap();
        (report, counted.fetched())
    };
    let in_order = [0..2, 2..3, 3..5];
    for view in [false, true] {
        let (report, fetched) = read(&bytes, &in_order, view);
        assert_eq!(report.datasum, ChecksumStatus::Valid, "view {view}");
        assert_eq!(report.checksum, ChecksumStatus::Valid, "view {view}");
        assert_eq!(
            fetched, padded,
            "view {view}: the decode and its finish fetch the unit once"
        );
    }

    // The whole image in one summed section is the image written, and its sum is valid.
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut whole = reader.image(0).unwrap().with_checksums();
    assert_eq!(whole.read(&[0..7, 0..5]).unwrap().samples, image.samples);
    assert_eq!(whole.finish().unwrap().datasum, ChecksumStatus::Valid);

    // A whole-image read feeds the padded unit it views, so the finish reads nothing more;
    // a handle that read nothing reads the unit at its finish, as `verify_checksum` does.
    for read_first in [true, false] {
        let mut counted = CountedReader::open(&bytes);
        let mut handle = counted.reader.image(0).unwrap().with_checksums();
        if read_first {
            assert_eq!(handle.read(Region::All).unwrap().samples, image.samples);
        }
        assert_eq!(handle.finish().unwrap().checksum, ChecksumStatus::Valid);
        assert_eq!(counted.fetched(), padded, "read first: {read_first}");
    }

    // Rows of 7 samples are 14 bytes. Rows 3 and 4 come first and are not fed; rows 0 to 2 then
    // continue the sum to byte 42, and the finish reads the unit from there.
    let (report, fetched) = read(&bytes, &[3..5, 0..2, 2..3], true);
    assert_eq!(report.datasum, ChecksumStatus::Valid);
    assert_eq!(
        fetched,
        70 + (padded - 42),
        "out of order, the finish reads from the first gap"
    );

    let mut corrupt = bytes.clone();
    let data_start = bytes.len() - padded;
    corrupt[data_start + 11] ^= 0x40;
    let (report, _) = read(&corrupt, &in_order, true);
    assert_eq!(report.datasum, ChecksumStatus::Invalid);
    let mut reader = FitsReader::from_bytes(&corrupt).unwrap();
    assert_eq!(reader.verify_checksum(0).unwrap(), report);
}

/// A compressed image's sections are decoded from tiles, which feed nothing, so the finish reads
/// the whole unit and gives `verify_checksum`'s report: valid, and the image the sections read.
#[cfg(feature = "compression")]
#[test]
fn a_compressed_image_summed_through_its_sections_reads_the_unit_at_the_finish() {
    let image = Image {
        shape: vec![9, 7],
        samples: Data::I16((0..63).map(|value| value * 97 - 3000).collect()),
        scaling: Scaling::IDENTITY,
    };
    let bytes = written_by(memory_writer().with_checksums(), |w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP).with_tiles([4, 3]),
            None,
        )
    });
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut sections = reader.image(1).unwrap().with_checksums();
    let mut words = Vec::new();
    for rows in [0..4, 4..7] {
        sections.view(&[0..9, rows], &mut words).unwrap();
    }
    let report = sections.finish().unwrap();
    assert_eq!(report.datasum, ChecksumStatus::Valid);
    assert_eq!(report.checksum, ChecksumStatus::Valid);
    assert_eq!(reader.verify_checksum(1).unwrap(), report);

    // A whole read fetches the container's stored bytes and feeds them, so the finish reads
    // only the block fill after them: the unit, once.
    let mut counted = CountedReader::open(&bytes);
    let mut whole = counted.reader.image(1).unwrap().with_checksums();
    assert_eq!(whole.read(Region::All).unwrap().samples, image.samples);
    assert_eq!(whole.finish().unwrap(), report);
    let padded = usize::try_from(counted.reader.hdus()[1].padded_data_bytes()).unwrap();
    assert_eq!(counted.fetched(), padded);
}

/// A partial read of a tiled-compressed table with no arrays fetches only the tiles its
/// rows touch: row 9 lies in tile 1 of tiles of 7 rows, and its read fetches what a
/// selection of container row 1 fetches; a read of every row fetches the container.
#[cfg(feature = "compression")]
#[test]
fn a_partial_read_of_a_compressed_table_fetches_its_tiles() {
    let table = Builder::with_columns(50, mixed_columns(50)).unwrap();
    let bytes = written(|w| {
        w.write_compressed_table(
            &table,
            &CompressionConfig::new(Compression::GZIP).with_tiles([7]),
            None,
        )
    });
    let fetched = |container_rows: bool, rows: Range<usize>| {
        let mut counted = CountedReader::open(&bytes);
        if container_rows {
            counted.reader.read_container_rows(1, rows).unwrap();
        } else {
            counted.reader.read_table_rows(1, rows).unwrap();
        }
        (counted.fetched(), counted.reader.hdus()[1].data_bytes)
    };
    let (part, container) = fetched(false, 9..10);
    assert_eq!(part, fetched(true, 1..2).0);
    assert!((part as u64) < container);
    assert_eq!(fetched(false, 0..50).0 as u64, container);
}
