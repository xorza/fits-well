use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::image_data::ImageData;
use crate::data::scaling::Scaling;
use crate::error::FitsError;
use crate::error::Indexed;
use crate::error::Ranked;
use crate::header::Header;
use crate::reader::internals::open_fixture;
use crate::reader::*;
use crate::table_impl::character_field::CharacterField;
use crate::table_impl::column_data::ColumnData;
use crate::table_impl::descriptor;
use crate::writer::FitsWriter;
use crate::writer::table::{TableBuilder, WriteColumn};
use num_complex::Complex;
use std::cell::{Cell, RefCell};
use std::io::{self, Cursor, Read, Seek, SeekFrom};
use std::ops::Range;
use std::rc::Rc;

#[derive(Debug)]
struct CountingCursor {
    inner: Cursor<Vec<u8>>,
    bytes_read: Rc<Cell<usize>>,
    read_ranges: Rc<RefCell<Vec<Range<usize>>>>,
}

impl Read for CountingCursor {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let start = self.inner.position() as usize;
        let count = self.inner.read(buffer)?;
        self.bytes_read.set(self.bytes_read.get() + count);
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
        &[("COORD", coordinates, shape.as_str())],
    );
}

fn write_tab_lookup_columns(
    writer: &mut FitsWriter<Cursor<Vec<u8>>>,
    version: i64,
    level: i64,
    columns: &[(&str, &[f64], &str)],
) {
    let mut header = Header::new();
    let row_len = columns
        .iter()
        .map(|(_, values, _)| values.len() * 8)
        .sum::<usize>();
    header
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", row_len as i64)
        .set_internal("NAXIS2", 1)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", columns.len() as i64)
        .set_internal("EXTNAME", "WCS-TABLE")
        .set_internal("EXTVER", version)
        .set_internal("EXTLEVEL", level);
    let mut bytes = Vec::with_capacity(row_len);
    for (index, (name, values, shape)) in columns.iter().enumerate() {
        let column = index + 1;
        header
            .set_internal(format!("TTYPE{column}").as_str(), *name)
            .set_internal(
                format!("TFORM{column}").as_str(),
                format!("{}D", values.len()),
            )
            .set_internal(format!("TDIM{column}").as_str(), *shape);
        bytes.extend(values.iter().flat_map(|value| value.to_be_bytes()));
    }
    writer.write_raw_hdu(&header, &bytes).unwrap();
}

#[test]
fn read_wcs_resolves_the_exact_tabular_extension() {
    let mut primary = Header::new();
    primary
        .set_internal("SIMPLE", true)
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 1)
        .set_internal("NAXIS1", 1)
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
    let mut decoy = Header::new();
    decoy
        .set_internal("XTENSION", "BINTABLE")
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 2)
        .set_internal("NAXIS1", 0)
        .set_internal("NAXIS2", 0)
        .set_internal("PCOUNT", 0)
        .set_internal("GCOUNT", 1)
        .set_internal("TFIELDS", 0)
        .set_internal("EXTNAME", "OTHER")
        .set_internal("EXTVER", "not-an-integer");
    writer.write_raw_hdu(&decoy, &[]).unwrap();
    write_tab_lookup(&mut writer, 2, 1, &[100.0, 200.0, 400.0]);
    write_tab_lookup(&mut writer, 2, 3, &[10.0, 20.0, 40.0]);

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        reader.hdus[0].header.wcs(None).unwrap().pixel_to_world(&[2.0]),
        Err(FitsError::UnsupportedWcsTransform { axes }) if axes == [0]
    ));
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(wcs.pixel_to_world(&[2.0]).unwrap(), [20.0]);
    assert_eq!(wcs.world_to_pixel(&[30.0]).unwrap(), [2.5]);
}

#[test]
fn read_wcs_resolves_shared_arrays_and_extensions_once_per_reference_group() {
    let mut primary = Header::new();
    primary
        .set_internal("SIMPLE", true)
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 0)
        .set_internal("WCSAXES", 5);
    for (axis, column, version, table_axis) in [
        (1, "FIRST", 1, 1),
        (2, "SECOND", 1, 1),
        (3, "COUPLED", 1, 1),
        (4, "COUPLED", 1, 2),
        (5, "DISTINCT", 2, 1),
    ] {
        primary
            .set_internal(format!("CTYPE{axis}").as_str(), format!("AX{axis:02}-TAB"))
            .set_internal(format!("CRPIX{axis}").as_str(), 0.0)
            .set_internal(format!("CRVAL{axis}").as_str(), 0.0)
            .set_internal(format!("CDELT{axis}").as_str(), 1.0)
            .set_internal(format!("PS{axis}_0").as_str(), "WCS-TABLE")
            .set_internal(format!("PV{axis}_1").as_str(), version)
            .set_internal(format!("PS{axis}_1").as_str(), column)
            .set_internal(format!("PV{axis}_3").as_str(), table_axis);
    }
    let coupled = [30.0, 40.0, 31.0, 40.0, 30.0, 42.0, 31.0, 42.0];
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[]).unwrap();
    write_tab_lookup_columns(
        &mut writer,
        1,
        1,
        &[
            ("FIRST", &[10.0, 20.0], "(1,2)"),
            ("SECOND", &[100.0, 200.0], "(1,2)"),
            ("COUPLED", &coupled, "(2,2,2)"),
        ],
    );
    write_tab_lookup_columns(
        &mut writer,
        2,
        1,
        &[("DISTINCT", &[1000.0, 2000.0], "(1,2)")],
    );

    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(
        wcs.pixel_to_world(&[1.0, 1.0, 1.0, 1.0, 1.0]).unwrap(),
        [10.0, 100.0, 30.0, 40.0, 1000.0]
    );
}

