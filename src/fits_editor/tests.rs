use std::io::Cursor;

use crate::bintable::column_data::ColumnData;
use crate::block::BLOCK_SIZE;
use crate::error::FitsError;
use crate::fits_editor::FitsEditor;
use crate::fits_writer::FitsWriter;
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::header_model::{Header, keywords};
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::reader::FitsReader;
use crate::reader::internals::read_whole;

/// A primary image, a binary table and an image extension, written by `writer`.
fn three_hdus(writer: FitsWriter<Cursor<Vec<u8>>>) -> Vec<u8> {
    let mut writer = writer;
    let primary = Image::new(vec![4, 3], (0..12).collect::<Vec<i16>>(), Scaling::IDENTITY).unwrap();
    writer.write_image(primary.view(), None).unwrap();
    let table = Builder::with_columns(
        3,
        vec![WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3]))],
    )
    .unwrap();
    writer.write_table(&table, None).unwrap();
    let last = Image::new(vec![2, 2], vec![1.5f32, -2.0, 3.25, 4.0], Scaling::IDENTITY).unwrap();
    writer.write_image(last.view(), None).unwrap();
    writer.into_inner().into_inner()
}

fn plain_file() -> Vec<u8> {
    three_hdus(FitsWriter::new(Cursor::new(Vec::new())))
}

/// The bytes of `file` outside `range`.
fn outside(file: &[u8], range: std::ops::Range<usize>) -> Vec<u8> {
    [&file[..range.start], &file[range.end..]].concat()
}

/// A header replaced in place by one of the same and of fewer records keeps the file's
/// length, its data and every byte outside the header; one that needs another block is
/// `HeaderDoesNotFit` with the counts, and a different data unit `DataSizeMismatch`.
#[test]
fn replace_header_writes_in_place() {
    let original = plain_file();
    let mut editor = FitsEditor::open(Cursor::new(original.clone())).unwrap();
    let hdu = &editor.hdus()[1];
    let range =
        usize::try_from(hdu.header_offset).unwrap()..usize::try_from(hdu.data_offset).unwrap();
    let mut header = hdu.header.clone();
    header.set("OBSERVER", "Hubble").unwrap();
    editor.replace_header(1, &header).unwrap();
    let mut fewer = header.clone();
    fewer.remove("OBSERVER").unwrap();
    fewer.remove(keywords::TTYPE.indexed(1).unwrap()).unwrap();
    editor.replace_header(1, &fewer).unwrap();
    assert_eq!(editor.hdus()[1].header.get("TTYPE1").unwrap(), None);
    editor.replace_header(1, &header).unwrap();
    assert_eq!(
        editor.hdus()[1].header.get_text("OBSERVER").unwrap(),
        Some("Hubble")
    );
    let edited = editor.into_inner().into_inner();
    assert_eq!(edited.len(), original.len());
    assert_eq!(outside(&edited, range.clone()), outside(&original, range));

    let mut editor = FitsEditor::open(Cursor::new(original.clone())).unwrap();
    let mut grown = editor.hdus()[1].header.clone();
    let records = grown.len();
    // 36 records fill a block, `END` included: `records + 36 − records` more need two.
    for _ in 0..36 - records {
        grown
            .push_comment("a comment for each record the block holds")
            .unwrap();
    }
    assert!(matches!(
        editor.replace_header(1, &grown),
        Err(FitsError::HeaderDoesNotFit {
            needed: 2,
            available: 1
        })
    ));
    let mut resized = editor.hdus()[1].header.clone();
    resized.set(keywords::NAXIS.indexed(2).unwrap(), 4).unwrap();
    assert!(matches!(
        editor.replace_header(1, &resized),
        Err(FitsError::DataSizeMismatch {
            expected: 12,
            got: 16
        })
    ));
    assert_eq!(
        editor.into_inner().into_inner(),
        original,
        "a refused edit writes nothing"
    );
}

/// `with_spare_records(40)` gives each header room for 40 more records: a primary of
/// `n` records takes `⌈(n + 40 + 1)/36⌉` blocks, `END` included, and its header grown by
/// 40 records fits in place, read back and replaced again.
#[test]
fn spare_records_leave_room_for_an_edit() {
    let file = three_hdus(FitsWriter::new(Cursor::new(Vec::new())).with_spare_records(40));
    let mut editor = FitsEditor::open(Cursor::new(file)).unwrap();
    let primary = &editor.hdus()[0];
    let records = primary.header.len();
    assert_eq!(
        primary.header_blocks(),
        u64::try_from((records + 40 + 1).div_ceil(36)).unwrap()
    );
    let mut grown = primary.header.clone();
    for n in 0..40 {
        grown.push_history(&format!("step {n}")).unwrap();
    }
    editor.replace_header(0, &grown).unwrap();
    let reread = editor.hdus()[0].header.clone();
    assert_eq!(reread.history().count(), 40);
    editor.replace_header(0, &reread).unwrap();
    let bytes = editor.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        read_whole(&mut reader, 0).samples,
        Data::I16((0..12).collect())
    );
}

