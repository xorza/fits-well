//! Each feature through the public API alone: a write, then a read back with exact
//! values. The unit tests reach the crate's privates; these show that a caller needs
//! none of them.

use std::io::Cursor;

use bitvec::bitvec;
use bitvec::order::Msb0;
use fits_well::header::{CommentaryName, Header};
use fits_well::image::{Bitpix, Data, Image, Metadata, Scaling, View};
use fits_well::io::ChecksumStatus;
use fits_well::table::{
    AsciiBuilder, AsciiColumnData, AsciiFloatFormat, AsciiText, AsciiWriteColumn, Builder,
    ColumnData, DescriptorWidth, Ragged, SelectedData, WriteColumn,
};
use fits_well::wcs::Wcs;
use fits_well::{FitsEditor, FitsReader, FitsWriter};

/// The bytes `write` writes into a fresh in-memory writer.
fn file(write: impl FnOnce(&mut FitsWriter<Cursor<Vec<u8>>>)) -> Vec<u8> {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()));
    write(&mut writer);
    writer.into_inner().into_inner()
}

/// The 4×3 image whose sample at column x, row y is x + 10y.
fn ramp(scaling: Scaling) -> Image {
    let samples: Vec<i16> = (0..3)
        .flat_map(|y| (0..4).map(move |x| x + 10 * y))
        .collect();
    Image::new(vec![4, 3], samples, scaling).unwrap()
}

/// An image reads back whole, by section and by name, and its physical values apply
/// `BSCALE` and `BZERO`: 2v + 1.
#[test]
fn an_image_reads_back_whole_by_section_and_by_name() {
    let scaled = Scaling {
        bscale: 2.0,
        bzero: 1.0,
        blank: None,
    };
    let mut named = Header::new();
    named.set("EXTNAME", "SCI").unwrap();
    let bytes = file(|writer| {
        writer
            .write_image(ramp(Scaling::IDENTITY).view(), None)
            .unwrap();
        writer
            .write_image(ramp(scaled).view(), Some(&named))
            .unwrap();
    });

    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut image = reader.image(0).unwrap();
    assert_eq!(
        image.read(..).unwrap().samples(),
        ramp(Scaling::IDENTITY).samples()
    );
    // Columns 1 and 2 of rows 1 and 2: 11, 12, 21, 22.
    let section = image.read(&[1..3, 1..3]).unwrap();
    assert_eq!(section.shape(), [2, 2]);
    assert_eq!(section.samples(), &Data::I16(vec![11, 12, 21, 22]));

    let mut image = reader.image("SCI").unwrap();
    assert_eq!(image.metadata().scaling, scaled);
    let mut physical: Vec<f64> = Vec::new();
    image.physical_into(&[0..2, 0..1], &mut physical).unwrap();
    assert_eq!(physical, [1.0, 3.0]);
}

/// A streamed image reads back as the chunks held it, and its checksums are valid.
#[test]
fn a_streamed_image_reads_back_with_valid_checksums() {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new())).with_checksums();
    let layout = Metadata {
        shape: &[4, 3],
        bitpix: Bitpix::I16,
        scaling: Scaling::IDENTITY,
    };
    let mut stream = writer.stream_image(layout, None).unwrap();
    for row in [[0i16, 1, 2, 3], [10, 11, 12, 13], [20, 21, 22, 23]] {
        stream.write_chunk(View::from(row.as_slice())).unwrap();
    }
    stream.finish().unwrap();
    let bytes = writer.into_inner().into_inner();

    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let mut image = reader.image(0).unwrap().with_checksums();
    assert_eq!(
        image.read(..).unwrap().samples(),
        ramp(Scaling::IDENTITY).samples()
    );
    let report = image.finish().unwrap();
    assert_eq!(report.datasum, ChecksumStatus::Valid);
    assert_eq!(report.checksum, ChecksumStatus::Valid);
}