#[test]
fn read_wcs_fetches_only_the_referenced_first_row_heap_cells() {
    let mut primary = Header::new();
    primary
        .set_internal("SIMPLE", true)
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 1)
        .set_internal("NAXIS1", 1)
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
    let coordinates = (0..row_count)
        .map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]))
        .collect();
    let indices = (0..row_count)
        .map(|_| ColumnData::F64(vec![10.0, 20.0, 40.0]))
        .collect();
    let unused = (0..row_count)
        .map(|_| ColumnData::Bytes(vec![7; 1024]))
        .collect();
    let table = TableBuilder::explicit(
        row_count,
        vec![
            WriteColumn::vla("COORD", coordinates)
                .unwrap()
                .with_tdim(vec![1, 3]),
            WriteColumn::vla("INDEX", indices).unwrap(),
            WriteColumn::vla("UNUSED", unused).unwrap(),
        ],
    )
    .unwrap();
    let mut lookup_header = Header::new();
    lookup_header.set_internal("EXTNAME", "WCS-TABLE");
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&primary, &[0]).unwrap();
    writer
        .write_table_with_header(&table, &lookup_header)
        .unwrap();

    let bytes_read = Rc::new(Cell::new(0));
    let source = CountingCursor {
        inner: Cursor::new(writer.into_inner().into_inner()),
        bytes_read: Rc::clone(&bytes_read),
        read_ranges: Rc::new(RefCell::new(Vec::new())),
    };
    let mut reader = FitsReader::open(source).unwrap();
    let before = bytes_read.get();
    let wcs = reader.read_wcs(0, None).unwrap();
    assert_eq!(bytes_read.get() - before, 24 + 6 * size_of::<f64>());
    assert_eq!(wcs.pixel_to_world(&[2.0]).unwrap(), [20.0]);
}

#[test]
fn reads_a_single_hdu_image_with_exact_boundaries() {
    let f = open_fixture("UITfuv2582gc.fits");
    assert_eq!(f.hdus.len(), 1);
    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.bitpix().unwrap(), Bitpix::I16);
    assert_eq!(p.header.axes().unwrap(), vec![512, 512]);
    assert_eq!(p.data_offset, 11_520);
    assert_eq!(padded_len(p.data_bytes), 527_040);
    let bytes = std::fs::read("tests/data/fits/UITfuv2582gc.fits").unwrap();
    assert_eq!(
        p.header_sum,
        checksum::accumulate(&bytes[..p.data_offset as usize], 0)
    );
}

#[test]
fn read_data_raw_is_stable_across_reads() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    let a = f.read_data_raw(0).unwrap();
    let b = f.read_data_raw(0).unwrap();
    assert_eq!(
        a.data(),
        b.data(),
        "repeated raw reads yield identical data"
    );
    assert_eq!(
        a.bytes.len(),
        padded_len(f.hdus[0].data_bytes) as usize,
        "owned buffer is the full block-padded unit"
    );
}

#[cfg(feature = "mmap")]
#[test]
fn mmap_read_matches_seeking_read() {
    let path = "tests/data/fits/UITfuv2582gc.fits";
    let mut seek = open_fixture("UITfuv2582gc.fits");
    let want = seek.read_image(0).unwrap();
    let want_shape = want.shape.clone();
    let want_samples = want.decode(); // own, releasing the borrow on `seek`

    let mut m = FitsReader::open_mmap(path).unwrap();
    assert_eq!(m.hdus.len(), 1);
    let got = m.read_image(0).unwrap();
    assert_eq!(got.shape, want_shape);
    assert_eq!(
        got.decode(),
        want_samples,
        "mmap decode matches the seeking read"
    );
}

#[test]
fn read_image_reuses_internal_scratch_across_reads() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    let raw1 = f.read_image(0).unwrap();
    let shape1 = raw1.shape.clone();
    let data1 = raw1.decode(); // own the samples, releasing the borrow on `f`
    // The reader staged the raw unit through its internal scratch, which now holds
    // the full block-padded data unit and is reused (not reallocated) on the next
    // read — only each `decode()` freshly allocates.
    assert_eq!(
        f.scratch.len(),
        padded_len(f.hdus[0].data_bytes) as usize,
        "scratch holds the padded data unit after a read"
    );
    let cap = f.scratch.capacity();
    let raw2 = f.read_image(0).unwrap();
    let shape2 = raw2.shape.clone();
    let data2 = raw2.decode();
    assert_eq!(shape1, shape2);
    assert_eq!(data1, data2);
    assert_eq!(
        f.scratch.capacity(),
        cap,
        "internal scratch reused across image reads, not reallocated"
    );
}

#[test]
fn reads_random_groups_primary_plus_bintable_extension() {
    let f = open_fixture("DDTSUVDATA.fits");
    assert_eq!(f.hdus.len(), 2);

    let g = &f.hdus[0];
    assert_eq!(g.kind, HduKind::RandomGroups);
    assert_eq!(g.header.bitpix().unwrap(), Bitpix::F32);
    assert_eq!(g.header.axes().unwrap(), vec![0, 3, 4, 1, 1, 1]);
    assert_eq!(g.data_offset, 14_400);
    assert_eq!(padded_len(g.data_bytes), 573_120);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 593_280);
    assert_eq!(padded_len(t.data_bytes), 2_880);

    let bytes = std::fs::read("tests/data/fits/DDTSUVDATA.fits").unwrap();
    assert_eq!(
        g.header_sum,
        checksum::accumulate(&bytes[..g.data_offset as usize], 0)
    );
    let second_header = g.data_offset + padded_len(g.data_bytes);
    assert_eq!(
        t.header_sum,
        checksum::accumulate(&bytes[second_header as usize..t.data_offset as usize], 0,)
    );
}

#[test]
fn reads_dataless_primary_then_bintable() {
    let f = open_fixture("IUElwp25637mxlo.fits");
    assert_eq!(f.hdus.len(), 2);

    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.naxis().unwrap(), 0);
    assert_eq!(p.data_offset, 28_800);
    assert_eq!(padded_len(p.data_bytes), 0);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 34_560);
    assert_eq!(padded_len(t.data_bytes), 14_400);
}

