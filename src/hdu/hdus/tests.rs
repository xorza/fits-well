use crate::block::BLOCK_SIZE;
use crate::block::CARD_SIZE;
use crate::block::padded_len;
use crate::checksum;
use crate::fits_error::FitsError;
use crate::fits_error::Indexed;
use crate::fits_error::ValueKind;
use crate::hdu::hdu_kind::HduKind;
use crate::hdu::hdu_kind::HduPosition;
use crate::hdu::hdu_selector::HduSelector;
use crate::hdu::hdus::{NextHeader, Scan};
use crate::header_model::Header;
use crate::header_model::keywords;
use crate::header_model::record_name::RecordName;
use crate::header_model::repair::{Repair, RepairKind};
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::internals::cards;
use crate::internals::fixture::Fixture;
use crate::internals::memory;
use crate::reader::source::SliceSource;
use crate::reader::*;
use std::assert_matches;
use std::io::Cursor;
use std::iter;

#[test]
fn multi_block_header_scan_keeps_geometric_spare_capacity() {
    let mut cards = vec!["COMMENT"; 2 * BLOCK_SIZE / CARD_SIZE];
    cards[0] = "SIMPLE  = T";
    let bytes = cards::fits_file(&cards, &[]);
    let mut source = SliceSource::new(&bytes);
    let mut scan = Scan {
        source: &mut source,
        offset: 0,
    };
    let NextHeader::Found { bytes, .. } = scan.next_header(HduPosition::Primary).unwrap() else {
        panic!("expected a complete header");
    };
    assert_eq!(bytes.len(), 3 * BLOCK_SIZE);
    assert!(
        bytes.capacity() > bytes.len(),
        "multi-block scanning should grow geometrically"
    );
}