/// Each kind of binary-table column reads back as written, whole, by rows, by columns
/// and by cell.
#[test]
fn each_kind_of_table_column_reads_back() {
    let mut table = Builder::new();
    table
        .push(WriteColumn::scalar("ID", ColumnData::I32(vec![1, 2, 3])))
        .unwrap()
        .push(WriteColumn::characters("NAME", ["Vega", "Sirius", "Rigel"], 8).unwrap())
        .unwrap()
        .push(WriteColumn::fixed(
            "POS",
            ColumnData::F64(vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0]),
            2,
        ))
        .unwrap()
        .push(WriteColumn::vla(
            "FLUXES",
            Ragged::from_rows([
                ColumnData::F32(vec![1.0]),
                ColumnData::F32(vec![2.0, 2.5]),
                ColumnData::F32(vec![]),
            ])
            .unwrap(),
            DescriptorWidth::Q,
        ))
        .unwrap()
        .push(
            WriteColumn::bits(
                "FLAGS",
                [
                    bitvec![u8, Msb0; 1, 0, 1],
                    bitvec![u8, Msb0; 0, 0, 0],
                    bitvec![u8, Msb0; 1, 1, 0],
                ],
                3,
            )
            .unwrap(),
        )
        .unwrap();
    let bytes = file(|writer| writer.write_table(&table, None).unwrap());

    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let read = reader.read_table(1).unwrap();
    assert_eq!(read.schema().row_count(), 3);
    assert_eq!(
        read.column("ID").unwrap().raw().unwrap(),
        ColumnData::I32(vec![1, 2, 3])
    );
    assert_eq!(
        read.column("NAME").unwrap().raw().unwrap(),
        ColumnData::Character(b"Vega    Sirius  Rigel   ".to_vec())
    );
    assert_eq!(
        read.column("POS").unwrap().physical().unwrap(),
        [1.0, 2.0, 3.0, 4.0, 5.0, 6.0]
    );
    let fluxes = read.column("FLUXES").unwrap().vla_physical().unwrap();
    assert_eq!(
        fluxes.iter().collect::<Vec<_>>(),
        [&[1.0][..], &[2.0, 2.5], &[]]
    );
    let flags = read.column("FLAGS").unwrap().bits().unwrap();
    assert_eq!(flags.row(0), bitvec![u8, Msb0; 1, 0, 1]);
    assert_eq!(flags.row(2), bitvec![u8, Msb0; 1, 1, 0]);

    let rows = reader.read_table_rows(1, 1..3).unwrap();
    assert_eq!(
        rows.column("ID").unwrap().raw().unwrap(),
        ColumnData::I32(vec![2, 3])
    );
    let selection = reader
        .read_table_columns(1, 2..3, ["FLUXES", "ID"])
        .unwrap();
    let names: Vec<_> = selection
        .columns
        .iter()
        .map(|column| column.descriptor.name.as_deref())
        .collect();
    assert_eq!(names, [Some("FLUXES"), Some("ID")]);
    assert_eq!(
        selection.column("ID").unwrap().data,
        SelectedData::Fixed(ColumnData::I32(vec![3]))
    );
    assert_eq!(
        reader.read_table_cell(1, 1, "POS").unwrap(),
        ColumnData::F64(vec![3.0, 4.0])
    );
}

/// An ASCII table reads back as written, its nulls as `None` and as NaN.
#[test]
fn an_ascii_table_reads_back_with_its_nulls() {
    let names: AsciiText = [Some("Vega"), None].into_iter().collect();
    let mut table = AsciiBuilder::new();
    table
        .push(AsciiWriteColumn::text("NAME", names.clone(), 8).with_null("-"))
        .unwrap()
        .push(
            AsciiWriteColumn::float(
                "MAG",
                [Some(0.03), None],
                AsciiFloatFormat::F {
                    width: 7,
                    decimals: 2,
                },
            )
            .with_null("NaN"),
        )
        .unwrap();
    let bytes = file(|writer| writer.write_ascii_table(&table, None).unwrap());

    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    let read = reader.read_ascii_table(1).unwrap();
    assert_eq!(
        read.column("NAME").unwrap().raw().unwrap(),
        AsciiColumnData::Text(names)
    );
    assert_eq!(
        read.column("MAG").unwrap().raw().unwrap(),
        AsciiColumnData::Float(vec![Some(0.03), None])
    );
    let physical = read.column("MAG").unwrap().physical().unwrap();
    assert_eq!(physical[0], 0.03);
    assert!(physical[1].is_nan());
}

/// A header replaced in place keeps the data and gets a valid checksum, and an HDU
/// appended after it reads back.
#[test]
fn an_edited_file_keeps_valid_checksums() {
    let mut writer = FitsWriter::new(Cursor::new(Vec::new()))
        .with_checksums()
        .with_spare_records(8);
    writer
        .write_image(ramp(Scaling::IDENTITY).view(), None)
        .unwrap();

    let mut editor = FitsEditor::open(writer.into_inner()).unwrap();
    let mut header = editor.hdu(0).unwrap().header.clone();
    header
        .set("OBJECT", "M31")
        .unwrap()
        .set("HIERARCH ESO DET DIT", 2.5)
        .unwrap()
        .push_commentary(CommentaryName::HISTORY, "edited")
        .unwrap();
    editor.replace_header(0, &header).unwrap();
    let mut writer = editor.append().unwrap().with_checksums();
    writer
        .write_image(ramp(Scaling::IDENTITY).view(), None)
        .unwrap();
    let bytes = writer.into_inner().into_inner();

    let mut reader = FitsReader::from_bytes(&bytes).unwrap();
    assert_eq!(reader.hdus().len(), 2);
    let edited = &reader.hdus()[0].header;
    assert_eq!(edited.get_text("OBJECT").unwrap(), Some("M31"));
    assert_eq!(edited.get_real("HIERARCH ESO DET DIT").unwrap(), Some(2.5));
    assert_eq!(
        edited
            .commentary(CommentaryName::HISTORY)
            .collect::<Vec<_>>(),
        ["edited"]
    );
    for hdu in 0..2 {
        assert!(reader.verify_checksum(hdu).unwrap().is_valid(), "HDU {hdu}");
        assert_eq!(
            reader.image(hdu).unwrap().read(..).unwrap().samples(),
            ramp(Scaling::IDENTITY).samples()
        );
    }
}

