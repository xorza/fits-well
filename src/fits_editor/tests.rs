use std::assert_matches;
use std::io::Cursor;

use crate::bintable::column_data::ColumnData;
use crate::block::{BLOCK_SIZE, RECORDS_PER_BLOCK};
use crate::fits_editor::FitsEditor;
use crate::fits_error::{FitsError, SizeUnit};
use crate::fits_writer::builder::{Builder, WriteColumn};
use crate::hdu::hdu_kind::HduKind;
use crate::header_model::commentary_name::CommentaryName;
use crate::header_model::{Header, keywords};
use crate::image_model::Image;
use crate::image_model::data::Data;
use crate::image_model::scaling::Scaling;
use crate::internals::cards;
use crate::internals::memory;
use crate::internals::memory::MemoryWriter;
use crate::reader::FitsReader;

/// A primary image, a binary table and an image extension, written by `writer`, which
/// sets the form.
fn three_hdus(writer: MemoryWriter) -> Vec<u8> {
    let primary = Image::new(vec![4, 3], (0..12).collect::<Vec<i16>>(), Scaling::IDENTITY).unwrap();
    let table = Builder::with_columns(
        3,
        vec![WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3]))],
    )
    .unwrap();
    let last = Image::new(vec![2, 2], vec![1.5f32, -2.0, 3.25, 4.0], Scaling::IDENTITY).unwrap();
    memory::written_by(writer, |w| {
        w.write_image(primary.view(), None)?;
        w.write_table(&table, None)?;
        w.write_image(last.view(), None)
    })
    .into_bytes()
}

fn plain_file() -> Vec<u8> {
    three_hdus(memory::memory_writer())
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
    // The editor selects and lists as the reader does: the primary and the last image
    // extension are the images.
    assert_eq!(editor.hdu(2).unwrap().index, 2);
    assert_eq!(
        editor.images().map(|hdu| hdu.index).collect::<Vec<_>>(),
        [0, 2]
    );
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
    for _ in 0..RECORDS_PER_BLOCK - records {
        grown
            .push_commentary(
                CommentaryName::COMMENT,
                "a comment for each record the block holds",
            )
            .unwrap();
    }
    assert_matches!(
        editor.replace_header(1, &grown),
        Err(FitsError::HeaderDoesNotFit {
            needed: 2,
            available: 1
        })
    );
    let mut resized = editor.hdus()[1].header.clone();
    resized.set(keywords::NAXIS.indexed(2).unwrap(), 4).unwrap();
    assert_matches!(
        editor.replace_header(1, &resized),
        Err(FitsError::DataSizeMismatch {
            unit: SizeUnit::Bytes,
            expected: 12,
            got: 16
        })
    );
    // The same data unit as an ASCII `TABLE` would fill its last block with spaces.
    let mut retyped = editor.hdus()[1].header.clone();
    retyped.set(keywords::XTENSION, "TABLE").unwrap();
    retyped
        .set(keywords::TFORM.indexed(1).unwrap(), "I4")
        .unwrap();
    retyped.set(keywords::TBCOL.indexed(1).unwrap(), 1).unwrap();
    assert_matches!(
        editor.replace_header(1, &retyped),
        Err(FitsError::HduKindChange {
            from: HduKind::BinTable,
            to: HduKind::AsciiTable
        })
    );
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
    let file = three_hdus(memory::memory_writer().with_spare_records(40));
    let mut editor = FitsEditor::open(Cursor::new(file)).unwrap();
    let primary = &editor.hdus()[0];
    let records = primary.header.len();
    assert_eq!(
        primary.header_blocks(),
        (records + 40 + 1).div_ceil(RECORDS_PER_BLOCK)
    );
    let mut grown = primary.header.clone();
    for n in 0..40 {
        grown
            .push_commentary(CommentaryName::HISTORY, &format!("step {n}"))
            .unwrap();
    }
    editor.replace_header(0, &grown).unwrap();
    let reread = editor.hdus()[0].header.clone();
    assert_eq!(reread.commentary(CommentaryName::HISTORY).count(), 40);
    editor.replace_header(0, &reread).unwrap();
    let bytes = editor.into_inner().into_inner();
    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(
        memory::read_whole(&mut reader, 0).samples,
        Data::I16((0..12).collect())
    );
}

