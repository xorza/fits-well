use super::*;
use crate::bitpix::Bitpix;
use crate::data::Image;
use crate::data::ImageData;
use crate::reader::source::StreamSource;
use crate::writer::FitsWriter;
use std::fs::File;
use std::io::Cursor;

fn open(name: &str) -> FitsReader<StreamSource<File>> {
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
    assert_eq!(padded_len(p.data_bytes), 527_040);
}

#[test]
fn read_data_raw_is_stable_across_reads() {
    let mut f = open("UITfuv2582gc.fits");
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
    let mut seek = open("UITfuv2582gc.fits");
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
    let mut f = open("UITfuv2582gc.fits");
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
    let f = open("DDTSUVDATA.fits");
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
}

#[test]
fn reads_dataless_primary_then_bintable() {
    let f = open("IUElwp25637mxlo.fits");
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
        assert_eq!(
            last.data_offset + padded_len(last.data_bytes),
            file_len,
            "{name}"
        );
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
    let raw = f.read_image(0).unwrap();
    assert_eq!(raw.shape, vec![512, 512]);
    assert_eq!(raw.bitpix, Bitpix::I16);
    assert_eq!(raw.physical().len(), 512 * 512);
    assert_eq!(raw.decode().len(), 512 * 512);
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
    match img.decode() {
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

#[test]
fn hdu_index_finds_extensions_by_extname() {
    let f = open("DDTSUVDATA.fits");
    // hdu 1 is the AIPS antenna table, EXTNAME = 'AIPS AN' (trailing spaces trimmed).
    assert_eq!(f.hdu_index("AIPS AN", None), Some(1));
    assert_eq!(f.hdu_index("aips an", None), Some(1)); // case-insensitive
    assert_eq!(f.hdu_index("AIPS AN", Some(99)), None); // no such EXTVER
    assert_eq!(f.hdu_index("MISSING", None), None);
    // A tiled-compressed image extension is found by its EXTNAME too.
    assert_eq!(
        open("comp_gzip_i16.fits").hdu_index("COMPRESSED_IMAGE", None),
        Some(1)
    );
}

#[test]
fn image_indices_lists_readable_images_including_compressed() {
    // A single primary array image.
    assert_eq!(open("UITfuv2582gc.fits").image_indices(), vec![0]);
    // Empty primary + a tiled-compressed image extension (classified by ZIMAGE),
    // so only HDU 1 is an image — the `NAXIS = 0` primary is skipped.
    assert_eq!(open("comp_gzip_i16.fits").image_indices(), vec![1]);
    // Random-groups primary + plain bintable: no images at all.
    assert!(open("DDTSUVDATA.fits").image_indices().is_empty());
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
    assert_eq!(raw.bitpix, Bitpix::U8);
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