#[test]
fn trailing_special_records_and_partial_blocks_are_ignored() {
    use crate::block::BLOCK_SIZE;
    use std::io::Cursor;
    // A valid single-HDU file, then §3.5 special records / §3.6 trailing fill and
    // partial blocks appended — none carrying an `END`. The reader must still find
    // exactly the one real HDU and not error on the trailing bytes.
    let mut bytes = std::fs::read("tests/data/fits/UITfuv2582gc.fits").unwrap();
    bytes.extend(std::iter::repeat_n(0u8, BLOCK_SIZE)); // trailing all-zero fill block
    bytes.extend(std::iter::repeat_n(b'x', BLOCK_SIZE)); // a special record (no END)
    bytes.extend_from_slice(b"a truncated tail"); // sub-block partial remnant
    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus.len(), 1);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);

    // A special block may legally contain a canonical END-shaped card later in
    // the block. Its first card is not XTENSION, so none of it is an HDU header.
    let mut bytes = std::fs::read("tests/data/fits/UITfuv2582gc.fits").unwrap();
    bytes.extend_from_slice(&fits_file(
        &["COMMENT special records", "BITPIX  = 8", "NAXIS   = 0"],
        &[],
    ));
    let f = FitsReader::open(Cursor::new(bytes)).unwrap();
    assert_eq!(f.hdus.len(), 1);
    assert_eq!(f.hdus[0].kind, HduKind::Primary);
}

/// Assemble an in-memory FITS file from card strings + a raw data unit, both
/// block-padded (header with spaces, data with NUL).
fn fits_file(cards: &[&str], data: &[u8]) -> Vec<u8> {
    use crate::block::BLOCK_SIZE;
    let mut buf = Vec::new();
    let mut push_card = |text: &str| {
        let mut card = [b' '; 80];
        card[..text.len()].copy_from_slice(text.as_bytes());
        buf.extend_from_slice(&card);
    };
    for c in cards {
        push_card(c);
    }
    push_card("END");
    while buf.len() % BLOCK_SIZE != 0 {
        buf.push(b' ');
    }
    buf.extend_from_slice(data);
    while buf.len() % BLOCK_SIZE != 0 {
        buf.push(0);
    }
    buf
}

#[test]
fn multi_block_header_scan_keeps_geometric_spare_capacity() {
    use crate::block::BLOCK_SIZE;
    use crate::block::CARD_SIZE;

    let mut cards = vec!["COMMENT"; 2 * BLOCK_SIZE / CARD_SIZE];
    cards[0] = "SIMPLE  = T";
    let bytes = fits_file(&cards, &[]);
    let mut source = SliceSource::new(&bytes);
    let mut offset = 0;
    let mut scratch = Vec::new();
    let NextHeader::Found { bytes, .. } =
        scan_header_unit(&mut source, &mut offset, &mut scratch, HduPosition::Primary).unwrap()
    else {
        panic!("expected a complete header");
    };
    assert_eq!(bytes.len(), 3 * BLOCK_SIZE);
    assert!(
        bytes.capacity() > bytes.len(),
        "multi-block scanning should grow geometrically"
    );
}

#[test]
fn malformed_image_pcount_is_rejected_not_panicked() {
    use std::io::Cursor;
    // A primary array with PCOUNT=5 is non-conforming (§4.3). `data_extent` sizes
    // (10+5) bytes, so the old `assert_eq!` would panic; now it is a clean error.
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
    assert!(matches!(r.read_image(0), Err(FitsError::ImageHasGroups)));
}

#[test]
fn data_unit_larger_than_the_file_is_rejected_not_allocated() {
    use std::io::Cursor;
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
    assert!(matches!(r.read_image(0), Err(FitsError::UnexpectedEof)));
    assert!(matches!(r.read_table(0), Err(FitsError::NotABinTable)));
    assert!(matches!(
        r.read_ascii_table(0),
        Err(FitsError::NotAnAsciiTable)
    ));
}

#[test]
fn content_before_any_valid_hdu_is_rejected() {
    use crate::block::BLOCK_SIZE;
    use std::io::Cursor;
    // Garbage with no `END` and no preceding HDU is not a FITS file.
    let bytes = vec![b'x'; BLOCK_SIZE + 17];
    assert!(matches!(
        FitsReader::open(Cursor::new(bytes)),
        Err(FitsError::MissingKeyword { name: "SIMPLE" })
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
        Err(FitsError::MissingKeyword { name: "SIMPLE" })
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
        Err(FitsError::KeywordOutOfRange { name: "NAXIS1" })
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
    assert_eq!(reader.hdus.len(), 2);
    assert_eq!(reader.hdus[0].kind, HduKind::Primary);
    assert_eq!(reader.hdus[0].data_offset, crate::block::BLOCK_SIZE as u64);
    assert_eq!(reader.hdus[0].data_bytes, 3);
    assert_eq!(reader.hdus[1].kind, HduKind::Image);
    assert_eq!(
        reader.hdus[1].data_offset,
        (3 * crate::block::BLOCK_SIZE) as u64
    );
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
        Err(FitsError::MissingKeyword { name: "PCOUNT" })
    ));
}

#[test]
fn last_data_unit_ends_exactly_at_end_of_file() {
    for name in [
        "UITfuv2582gc.fits",
        "DDTSUVDATA.fits",
        "IUElwp25637mxlo.fits",
    ] {
        let f = open_fixture(name);
        let last = f.hdus.last().unwrap();
        let file_len = std::fs::metadata(format!("tests/data/fits/{name}"))
            .unwrap()
            .len();
        assert_eq!(
            last.data_offset + padded_len(last.data_bytes),
            file_len,
            "{name}"
        );
    }
}

#[test]
fn read_data_raw_returns_padded_bytes_and_the_data_range() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    let unit = f.read_data_raw(0).unwrap();
    // 512×512 i16: 524_288 bytes of data, padded up to 527_040 on disk.
    let view = unit.view();
    assert_eq!(view.padded.len(), 527_040);
    assert_eq!(view.data_range, 0..524_288);
    assert_eq!(unit.data().len(), 524_288);
    // The padding past the data range is block fill, not samples.
    assert!(view.padded[524_288..].iter().all(|&b| b == 0));

    let mut detached = unit.view();
    detached.data_range = usize::MAX..usize::MAX;
    detached.padded = &[];
    assert_eq!(detached.data_range, usize::MAX..usize::MAX);
    assert!(detached.padded.is_empty());
    assert_eq!(unit.data().len(), 524_288);
    assert_eq!(unit.clone().into_data().len(), 524_288);
    assert_eq!(unit.into_padded().len(), 527_040);
}

