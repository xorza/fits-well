//! A data checksum summed from the bytes the reads fetch.

use crate::reader::tests::*;

/// A data checksum fed by a decode reading a BITPIX=16 image front to back, in sections whose
/// runs split words, gives the report `verify_checksum` gives — valid, then invalid once a stored
/// byte changes — and the decode with its finish fetch the data unit once, block fill included,
/// where the decode and a separate verification fetch it twice. Sections read out of order feed
/// only what continues the sum, and the finish reads the rest. The owned and the borrowed section
/// reads feed the sum alike.
#[test]
fn data_checksum_summed_during_the_decode_reads_the_unit_once() {
    let image = Image::new(
        vec![7, 5],
        Data::I16((0..35).map(|value| value * 731 - 9000).collect()),
        Scaling::IDENTITY,
    )
    .unwrap();
    let bytes = memory::written_by(memory::memory_writer().with_checksums(), |w| {
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
        assert_eq!(report, cards::VALID, "view {view}");
        assert_eq!(
            fetched, padded,
            "view {view}: the decode and its finish fetch the unit once"
        );
    }

    // The whole image in one summed section is the image written, and its sum is valid.
    let mut reader = bytes.slice();
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

/// A compressed image's checksum sums its tile index when the sum begins, and then each
/// tile's heap cells as a section fetches them: sections that take the tile rows in order,
/// tiles 3 rows high, leave the finish only the block fill, so the unit is read once, and
/// a section that cuts a tile row fetches that row's cells again but sums them once. A
/// whole read fetches the heap after the index. Each gives `verify_checksum`'s report:
/// valid, and the image the reads read.
#[cfg(feature = "compression")]
#[test]
fn compressed_image_summed_through_its_reads_reads_its_unit_once() {
    let image = Image::new(
        vec![9, 7],
        Data::I16((0..63).map(|value| value * 97 - 3000).collect()),
        Scaling::IDENTITY,
    )
    .unwrap();
    let bytes = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        w.write_compressed_image(
            image.view(),
            &CompressionConfig::new(Compression::GZIP).with_tiles([4, 3]),
            None,
        )
    });
    let mut reader = bytes.slice();
    let report = reader.verify_checksum(1).unwrap();
    assert_eq!(report, cards::VALID);
    let padded = usize::try_from(reader.hdus()[1].padded_data_bytes()).unwrap();

    let mut words = Vec::new();
    for sections in [&[0..3, 3..6, 6..7][..], &[0..4, 4..7]] {
        let mut counted = CountedReader::open(&bytes);
        let mut image = counted.reader.image(1).unwrap().with_checksums();
        for rows in sections {
            image.view(&[0..9, rows.clone()], &mut words).unwrap();
        }
        assert_eq!(image.finish().unwrap(), report, "{sections:?}");
        if sections.len() == 3 {
            assert_eq!(counted.fetched(), padded);
        }
    }

    let mut counted = CountedReader::open(&bytes);
    let mut whole = counted.reader.image(1).unwrap().with_checksums();
    assert_eq!(whole.read(Region::All).unwrap().samples, image.samples);
    assert_eq!(whole.finish().unwrap(), report);
    assert_eq!(counted.fetched(), padded);
}