/// A header that grows past its blocks writes the whole file to a second sink, every
/// other HDU and every data unit as they were; the file itself is left as it was.
#[test]
fn replace_header_into_writes_a_whole_file() {
    let original = plain_file();
    let mut editor = FitsEditor::open(Cursor::new(original.clone())).unwrap();
    let mut grown = editor.hdus()[1].header.clone();
    for n in 0..60 {
        grown.push_comment(&format!("line {n}")).unwrap();
    }
    let mut copy = Vec::new();
    editor.replace_header_into(1, &grown, &mut copy).unwrap();
    assert_eq!(editor.into_inner().into_inner(), original);

    let old = FitsReader::from_bytes(&original).unwrap();
    let mut new = FitsReader::from_bytes(&copy).unwrap();
    assert_eq!(new.hdus().len(), 3);
    assert_eq!(new.hdus()[1].header_blocks(), 2);
    assert_eq!(new.hdus()[1].header.comments().count(), 60);
    let primary_end = usize::try_from(old.hdus()[1].header_offset).unwrap();
    assert_eq!(copy[..primary_end], original[..primary_end]);
    let old_tail = usize::try_from(old.hdus()[1].data_offset).unwrap();
    let new_tail = usize::try_from(new.hdus()[1].data_offset).unwrap();
    assert_eq!(copy[new_tail..], original[old_tail..]);
    assert_eq!(
        read_whole(&mut new, 2).samples,
        Data::F32(vec![1.5, -2.0, 3.25, 4.0])
    );
}

/// A replaced header that holds `CHECKSUM` gets a valid one: from its `DATASUM`, or
/// from the data unit when it holds none or a blank one. The editor's HDU records
/// match a scan of the edited file.
#[test]
fn replace_header_keeps_the_checksum_valid() {
    let file = three_hdus(FitsWriter::new(Cursor::new(Vec::new())).with_checksums());
    let mut editor = FitsEditor::open(Cursor::new(file)).unwrap();
    let mut header = editor.hdus()[2].header.clone();
    header.set("OBSERVER", "Hubble").unwrap();
    editor.replace_header(2, &header).unwrap();
    let mut without_datasum = editor.hdus()[1].header.clone();
    without_datasum.remove(keywords::DATASUM).unwrap();
    without_datasum.set("OBSERVER", "Leavitt").unwrap();
    editor.replace_header(1, &without_datasum).unwrap();
    let mut blank_datasum = editor.hdus()[0].header.clone();
    blank_datasum.set(keywords::DATASUM, "  ").unwrap();
    editor.replace_header(0, &blank_datasum).unwrap();
    let records: Vec<_> = editor
        .hdus()
        .iter()
        .map(|hdu| (hdu.header_sum, hdu.header_offset, hdu.data_offset))
        .collect();
    let bytes = editor.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    for hdu in 0..3 {
        assert!(reader.verify_checksum(hdu).unwrap().is_valid(), "HDU {hdu}");
    }
    let scanned: Vec<_> = reader
        .hdus()
        .iter()
        .map(|hdu| (hdu.header_sum, hdu.header_offset, hdu.data_offset))
        .collect();
    assert_eq!(records, scanned);
}

/// `append` gives a writer at the end of a file that ends on a block boundary, and
/// refuses one that does not.
#[test]
fn append_adds_an_hdu() {
    let editor = FitsEditor::open(Cursor::new(plain_file())).unwrap();
    let mut writer = editor.append().unwrap();
    let image = Image::new(vec![3], vec![7u8, 8, 9], Scaling::IDENTITY).unwrap();
    writer
        .write_image(image.view(), Some(&Header::new()))
        .unwrap();
    let bytes = writer.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.hdus().len(), 4);
    assert_eq!(
        read_whole(&mut reader, 0).samples,
        Data::I16((0..12).collect())
    );
    assert_eq!(read_whole(&mut reader, 3).samples, Data::U8(vec![7, 8, 9]));

    let mut ragged = plain_file();
    ragged.extend_from_slice(&[0; 10]);
    let len = u64::try_from(ragged.len()).unwrap();
    let editor = FitsEditor::open(Cursor::new(ragged)).unwrap();
    assert!(matches!(editor.append(), Err(FitsError::FileOffBlockGrid { len: got }) if got == len));
    assert_eq!(len % BLOCK_SIZE as u64, 10);
}