#[test]
fn read_data_raw_rejects_out_of_bounds_index() {
    let mut f = open_fixture("UITfuv2582gc.fits"); // a single-HDU file
    assert!(matches!(
        f.read_data_raw(5),
        Err(FitsError::IndexOutOfBounds {
            indexed: Indexed::Hdu,
            index: 5,
            len: 1
        })
    ));
}

#[test]
fn read_image_decodes_the_primary_array_shape_and_type() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    let raw = f.read_image(0).unwrap();
    assert_eq!(raw.shape, vec![512, 512]);
    assert_eq!(raw.bitpix(), Bitpix::I16);
    assert_eq!(raw.physical().len(), 512 * 512);
    assert_eq!(raw.decode().len(), 512 * 512);
}

#[test]
fn read_image_raw_samples_match_a_manual_big_endian_decode() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    // Independently decode the first few pixels straight from the data bytes.
    let unit = f.read_data_raw(0).unwrap();
    let manual: Vec<i16> = unit.data()[..8]
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();
    let img = f.read_image(0).unwrap();
    match img.decode() {
        ImageData::I16(v) => assert_eq!(&v[..4], manual.as_slice()),
        other => panic!("expected I16 samples, got {other:?}"),
    }
}

#[test]
fn read_image_rejects_non_image_hdus() {
    // hdu[0] is random groups, hdu[1] is a binary table — neither is an image.
    let mut f = open_fixture("DDTSUVDATA.fits");
    assert!(matches!(f.read_image(0), Err(FitsError::NotAnImage)));
    assert!(matches!(f.read_image(1), Err(FitsError::NotAnImage)));
}

#[test]
fn hdu_index_finds_extensions_by_extname() {
    let f = open_fixture("DDTSUVDATA.fits");
    // hdu 1 is the AIPS antenna table, EXTNAME = 'AIPS AN' (trailing spaces trimmed).
    assert_eq!(f.hdu_index("AIPS AN", None).unwrap(), Some(1));
    assert_eq!(f.hdu_index("aips an", None).unwrap(), Some(1)); // case-insensitive
    assert_eq!(f.hdu_index("AIPS AN", Some(99)).unwrap(), None); // no such EXTVER
    assert_eq!(f.hdu_index("MISSING", None).unwrap(), None);
    // A tiled-compressed image extension is found by its EXTNAME too.
    assert_eq!(
        open_fixture("comp_gzip_i16.fits")
            .hdu_index("COMPRESSED_IMAGE", None)
            .unwrap(),
        Some(1)
    );

    let mut header = Header::new();
    header
        .set_internal("SIMPLE", true)
        .set_internal("BITPIX", 8)
        .set_internal("NAXIS", 0)
        .set_internal("EXTNAME", 7);
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_raw_hdu(&header, &[]).unwrap();
    let bytes = writer.into_inner().into_inner();
    let malformed = FitsReader::from_bytes(&bytes).unwrap();
    assert!(matches!(
        malformed.hdu_index("SCI", None),
        Err(FitsError::TypeMismatch { name, expected })
            if name == "EXTNAME" && expected == "text"
    ));
}

#[test]
fn image_indices_lists_readable_images_including_compressed() {
    // A single primary array image.
    assert_eq!(open_fixture("UITfuv2582gc.fits").image_indices(), vec![0]);
    // Empty primary + a tiled-compressed image extension (classified by ZIMAGE),
    // so only HDU 1 is an image — the `NAXIS = 0` primary is skipped.
    assert_eq!(open_fixture("comp_gzip_i16.fits").image_indices(), vec![1]);
    // Random-groups primary + plain bintable: no images at all.
    assert!(open_fixture("DDTSUVDATA.fits").image_indices().is_empty());
}

fn write_to_vec(image: &Image) -> Vec<u8> {
    let mut w = FitsWriter::new(Cursor::new(Vec::new()));
    w.write_image(image).unwrap();
    w.into_inner().into_inner()
}

