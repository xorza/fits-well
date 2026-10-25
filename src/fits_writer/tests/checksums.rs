//! The checksums a writer signs, and the reader's judgement of them.

use crate::fits_writer::tests::*;

/// A checksummed HDU verifies, from a stream and from a slice. A flipped data byte fails
/// both sums, and a flipped byte of the header's blank fill fails only the whole-HDU one.
/// An unsigned HDU declares neither sum.
#[test]
fn checksums_verify_and_tell_which_part_a_flipped_byte_is_in() {
    let image = Image::new(vec![2, 2], Data::I16(vec![1, 2, 3, 4]), Scaling::IDENTITY).unwrap();
    let unsigned = memory::written(|w| w.write_image(image.view(), None));
    assert_eq!(
        unsigned.stream().verify_checksum(0).unwrap(),
        cards::report(ChecksumStatus::Absent, ChecksumStatus::Absent)
    );
    let written = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    let fill = (cards::card_index(&written[..BLOCK_SIZE], "END") + 1) * CARD_SIZE;
    let both = cards::report(ChecksumStatus::Invalid, ChecksumStatus::Invalid);
    let whole_hdu = cards::report(ChecksumStatus::Valid, ChecksumStatus::Invalid);
    for (flipped, expected) in [
        (None, VALID),
        (Some(BLOCK_SIZE), both),
        (Some(fill), whole_hdu),
    ] {
        let mut bytes = written.clone();
        if let Some(at) = flipped {
            bytes[at] ^= 1;
        }
        let stream = bytes.stream().verify_checksum(0).unwrap();
        let slice = FitsReader::from_bytes(&bytes)
            .unwrap()
            .verify_checksum(0)
            .unwrap();
        assert_eq!([stream, slice], [expected; 2], "byte {flipped:?}");
    }
}

/// An image of more samples than one encode chunk is summed in a first pass and written
/// in a second, chunk by chunk: `CHUNK_SAMPLES + 3` samples take two chunks, the second of
/// 3 samples, and the signed file verifies and reads back each sample.
#[test]
fn checksummed_image_of_several_chunks_verifies() {
    let len = CHUNK_SAMPLES + 3;
    let samples: Vec<i16> = (0..len).map(|i| (i % 65_536) as u16 as i16).collect();
    let image = Image::new(vec![len], samples.clone(), Scaling::IDENTITY).unwrap();
    let file = memory::written_by(memory::memory_writer().with_checksums(), |w| {
        w.write_image(image.view(), None)
    });
    let mut reader = file.stream();
    assert_eq!(reader.verify_checksum(0).unwrap(), VALID);
    assert_eq!(
        memory::read_whole(&mut reader, 0).samples,
        Data::I16(samples)
    );
}