/// A WCS written into a header reads back as the same transform, and a crop moves only
/// its reference pixel.
#[test]
fn a_wcs_written_into_a_header_reads_back() {
    let mut header = Header::new();
    for (keyword, value) in [
        ("CRPIX1", 50.5),
        ("CRPIX2", 50.5),
        ("CRVAL1", 83.633),
        ("CRVAL2", 22.0145),
        ("CDELT1", -1.0 / 3600.0),
        ("CDELT2", 1.0 / 3600.0),
    ] {
        header.set(keyword, value).unwrap();
    }
    header
        .set("CTYPE1", "RA---TAN")
        .unwrap()
        .set("CTYPE2", "DEC--TAN")
        .unwrap();
    let wcs = Wcs::from_header(&header, None).unwrap();

    let mut written = Header::new();
    wcs.write_into(&mut written, None).unwrap();
    let reread = Wcs::from_header(&written, None).unwrap();
    let mut world = [0.0; 2];
    let mut reworld = [0.0; 2];
    for pixel in [[1.0, 1.0], [50.5, 50.5], [100.0, 37.0]] {
        wcs.pixel_to_world(&pixel, &mut world).unwrap();
        reread.pixel_to_world(&pixel, &mut reworld).unwrap();
        assert_eq!(world, reworld, "{pixel:?}");
    }

    // The cut-out from column 21 and row 31: its pixel (1, 1) is the full image's (21, 31).
    let cutout = wcs.crop((&[20..40, 30..50]).into()).unwrap();
    assert_eq!(cutout.description().axes[0].crpix, 30.5);
    assert_eq!(cutout.description().axes[1].crpix, 20.5);
    wcs.pixel_to_world(&[21.0, 31.0], &mut world).unwrap();
    cutout.pixel_to_world(&[1.0, 1.0], &mut reworld).unwrap();
    assert_eq!(world, reworld);
}

#[cfg(feature = "compression")]
mod compressed {
    use fits_well::FitsReader;
    use fits_well::compress::{Compression, CompressionConfig, Quantization};
    use fits_well::image::{Data, Image, Scaling};
    use fits_well::table::{Builder, ColumnData, WriteColumn};

    use crate::{file, ramp};

    /// A RICE image reads back whole and by section as the plain one does, and a lossless
    /// float image keeps every bit, a NaN's payload and a negative zero included.
    #[test]
    fn a_compressed_image_reads_as_the_plain_one() {
        let floats = vec![
            1.5f32,
            -0.0,
            f32::from_bits(0x7fc0_1234),
            f32::MIN_POSITIVE / 2.0,
        ];
        let floats = Image::new(vec![2, 2], Data::F32(floats), Scaling::IDENTITY).unwrap();
        let bytes = file(|writer| {
            let rice = CompressionConfig::new(Compression::Rice).with_tiles([4, 1]);
            writer
                .write_compressed_image(ramp(Scaling::IDENTITY).view(), &rice, None)
                .unwrap();
            let lossless = CompressionConfig::new(Compression::GZIP_SHUFFLED)
                .with_quantization(Quantization::NONE);
            writer
                .write_compressed_image(floats.view(), &lossless, None)
                .unwrap();
        });

        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        let indices: Vec<usize> = reader.images().map(|hdu| hdu.index).collect();
        assert_eq!(indices, [1, 2]);
        let mut image = reader.image(1).unwrap();
        assert_eq!(
            image.read(..).unwrap().samples(),
            ramp(Scaling::IDENTITY).samples()
        );
        assert_eq!(
            image.read(&[1..3, 1..3]).unwrap().samples(),
            &Data::I16(vec![11, 12, 21, 22])
        );
        let (Data::F32(written), Data::F32(read)) = (
            floats.samples(),
            reader.image(2).unwrap().read(..).unwrap().into_samples(),
        ) else {
            unreachable!("both hold f32 samples");
        };
        let bits = |samples: &[f32]| {
            samples
                .iter()
                .map(|sample| sample.to_bits())
                .collect::<Vec<_>>()
        };
        assert_eq!(bits(written), bits(&read));
    }

    /// A compressed table reads back by rows as the plain table does.
    #[test]
    fn a_compressed_table_reads_by_rows() {
        let mut table = Builder::new();
        table
            .push(WriteColumn::scalar(
                "ID",
                ColumnData::I32((0..250).collect()),
            ))
            .unwrap();
        let config = CompressionConfig::new(Compression::Rice).with_tiles([100]);
        let bytes = file(|writer| {
            writer
                .write_compressed_table(&table, &config, None)
                .unwrap();
        });

        let mut reader = FitsReader::from_bytes(&bytes).unwrap();
        assert_eq!(reader.read_table(1).unwrap().schema().row_count(), 250);
        // Rows 98 to 101 cross the first tile's end.
        let rows = reader.read_table_rows(1, 98..102).unwrap();
        assert_eq!(
            rows.column("ID").unwrap().raw().unwrap(),
            ColumnData::I32(vec![98, 99, 100, 101])
        );
    }
}