#[test]
fn read_image_borrows_u8_samples_with_zero_copy() {
    let image = Image {
        shape: vec![4],
        samples: ImageData::U8(vec![10, 20, 30, 40]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let buf = write_to_vec(&image);

    let mut reader = FitsReader::from_bytes(&buf).unwrap();
    let raw = reader.read_image(0).unwrap();
    assert_eq!(raw.shape, vec![4]);
    assert_eq!(raw.bitpix(), Bitpix::U8);
    // U8 needs no byte-swap, so `.u8()` hands back the stored bytes directly.
    let view = raw.u8().expect("a U8 image has a zero-copy u8 view");
    assert_eq!(view, &[10, 20, 30, 40]);
    // Prove it is a borrow into the source buffer, not a copy: the view's address
    // lies within `buf`.
    let base = buf.as_ptr() as usize;
    let view_ptr = view.as_ptr() as usize;
    assert!(
        (base..base + buf.len()).contains(&view_ptr),
        "the u8 view must point inside the source buffer (zero-copy)"
    );
}

#[test]
fn read_image_exposes_big_endian_bytes_for_multibyte_types() {
    let image = Image {
        shape: vec![3],
        samples: ImageData::I16(vec![1, -2, 300]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let buf = write_to_vec(&image);

    let mut reader = FitsReader::from_bytes(&buf).unwrap();
    let raw = reader.read_image(0).unwrap();
    // A type that needs byte-swapping has no zero-copy typed view.
    assert_eq!(raw.u8(), None);
    // The raw bytes are big-endian: 1 → 0x0001, -2 → 0xFFFE, 300 → 0x012C.
    assert_eq!(raw.raw_bytes(), Some(&[0, 1, 255, 254, 1, 44][..]));
    // `decode()` swaps them into the same host-endian samples it borrowed.
    assert_eq!(raw.decode(), ImageData::I16(vec![1, -2, 300]));
}

#[test]
fn read_image_view_matches_decode_for_a_plain_image() {
    let mut f = open_fixture("UITfuv2582gc.fits");
    // The owned decode is the reference; the borrowed view (into a caller scratch)
    // must equal it.
    let expected_scaling = f.hdus()[0].header.scaling().unwrap();
    let owned = f.read_image(0).unwrap().decode();
    let mut scratch = Vec::new();
    let image = f.read_image_view(0, &mut scratch).unwrap();
    assert_eq!(image.metadata().shape, &[512, 512]);
    assert_eq!(image.metadata().scaling, expected_scaling);
    match (image.samples, &owned) {
        (ImageView::I16(v), ImageData::I16(o)) => assert_eq!(v, o.as_slice()),
        (v, o) => panic!("expected matching I16 view/decode, got {v:?} / {o:?}"),
    }
}

#[test]
fn read_image_view_borrows_u8_samples_with_zero_copy() {
    let image = Image {
        shape: vec![4],
        samples: ImageData::U8(vec![10, 20, 30, 40]),
        scaling: Scaling {
            bscale: 1.0,
            bzero: 0.0,
            blank: None,
        },
    };
    let buf = write_to_vec(&image);
    let mut reader = FitsReader::from_bytes(&buf).unwrap();
    let mut scratch = Vec::new();
    let image = reader.read_image_view(0, &mut scratch).unwrap();
    assert_eq!(image.metadata().shape, &[4]);
    let ImageView::U8(v) = image.samples else {
        panic!("a U8 image must view as U8");
    };
    assert_eq!(v, &[10, 20, 30, 40]);
    // U8 needs no swap, so the view borrows the source buffer directly — the caller's
    // scratch stays untouched (empty).
    let base = buf.as_ptr() as usize;
    assert!(
        (base..base + buf.len()).contains(&(v.as_ptr() as usize)),
        "the u8 view must point inside the source buffer (zero-copy)"
    );
    assert!(scratch.is_empty(), "a U8 view must not touch the scratch");
}

#[test]
#[cfg(feature = "compression")]
fn read_image_view_matches_decode_for_a_compressed_image() {
    let mut f = open_fixture("comp_gzip_i16.fits");
    let owned = f.read_image(1).unwrap().decode();
    let mut scratch = Vec::with_capacity(owned.len().div_ceil(4));
    let scratch_ptr = scratch.as_ptr() as *const i16;
    let image = f.read_image_view(1, &mut scratch).unwrap();
    assert_eq!(image.metadata().shape, &[24, 16]);
    match (image.samples, &owned) {
        (ImageView::I16(v), ImageData::I16(o)) => {
            assert_eq!(v, o.as_slice());
            assert_eq!(v.as_ptr(), scratch_ptr);
        }
        (v, o) => panic!("expected matching I16 view/decode, got {v:?} / {o:?}"),
    }
}

#[test]
fn hdu_index_selects_by_case_insensitive_name_and_version() {
    let primary = Image::new(vec![1], vec![0u8]).unwrap();
    let extension = Image::new(vec![3], vec![10i16, 20, 30]).unwrap();
    let mut extension_header = Header::new();
    extension_header.set("EXTNAME", "SCI").unwrap();
    extension_header.set("EXTVER", 2).unwrap();
    extension_header.set("OBJECT", "target").unwrap();

    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_image(&primary).unwrap();
    writer
        .write_image_with_header(&extension, &extension_header)
        .unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();

    let index = reader.hdu_index("sci", Some(2)).unwrap().unwrap();
    assert_eq!(index, 1);
    assert_eq!(reader.hdus()[index].kind, HduKind::Image);
    assert_eq!(
        reader.hdus()[index].header.get_text("OBJECT").unwrap(),
        Some("target")
    );
    assert_eq!(
        reader.read_image(index).unwrap().decode(),
        ImageData::I16(vec![10, 20, 30])
    );
    assert_eq!(reader.hdus()[0].kind, HduKind::Primary);
    assert_eq!(reader.hdu_index("SCI", Some(3)).unwrap(), None);
}

fn region_indices() -> Vec<usize> {
    let mut indices = Vec::new();
    for z in 1..3 {
        for y in 1..3 {
            for x in 1..4 {
                indices.push(x + 5 * y + 20 * z);
            }
        }
    }
    indices
}

#[cfg(feature = "compression")]
fn select_2d_section(
    samples: &ImageData,
    width: usize,
    x: Range<usize>,
    y: Range<usize>,
) -> ImageData {
    let indices: Vec<usize> = y
        .flat_map(|row| x.clone().map(move |column| column + width * row))
        .collect();
    match samples {
        ImageData::U8(values) => ImageData::U8(indices.iter().map(|&i| values[i]).collect()),
        ImageData::I16(values) => ImageData::I16(indices.iter().map(|&i| values[i]).collect()),
        ImageData::I32(values) => ImageData::I32(indices.iter().map(|&i| values[i]).collect()),
        ImageData::I64(values) => ImageData::I64(indices.iter().map(|&i| values[i]).collect()),
        ImageData::F32(values) => ImageData::F32(indices.iter().map(|&i| values[i]).collect()),
        ImageData::F64(values) => ImageData::F64(indices.iter().map(|&i| values[i]).collect()),
    }
}

#[cfg(feature = "compression")]
fn assert_image_data_exact(actual: &ImageData, expected: &ImageData, label: &str) {
    match (actual, expected) {
        (ImageData::U8(actual), ImageData::U8(expected)) => {
            assert_eq!(actual, expected, "{label}")
        }
        (ImageData::I16(actual), ImageData::I16(expected)) => {
            assert_eq!(actual, expected, "{label}")
        }
        (ImageData::I32(actual), ImageData::I32(expected)) => {
            assert_eq!(actual, expected, "{label}")
        }
        (ImageData::I64(actual), ImageData::I64(expected)) => {
            assert_eq!(actual, expected, "{label}")
        }
        (ImageData::F32(actual), ImageData::F32(expected)) => {
            assert_eq!(actual.len(), expected.len(), "{label}");
            for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual.to_bits(),
                    expected.to_bits(),
                    "{label} pixel {index}"
                );
            }
        }
        (ImageData::F64(actual), ImageData::F64(expected)) => {
            assert_eq!(actual.len(), expected.len(), "{label}");
            for (index, (&actual, &expected)) in actual.iter().zip(expected).enumerate() {
                assert_eq!(
                    actual.to_bits(),
                    expected.to_bits(),
                    "{label} pixel {index}"
                );
            }
        }
        _ => panic!(
            "{label}: sample types differ (actual {:?}, expected {:?})",
            actual.bitpix(),
            expected.bitpix()
        ),
    }
}

#[test]
fn image_sections_match_hand_computed_values_for_every_bitpix() {
    let all: Vec<usize> = (0..60).collect();
    let selected = region_indices();
    let cases = [
        (
            ImageData::U8(all.iter().map(|&value| value as u8).collect()),
            ImageData::U8(selected.iter().map(|&value| value as u8).collect()),
        ),
        (
            ImageData::I16(all.iter().map(|&value| value as i16 - 30).collect()),
            ImageData::I16(selected.iter().map(|&value| value as i16 - 30).collect()),
        ),
        (
            ImageData::I32(all.iter().map(|&value| value as i32 * 1000 - 7).collect()),
            ImageData::I32(
                selected
                    .iter()
                    .map(|&value| value as i32 * 1000 - 7)
                    .collect(),
            ),
        ),
        (
            ImageData::I64(all.iter().map(|&value| value as i64 * -50).collect()),
            ImageData::I64(selected.iter().map(|&value| value as i64 * -50).collect()),
        ),
        (
            ImageData::F32(all.iter().map(|&value| value as f32 * 0.5).collect()),
            ImageData::F32(selected.iter().map(|&value| value as f32 * 0.5).collect()),
        ),
        (
            ImageData::F64(all.iter().map(|&value| value as f64 * -0.25).collect()),
            ImageData::F64(selected.iter().map(|&value| value as f64 * -0.25).collect()),
        ),
    ];

    for (samples, expected) in cases {
        let bitpix = samples.bitpix();
        let image = Image::new(vec![5, 4, 3], samples).unwrap();
        let bytes = write_to_vec(&image);
        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        let mut scratch = Vec::new();
        let section = reader
            .read_image_section_view(0, &[1..4, 1..3, 1..3], &mut scratch)
            .unwrap();
        assert_eq!(section.shape, [3, 2, 2], "{bitpix:?}");
        assert_eq!(section.scaling, Scaling::IDENTITY, "{bitpix:?}");
        assert_eq!(section.samples.to_owned_data(), expected, "{bitpix:?}");
    }
}

#[test]
fn image_sections_preserve_scaling_and_validate_empty_and_invalid_regions() {
    let image = Image::new_scaled(
        vec![4, 3],
        vec![1i16, 2, -99, 4, 5, 6, 7, 8, 9, 10, 11, 12],
        Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank: Some(-99),
        },
    )
    .unwrap();
    let bytes = write_to_vec(&image);
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let section = reader.read_image_section(0, &[1..4, 0..2]).unwrap();
    assert_eq!(section.metadata().shape, [3, 2]);
    assert_eq!(section.metadata().scaling, image.metadata().scaling);
    let physical = section.physical();
    assert_eq!(physical[0], 14.0);
    assert!(physical[1].is_nan());
    assert_eq!(physical[2..], [18.0, 22.0, 24.0, 26.0]);

    let empty = reader.read_image_section(0, &[2..2, 1..3]).unwrap();
    assert_eq!(empty.metadata().shape, [0, 2]);
    assert!(empty.stored().is_empty());
    let wrong_rank = 0..1;
    assert!(matches!(
        reader.read_image_section(0, std::slice::from_ref(&wrong_rank)),
        Err(FitsError::RankMismatch {
            ranked: Ranked::ImageRegion,
            expected: 2,
            got: 1,
        })
    ));
    assert!(matches!(
        reader.read_image_section(0, &[0..5, 0..1]),
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
        .map(|index| index as i16 - 100)
        .collect();
    let image = Image::new(shape.to_vec(), samples.clone()).unwrap();
    let bytes = write_to_vec(&image);
    let bytes_read = Rc::new(Cell::new(0));
    let read_ranges = Rc::new(RefCell::new(Vec::new()));
    let source = CountingCursor {
        inner: Cursor::new(bytes.clone()),
        bytes_read,
        read_ranges: Rc::clone(&read_ranges),
    };
    let mut stream = FitsReader::open(source).unwrap();
    let data_offset = stream.hdus[0].data_offset as usize;
    read_ranges.borrow_mut().clear();
    let ranges = [1..3, 1..4, 0..4, 1..3];
    let section = stream.read_image_section(0, &ranges).unwrap();

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
    assert_eq!(section.metadata().shape, [2, 3, 4, 2]);
    assert_eq!(section.stored(), ImageView::I16(&expected_values));
    assert_eq!(&*read_ranges.borrow(), &expected_reads);

    let mut slice = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        slice.read_image_section(0, &ranges).unwrap().stored(),
        ImageView::I16(&expected_values)
    );
}

#[cfg(feature = "compression")]
#[test]
fn compressed_image_sections_cross_tile_boundaries_and_match_the_whole_image() {
    use crate::compress::{Compression, CompressionOptions};
    use crate::reader::source::SliceSource;
    use crate::reader::source::internals::CountingSource;

    let count = 9usize * 7;
    let mut f32_values: Vec<f32> = (0..count).map(|index| index as f32 * 0.5).collect();
    f32_values[13] = f32::from_bits(0x7FC0_1234);
    let mut f64_values: Vec<f64> = (0..count).map(|index| index as f64 * -0.25).collect();
    f64_values[13] = f64::from_bits(0x7FF8_0000_0000_1234);
    let cases = [
        (
            ImageData::U8((0..count).map(|index| index as u8).collect()),
            Some(13),
        ),
        (
            ImageData::I16((0..count).map(|index| index as i16 - 30).collect()),
            Some(-17),
        ),
        (
            ImageData::I32((0..count).map(|index| index as i32 * 1000 - 7).collect()),
            Some(12_993),
        ),
        (
            ImageData::I64((0..count).map(|index| index as i64 * -50).collect()),
            Some(-650),
        ),
        (ImageData::F32(f32_values), None),
        (ImageData::F64(f64_values), None),
    ];

    for (samples, blank) in cases {
        let bitpix = samples.bitpix();
        let scaling = Scaling {
            bscale: 2.0,
            bzero: 10.0,
            blank,
        };
        let image = Image::new_scaled(vec![9, 7], samples, scaling).unwrap();
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        writer
            .write_compressed_image(
                &image,
                Compression::GZIP,
                &CompressionOptions::tiled([4, 3]),
            )
            .unwrap();
        let bytes = writer.into_inner().into_inner();
        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        let whole = reader.read_image(1).unwrap().decode();
        let expected = select_2d_section(&whole, 9, 2..9, 1..7);
        let owned = reader.read_image_section(1, &[2..9, 1..7]).unwrap();
        assert_eq!(owned.metadata().shape, [7, 6], "{bitpix:?}");
        assert_eq!(owned.metadata().scaling, scaling, "{bitpix:?}");
        assert_image_data_exact(
            &owned.stored().to_owned_data(),
            &expected,
            &format!("{bitpix:?} owned"),
        );
        assert!(owned.physical().iter().any(|value| value.is_nan()));

        let mut words = Vec::new();
        let view = reader
            .read_image_section_view(1, &[2..9, 1..7], &mut words)
            .unwrap();
        assert_eq!(view.shape, [7, 6], "{bitpix:?}");
        assert_eq!(view.scaling, scaling, "{bitpix:?}");
        assert_image_data_exact(
            &view.samples.to_owned_data(),
            &expected,
            &format!("{bitpix:?} view"),
        );

        let empty = reader.read_image_section(1, &[2..2, 1..7]).unwrap();
        assert_eq!(empty.metadata().shape, [0, 6], "{bitpix:?}");
        assert!(empty.stored().is_empty(), "{bitpix:?}");

        if bitpix == Bitpix::U8 {
            for ranges in [[0..2, 0..2], [0..9, 0..7]] {
                let owned_reads = Rc::new(Cell::new(0));
                let source = CountingSource {
                    inner: SliceSource::new(&bytes),
                    owned_reads: Rc::clone(&owned_reads),
                };
                let mut counted = FitsReader::from_source(source).unwrap();
                let mut scratch = Vec::new();
                counted
                    .read_image_section_view(1, &ranges, &mut scratch)
                    .unwrap();
                assert_eq!(owned_reads.get(), 0, "{ranges:?}");
            }
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
        WriteColumn::scalar("BYTE", ColumnData::Bytes(vec![1, 2, 3, 4])),
        WriteColumn::bits(
            "FLAGS",
            vec![0b1010_0000, 0b0101_0000, 0b1110_0000, 0b0001_0000],
            5,
        ),
        WriteColumn::fixed(
            "NAME",
            ColumnData::Character(
                [b"one ", b"two ", b"tri ", b"four"]
                    .into_iter()
                    .map(|value| CharacterField::new(value.to_vec()))
                    .collect(),
            ),
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
            .scaled(2.0, 10.0)
            .with_null(-99),
        WriteColumn::scalar("LONG", ColumnData::I64(vec![100, 200, 300, 400])),
        WriteColumn::scalar("FLOAT", ColumnData::F32(vec![1.5, 2.5, 3.5, 4.5])),
        WriteColumn::scalar("DOUBLE", ColumnData::F64(vec![5.5, 6.5, 7.5, 8.5])),
        WriteColumn::fixed("ZERO", ColumnData::I16(Vec::new()), 0),
        WriteColumn::vla(
            "VLA",
            vec![
                ColumnData::I32(vec![1]),
                ColumnData::I32(vec![2, 3]),
                ColumnData::I32(Vec::new()),
                ColumnData::I32(vec![4, 5, 6]),
            ],
        )
        .unwrap(),
        WriteColumn::vla(
            "QVLA",
            vec![
                ColumnData::F64(vec![0.5]),
                ColumnData::F64(Vec::new()),
                ColumnData::F64(vec![2.5, 3.5]),
                ColumnData::F64(vec![4.5]),
            ],
        )
        .unwrap()
        .wide()
        .unwrap(),
    ];
    let table = TableBuilder::explicit(rows, columns).unwrap();
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    writer.write_table(&table).unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();

    let schema = reader.table_schema(1).unwrap();
    assert_eq!(schema.nrows, 4);
    assert_eq!(schema.columns.len(), 14);
    let empty = reader.read_table_rows(1, 2..2).unwrap();
    assert_eq!(empty.metadata().nrows, 0);
    assert_eq!(
        empty.column_by_name("VLA").unwrap().vla().unwrap(),
        Vec::<ColumnData>::new()
    );
    let full = reader.read_table_rows(1, 0..4).unwrap();
    assert_eq!(
        full.column_by_name("QVLA").unwrap().vla().unwrap(),
        [
            ColumnData::F64(vec![0.5]),
            ColumnData::F64(Vec::new()),
            ColumnData::F64(vec![2.5, 3.5]),
            ColumnData::F64(vec![4.5]),
        ]
    );
    let ranged = reader.read_table_rows(1, 1..3).unwrap();
    assert_eq!(
        ranged.column_by_name("ID").unwrap().raw().unwrap(),
        ColumnData::I32(vec![20, 30])
    );
    let flags = ranged.column_by_name("FLAGS").unwrap().bits().unwrap();
    assert_eq!(
        (0..5)
            .map(|column| flags.get(0, column).unwrap())
            .collect::<Vec<_>>(),
        [false, true, false, true, false]
    );
    assert_eq!(
        ranged.column_by_name("NAME").unwrap().raw().unwrap(),
        ColumnData::Character(vec![
            CharacterField::new(b"two ".to_vec()),
            CharacterField::new(b"tri ".to_vec()),
        ])
    );
    assert_eq!(
        ranged.column_by_name("COMPLEX").unwrap().complex().unwrap(),
        [Complex::new(2.0, -2.0), Complex::new(3.0, -3.0)]
    );
    let physical = ranged.column_by_name("SCALED").unwrap().physical().unwrap();
    assert!(physical[0].is_nan());
    assert_eq!(physical[1], 14.0);
    assert_eq!(
        ranged.column_by_name("VLA").unwrap().vla().unwrap(),
        [ColumnData::I32(vec![2, 3]), ColumnData::I32(Vec::new())]
    );
    assert_eq!(
        ranged.column_by_name("QVLA").unwrap().vla().unwrap(),
        [ColumnData::F64(Vec::new()), ColumnData::F64(vec![2.5, 3.5])]
    );

    let selection = reader
        .read_table_columns(1, 1..3, &[ColumnSelector::from("ID"), "VLA".into()])
        .unwrap();
    assert_eq!(selection.rows, 1..3);
    assert!(matches!(
        &selection.columns[0].data,
        TableColumnData::Fixed(ColumnData::I32(values)) if values == &[20, 30]
    ));
    assert!(matches!(
        &selection.columns[1].data,
        TableColumnData::Variable(values)
            if values == &[ColumnData::I32(vec![2, 3]), ColumnData::I32(Vec::new())]
    ));
    assert_eq!(
        reader
            .read_table_cell(1, 3, ColumnSelector::from("QVLA"))
            .unwrap(),
        ColumnData::F64(vec![4.5])
    );
    for (column, expected) in [
        ("ID", ColumnData::I32(vec![30])),
        ("LOGICAL", ColumnData::Logical(vec![None])),
        ("BYTE", ColumnData::Bytes(vec![3])),
        ("FLAGS", ColumnData::Bytes(vec![0b1110_0000])),
        (
            "NAME",
            ColumnData::Character(vec![CharacterField::new(b"tri ".to_vec())]),
        ),
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
            reader
                .read_table_cell(1, 2, ColumnSelector::from(column))
                .unwrap(),
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

    let mut corrupted = bytes.clone();
    let qvla = &schema.columns[schema
        .columns
        .iter()
        .position(|column| column.name.as_deref() == Some("QVLA"))
        .unwrap()];
    let qvla_slot =
        usize::try_from(reader.hdus[1].data_offset).unwrap() + schema.row_len + qvla.byte_offset;
    corrupted[qvla_slot..qvla_slot + qvla.tform.byte_width()].fill(0xff);
    let mut selective = FitsReader::from_bytes(&corrupted).unwrap();
    let selection = selective
        .read_table_columns(1, 1..2, &[ColumnSelector::from("ID"), "VLA".into()])
        .unwrap();
    assert!(matches!(
        &selection.columns[0].data,
        TableColumnData::Fixed(ColumnData::I32(values)) if values == &[20]
    ));
    assert!(matches!(
        &selection.columns[1].data,
        TableColumnData::Variable(values) if values == &[ColumnData::I32(vec![2, 3])]
    ));
    assert_eq!(
        selective
            .read_table_cell(1, 1, ColumnSelector::from("ID"))
            .unwrap(),
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
        one.column_by_name("VLA").unwrap().vla().unwrap(),
        [ColumnData::I32(vec![4, 5, 6])]
    );
    assert_eq!(
        stream
            .read_table_columns(1, 0..4, &[ColumnSelector::from("QVLA")])
            .unwrap()
            .columns[0]
            .data,
        TableColumnData::Variable(vec![
            ColumnData::F64(vec![0.5]),
            ColumnData::F64(Vec::new()),
            ColumnData::F64(vec![2.5, 3.5]),
            ColumnData::F64(vec![4.5]),
        ])
    );
}

#[test]
fn malformed_pq_descriptors_match_across_table_read_paths() {
    for wide in [false, true] {
        let mut column = WriteColumn::vla("VLA", vec![ColumnData::Bytes(vec![7])]).unwrap();
        if wide {
            column = column.wide().unwrap();
        }
        let table = TableBuilder::explicit(1, vec![column]).unwrap();
        let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
        writer.write_table(&table).unwrap();
        let bytes = writer.into_inner().into_inner();
        let base_reader = FitsReader::from_bytes(&bytes).unwrap();
        let schema = base_reader.table_schema(1).unwrap();
        let slot = usize::try_from(base_reader.hdus[1].data_offset).unwrap()
            + schema.columns[0].byte_offset;

        for case in descriptor::internals::malformed_descriptor_cases()
            .into_iter()
            .filter(|case| case.wide == wide)
        {
            let mut corrupted = bytes.clone();
            corrupted[slot..slot + case.bytes.len()].copy_from_slice(&case.bytes);
            let mut reader = FitsReader::from_bytes(&corrupted).unwrap();

            let whole = reader.read_table(1).unwrap();
            case.assert_error(whole.column_by_name("VLA").unwrap().vla().unwrap_err());
            case.assert_error(
                reader
                    .read_table_cell(1, 0, ColumnSelector::from("VLA"))
                    .unwrap_err(),
            );
            case.assert_error(reader.read_table_rows(1, 0..1).unwrap_err());
            case.assert_error(
                reader
                    .read_table_columns(1, 0..1, &[ColumnSelector::from("VLA")])
                    .unwrap_err(),
            );
        }
    }
}

#[test]
fn readers_recover_their_original_sources() {
    let image = Image::new(vec![2], vec![1u8, 2]).unwrap();
    let bytes = write_to_vec(&image);
    let slice = FitsReader::from_bytes(&bytes).unwrap().into_bytes();
    assert!(std::ptr::eq(slice.as_ptr(), bytes.as_ptr()));
    assert_eq!(slice.len(), bytes.len());

    let cursor = FitsReader::open(Cursor::new(bytes.clone()))
        .unwrap()
        .into_inner();
    assert_eq!(cursor.into_inner(), bytes);
}
