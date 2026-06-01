use super::*;
use crate::bitpix::Bitpix;
use std::fs::File;

fn open(name: &str) -> FitsReader<File> {
    let path = format!("tests/data/fits/{name}");
    FitsReader::open(File::open(&path).unwrap_or_else(|e| panic!("open {path}: {e}")))
        .unwrap_or_else(|e| panic!("parse {name}: {e}"))
}

#[test]
fn reads_a_single_hdu_image_with_exact_boundaries() {
    let f = open("UITfuv2582gc.fits");
    assert_eq!(f.hdus.len(), 1);
    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.bitpix().unwrap(), Bitpix::I16);
    assert_eq!(p.header.axes().unwrap(), vec![512, 512]);
    assert_eq!(p.data_offset, 11_520);
    assert_eq!(p.data_len, 527_040);
}

#[test]
fn reads_random_groups_primary_plus_bintable_extension() {
    let f = open("DDTSUVDATA.fits");
    assert_eq!(f.hdus.len(), 2);

    let g = &f.hdus[0];
    assert_eq!(g.kind, HduKind::RandomGroups);
    assert_eq!(g.header.bitpix().unwrap(), Bitpix::F32);
    assert_eq!(g.header.axes().unwrap(), vec![0, 3, 4, 1, 1, 1]);
    assert_eq!(g.data_offset, 14_400);
    assert_eq!(g.data_len, 573_120);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 593_280);
    assert_eq!(t.data_len, 2_880);
}

#[test]
fn reads_dataless_primary_then_bintable() {
    let f = open("IUElwp25637mxlo.fits");
    assert_eq!(f.hdus.len(), 2);

    let p = &f.hdus[0];
    assert_eq!(p.kind, HduKind::Primary);
    assert_eq!(p.header.naxis().unwrap(), 0);
    assert_eq!(p.data_offset, 28_800);
    assert_eq!(p.data_len, 0);

    let t = &f.hdus[1];
    assert_eq!(t.kind, HduKind::BinTable);
    assert_eq!(t.data_offset, 34_560);
    assert_eq!(t.data_len, 14_400);
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
fn content_before_any_valid_hdu_is_rejected() {
    use crate::block::BLOCK_SIZE;
    use std::io::Cursor;
    // Garbage with no `END` and no preceding HDU is not a FITS file.
    let bytes = vec![b'x'; BLOCK_SIZE + 17];
    assert!(matches!(
        FitsReader::open(Cursor::new(bytes)),
        Err(FitsError::UnexpectedEof)
    ));
}

#[test]
fn last_data_unit_ends_exactly_at_end_of_file() {
    for name in [
        "UITfuv2582gc.fits",
        "DDTSUVDATA.fits",
        "IUElwp25637mxlo.fits",
    ] {
        let f = open(name);
        let last = f.hdus.last().unwrap();
        let file_len = std::fs::metadata(format!("tests/data/fits/{name}"))
            .unwrap()
            .len();
        assert_eq!(last.data_offset + last.data_len, file_len, "{name}");
    }
}

#[test]
fn read_data_raw_returns_padded_bytes_and_the_data_range() {
    let mut f = open("UITfuv2582gc.fits");
    let unit = f.read_data_raw(0).unwrap();
    // 512×512 i16: 524_288 bytes of data, padded up to 527_040 on disk.
    assert_eq!(unit.bytes.len(), 527_040);
    assert_eq!(unit.data_range, 0..524_288);
    assert_eq!(unit.data().len(), 524_288);
    // The padding past the data range is block fill, not samples.
    assert!(unit.bytes[524_288..].iter().all(|&b| b == 0));
}

#[test]
fn read_data_raw_rejects_out_of_bounds_index() {
    let mut f = open("UITfuv2582gc.fits"); // a single-HDU file
    assert!(matches!(
        f.read_data_raw(5),
        Err(FitsError::HduIndexOutOfBounds { index: 5, len: 1 })
    ));
}

#[test]
fn read_image_decodes_the_primary_array_shape_and_type() {
    let mut f = open("UITfuv2582gc.fits");
    let img = f.read_image(0).unwrap();
    assert_eq!(img.shape, vec![512, 512]);
    assert_eq!(img.samples.bitpix(), Bitpix::I16);
    assert_eq!(img.samples.len(), 512 * 512);
    assert_eq!(img.physical().len(), 512 * 512);
}

#[test]
fn read_image_raw_samples_match_a_manual_big_endian_decode() {
    let mut f = open("UITfuv2582gc.fits");
    // Independently decode the first few pixels straight from the data bytes.
    let unit = f.read_data_raw(0).unwrap();
    let manual: Vec<i16> = unit.data()[..8]
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();
    let img = f.read_image(0).unwrap();
    match img.samples {
        ImageData::I16(v) => assert_eq!(&v[..4], manual.as_slice()),
        other => panic!("expected I16 samples, got {other:?}"),
    }
}

#[test]
fn read_image_rejects_non_image_hdus() {
    // hdu[0] is random groups, hdu[1] is a binary table — neither is an image.
    let mut f = open("DDTSUVDATA.fits");
    assert!(matches!(f.read_image(0), Err(FitsError::NotAnImage)));
    assert!(matches!(f.read_image(1), Err(FitsError::NotAnImage)));
}