/// The HDUs of each fixture, each one's kind, data offset and padded data length: a single
/// image, a random-groups primary then a binary table, and a primary with no data then a
/// binary table. Each header sum covers the bytes from the end of the HDU before to its
/// data, and the last data unit ends at the end of the file.
#[test]
fn each_fixture_scans_to_its_exact_layout() {
    for (fixture, layout) in [
        (Fixture::Uit, &[(HduKind::Primary, 11_520, 527_040)][..]),
        (
            Fixture::Ddts,
            &[
                (HduKind::RandomGroups, 14_400, 573_120),
                (HduKind::BinTable, 593_280, 2_880),
            ],
        ),
        (
            Fixture::Iue,
            &[
                (HduKind::Primary, 28_800, 0),
                (HduKind::BinTable, 34_560, 14_400),
            ],
        ),
    ] {
        let reader = fixture.stream();
        let bytes = fixture.bytes();
        let scanned: Vec<_> = reader
            .hdus()
            .iter()
            .map(|hdu| {
                (
                    hdu.kind,
                    hdu.data_offset,
                    padded_len(hdu.data_bytes).unwrap(),
                )
            })
            .collect();
        assert_eq!(scanned, layout, "{fixture:?}");
        let mut header_start = 0;
        for hdu in reader.hdus() {
            let header =
                usize::try_from(header_start).unwrap()..usize::try_from(hdu.data_offset).unwrap();
            assert_eq!(
                hdu.header_sum,
                checksum::accumulate(&bytes[header], 0),
                "{fixture:?} HDU {}",
                hdu.index
            );
            header_start = hdu.data_offset + padded_len(hdu.data_bytes).unwrap();
        }
        assert_eq!(
            header_start,
            u64::try_from(bytes.len()).unwrap(),
            "{fixture:?}"
        );
    }
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
    bytes.extend_from_slice(&cards::fits_file(
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
fn end_record_with_stray_bytes_ends_the_header() {
    let mut file = cards::records(&[
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

#[test]
fn malformed_image_pcount_is_refused_not_panicked() {
    // A primary array with PCOUNT=5 is non-conforming (§4.3): an error, not a
    // panic in the data-extent arithmetic.
    let bytes = cards::fits_file(
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
    assert_matches!(r.image(0), Err(FitsError::ImageHasGroups));
    // The scan could not resolve the image, and reports why on request.
    assert_matches!(r.hdus()[0].image_metadata(), Err(FitsError::ImageHasGroups));
}

#[test]
fn content_before_any_valid_hdu_is_refused() {
    // Garbage with no `END` and no preceding HDU is not a FITS file.
    let bytes = vec![b'x'; BLOCK_SIZE + 17];
    assert_matches!(
        FitsReader::open(Cursor::new(bytes)),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::SIMPLE)
        })
    );
}

/// A file starts with a primary HDU whose header sizes it: an empty file, an extension
/// first, and a primary without `NAXIS` fail the open.
#[test]
fn file_must_start_with_a_sized_primary_hdu() {
    assert_matches!(
        FitsReader::open(Cursor::new(Vec::<u8>::new())),
        Err(FitsError::UnexpectedEof)
    );

    let extension_only = cards::fits_file(
        &[
            "XTENSION= 'IMAGE   '",
            "BITPIX  = 8",
            "NAXIS   = 0",
            "PCOUNT  = 0",
            "GCOUNT  = 1",
        ],
        &[],
    );
    assert_matches!(
        FitsReader::open(Cursor::new(extension_only)),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::SIMPLE)
        })
    );
    assert_matches!(
        FitsReader::from_bytes(&cards::fits_file(&["SIMPLE  = T", "BITPIX  = 8"], &[])),
        Err(FitsError::MissingKeyword {
            name: RecordName::Keyword(keywords::NAXIS)
        })
    );
}

#[test]
fn role_aware_extents_find_the_exact_next_hdu_boundary() {
    let mut bytes = cards::fits_file(
        &["SIMPLE  = T", "BITPIX  = 8", "NAXIS   = 1", "NAXIS1  = 3"],
        &[1, 2, 3],
    );
    bytes.extend_from_slice(&cards::fits_file(
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
    assert_matches!(
        f.hdu(("AIPS AN", 99)), // no such EXTVER
        Err(FitsError::HduNotFound { name, version: Some(99), level: None }) if name == "AIPS AN"
    );
    // EXTLEVEL is 1 where the card is absent.
    let level = |level| HduSelector::Extension {
        name: "AIPS AN",
        version: None,
        level: Some(level),
    };
    assert_eq!(f.hdu(level(1)).unwrap().index, 1);
    assert_matches!(
        f.hdu(level(2)),
        Err(FitsError::HduNotFound { level: Some(2), .. })
    );
    assert_matches!(
        f.hdu("MISSING"),
        Err(FitsError::HduNotFound { name, version: None, level: None }) if name == "MISSING"
    );
    assert_matches!(
        f.hdu(9),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index: 9,
            len: 2
        })
    );
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
    let bytes = memory::written(|w| w.write_raw_hdu(&header, &[]));
    // An `EXTNAME` that is no text names nothing, not even the primary's default, and
    // fails no lookup.
    let malformed = bytes.slice();
    assert_matches!(
        malformed.hdus()[0].extname(),
        Err(FitsError::ValueTypeMismatch {
            name: RecordName::Keyword(keywords::EXTNAME),
            expected: ValueKind::Text
        })
    );
    assert_matches!(
        malformed.hdu("SCI"),
        Err(FitsError::HduNotFound {
            name,
            version: None,
            level: None
        }) if name == "SCI"
    );
    assert!(!malformed.hdus()[0].matches("PRIMARY").unwrap());

    // A written extension is found by its name in any case and its EXTVER.
    let primary = Image::new(vec![1], vec![0u8], Scaling::IDENTITY).unwrap();
    let extension = Image::new(vec![3], vec![10i16, 20, 30], Scaling::IDENTITY).unwrap();
    let mut extension_header = Header::new();
    extension_header.set(keywords::EXTNAME, "SCI").unwrap();
    extension_header.set(keywords::EXTVER, 2).unwrap();
    extension_header.set(keywords::OBJECT, "target").unwrap();

    let bytes = memory::written(|w| {
        w.write_image(primary.view(), None)?;
        w.write_image(extension.view(), Some(&extension_header))
    });
    let mut reader = bytes.slice();

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
        memory::read_whole(&mut reader, index).samples,
        Data::I16(vec![10, 20, 30])
    );
    let image = reader.image(("SCI", 2)).unwrap().read(..).unwrap();
    assert_eq!(
        (image.shape(), image.samples(), image.scaling()),
        (&[3][..], &Data::I16(vec![10, 20, 30]), Scaling::IDENTITY)
    );
    assert_eq!(reader.hdus()[0].kind, HduKind::Primary);
    assert_matches!(
        reader.hdu(("SCI", 3)),
        Err(FitsError::HduNotFound {
            version: Some(3),
            ..
        })
    );
}