/// A header that grows past its blocks writes the whole file to a second sink, every
/// other HDU and every data unit as they were; the file itself is left as it was. The
/// new header keeps the free records of the old: 40 spare records before the edit give
/// 40 after it, and a header 60 records longer takes `⌈(n + 60 + 40 + 1)/36⌉` blocks.
#[test]
fn replace_header_into_writes_a_whole_file() {
    let original = three_hdus(memory::memory_writer().with_spare_records(40));
    let mut editor = FitsEditor::open(Cursor::new(original.clone())).unwrap();
    let mut grown = editor.hdus()[1].header.clone();
    for n in 0..60 {
        grown
            .push_commentary(CommentaryName::COMMENT, &format!("line {n}"))
            .unwrap();
    }
    let mut copy = Vec::new();
    editor.replace_header_into(1, &grown, &mut copy).unwrap();
    assert_eq!(editor.into_inner().into_inner(), original);

    let old = FitsReader::from_bytes(&original).unwrap();
    let mut new = FitsReader::from_bytes(&copy).unwrap();
    assert_eq!(new.hdus().len(), 3);
    let records = old.hdus()[1].header.len();
    assert_eq!(
        new.hdus()[1].header_blocks(),
        (records + 60 + 40 + 1).div_ceil(RECORDS_PER_BLOCK)
    );
    assert_eq!(
        new.hdus()[1]
            .header
            .commentary(CommentaryName::COMMENT)
            .count(),
        60
    );
    let primary_end = usize::try_from(old.hdus()[1].header_offset).unwrap();
    assert_eq!(copy[..primary_end], original[..primary_end]);
    let old_tail = usize::try_from(old.hdus()[1].data_offset).unwrap();
    let new_tail = usize::try_from(new.hdus()[1].data_offset).unwrap();
    assert_eq!(copy[new_tail..], original[old_tail..]);
    assert_eq!(
        memory::read_whole(&mut new, 2).samples,
        Data::F32(vec![1.5, -2.0, 3.25, 4.0])
    );
}

/// A replaced header that holds `CHECKSUM` gets a valid one: from its `DATASUM`, or
/// from the data unit when it holds none, or one that is blank, empty or no sum. The
/// editor's HDU records match a scan of the edited file.
#[test]
fn replace_header_keeps_the_checksum_valid() {
    let file = three_hdus(memory::memory_writer().with_checksums());
    let mut editor = FitsEditor::open(Cursor::new(file)).unwrap();
    let mut header = editor.hdus()[2].header.clone();
    header.set("OBSERVER", "Hubble").unwrap();
    editor.replace_header(2, &header).unwrap();
    let mut without_datasum = editor.hdus()[1].header.clone();
    without_datasum.remove(keywords::DATASUM).unwrap();
    without_datasum.set("OBSERVER", "Leavitt").unwrap();
    editor.replace_header(1, &without_datasum).unwrap();
    for declared in ["  ", "", "not a sum"] {
        let mut header = editor.hdus()[0].header.clone();
        header.set(keywords::DATASUM, declared).unwrap();
        editor.replace_header(0, &header).unwrap();
    }
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
        memory::read_whole(&mut reader, 0).samples,
        Data::I16((0..12).collect())
    );
    assert_eq!(
        memory::read_whole(&mut reader, 3).samples,
        Data::U8(vec![7, 8, 9])
    );

    // A block of zeros after the last HDU stays on the block grid, and a reader stops at
    // it: what follows it would not be read.
    let file = plain_file();
    let hdu_end = u64::try_from(file.len()).unwrap();
    for trailing in [10, BLOCK_SIZE] {
        let mut padded = file.clone();
        padded.resize(file.len() + trailing, 0);
        let file_len = u64::try_from(padded.len()).unwrap();
        let editor = FitsEditor::open(Cursor::new(padded.clone())).unwrap();
        assert_matches!(
            editor.append(),
            Err(FitsError::TrailingContent { hdu_end: end, file_len: len })
                if end == hdu_end && len == file_len
        );
    }
    // A header after the last HDU that does not read ends the HDU list, and the file does
    // not end at the last HDU it holds, so an append is refused as it is for trailing
    // bytes.
    let mut damaged = file.clone();
    damaged.extend_from_slice(&cards::records(&["XTENSION= 'IMAGE   '"]));
    damaged.resize(file.len() + BLOCK_SIZE, b' ');
    let editor = FitsEditor::open(Cursor::new(damaged)).unwrap();
    assert_eq!(editor.hdus().len(), 3);
    assert_matches!(
        editor.append(),
        Err(FitsError::TrailingContent { hdu_end: end, .. }) if end == hdu_end
    );
    let mut truncated = file;
    truncated.truncate(truncated.len() - 1);
    let editor = FitsEditor::open(Cursor::new(truncated)).unwrap();
    assert_matches!(editor.append(), Err(FitsError::UnexpectedEof));
}
